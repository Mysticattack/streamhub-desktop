//! StreamHub Desktop - native Steuerung fuer den StreamHub-Server.
//!
//! Zeigt Live-Status und Laufzeit, verwaltet Ziele, Kodierung, Aufnahme und
//! zeigt die Sitzungs-Historie. Spricht die HTTP-API des Servers.

use eframe::egui;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------- Datenmodell

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
struct Transcode {
    enabled: bool,
    height: u32,
    fps: u32,
    bitrate_kbps: u32,
    preset: String,
    audio_kbps: u32,
}

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
struct RecordCfg {
    enabled: bool,
    chunk_minutes: u32,
}

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
struct RecordState {
    enabled: bool,
    running: bool,
    chunk_minutes: u32,
}

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
struct Ingest {
    server: String,
    key: String,
}

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
struct Overlay {
    url: String,
    token: String,
}

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
struct Youtube {
    api_key: String,
    video_id: String,
}

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
struct DestCfg {
    name: String,
    url: String,
    enabled: bool,
}

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
struct Config {
    transcode: Transcode,
    record: RecordCfg,
    ingest: Ingest,
    overlay: Overlay,
    youtube: Youtube,
    destinations: Vec<DestCfg>,
}

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
struct DestStatus {
    name: String,
    enabled: bool,
    running: bool,
    up: u64,
    err: String,
    url: String,
}

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
struct Recording {
    name: String,
    mb: f64,
    at: String,
}

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
struct Status {
    live: bool,
    duration: u64,
    started_at: String,
    today: u64,
    followers: Option<u32>,
    goal: Option<u32>,
    scene: Option<String>,
    nowplaying: Option<String>,
    load: f64,
    disk_free_gb: f64,
    record: RecordState,
    recordings: Vec<Recording>,
    destinations: Vec<DestStatus>,
    transcode: Transcode,
}

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
struct Session {
    start: String,
    sec: u64,
    #[serde(default)]
    ids: Vec<String>,
}

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
struct Sessions {
    sessions: Vec<Session>,
}

#[derive(Deserialize, Serialize, Clone)]
struct Local {
    server: String,
    token: String,
}

impl Default for Local {
    fn default() -> Self {
        Local {
            server: "https://grafana.allo.bet/hub".to_string(),
            token: String::new(),
        }
    }
}

fn local_path() -> std::path::PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::path::PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
                .join(".config")
        });
    base.join("streamhub-desktop").join("local.json")
}

fn local_load() -> Local {
    std::fs::read_to_string(local_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn local_save(l: &Local) {
    if let Some(dir) = local_path().parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(s) = serde_json::to_string_pretty(l) {
        let _ = std::fs::write(local_path(), s);
    }
}

// ---------------------------------------------------------------- HTTP

fn api_get<T: for<'de> Deserialize<'de>>(base: &str, token: &str, path: &str) -> Result<T, String> {
    let url = format!("{}{}?token={}", base.trim_end_matches('/'), path, urlencode(token));
    let body = ureq::get(&url)
        .timeout(Duration::from_secs(8))
        .call()
        .map_err(|e| format!("{}", e))?
        .into_string()
        .map_err(|e| format!("{}", e))?;
    serde_json::from_str(&body).map_err(|e| format!("JSON: {}", e))
}

fn api_post(base: &str, token: &str, path: &str, body: &serde_json::Value) -> Result<(), String> {
    let url = format!("{}{}?token={}", base.trim_end_matches('/'), path, urlencode(token));
    ureq::post(&url)
        .timeout(Duration::from_secs(10))
        .set("Content-Type", "application/json")
        .send_string(&body.to_string())
        .map_err(|e| format!("{}", e))?;
    Ok(())
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{:02X}", b)
            }
        })
        .collect()
}

// ---------------------------------------------------------------- Hintergrund-Abfrage

#[derive(Default)]
struct Shared {
    status: Option<Status>,
    config: Option<Config>,
    sessions: Vec<Session>,
    error: String,
    updated: f64,
}

type SharedRef = Arc<Mutex<Shared>>;

fn spawn_poller(ctx: egui::Context, local: Arc<Mutex<Local>>, shared: SharedRef) {
    std::thread::spawn(move || loop {
        let (base, token) = {
            let l = local.lock().unwrap();
            (l.server.clone(), l.token.clone())
        };
        let mut next = Shared::default();
        let mut err = String::new();
        match api_get::<Status>(&base, &token, "/api/status") {
            Ok(s) => next.status = Some(s),
            Err(e) => err = format!("Status: {}", e),
        }
        match api_get::<serde_json::Value>(&base, &token, "/api/config") {
            Ok(v) => {
                if let Some(c) = v.get("config") {
                    if let Ok(cfg) = serde_json::from_value::<Config>(c.clone()) {
                        next.config = Some(cfg);
                    }
                }
            }
            Err(e) => {
                if err.is_empty() {
                    err = format!("Config: {}", e);
                }
            }
        }
        match api_get::<Sessions>(&base, &token, "/api/sessions") {
            Ok(s) => next.sessions = s.sessions,
            Err(_) => {}
        }
        next.error = err;
        next.updated = Instant::now().elapsed().as_secs_f64();
        {
            let mut g = shared.lock().unwrap();
            *g = next;
        }
        ctx.request_repaint();
        std::thread::sleep(Duration::from_secs(2));
    });
}

// ---------------------------------------------------------------- Tabs

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Status,
    Ziele,
    Kodierung,
    Aufnahme,
    Sitzungen,
    Einstellungen,
}

// ---------------------------------------------------------------- App

struct App {
    local: Arc<Mutex<Local>>,
    shared: SharedRef,
    tab: Tab,
    tick: Instant,
    new_name: String,
    new_url: String,
    draft: Option<Config>,
    msg: String,
    msg_time: Instant,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let local = Arc::new(Mutex::new(local_load()));
        let shared: SharedRef = Arc::new(Mutex::new(Shared::default()));
        spawn_poller(cc.egui_ctx.clone(), local.clone(), shared.clone());
        App {
            local,
            shared,
            tab: Tab::Status,
            tick: Instant::now(),
            new_name: String::new(),
            new_url: String::new(),
            draft: None,
            msg: String::new(),
            msg_time: Instant::now(),
        }
    }

    fn note(&mut self, s: impl Into<String>) {
        self.msg = s.into();
        self.msg_time = Instant::now();
    }

    fn post(&mut self, path: &str, body: serde_json::Value) {
        let (base, token) = {
            let l = self.local.lock().unwrap();
            (l.server.clone(), l.token.clone())
        };
        match api_post(&base, &token, path, &body) {
            Ok(_) => self.note("gespeichert"),
            Err(e) => self.note(format!("Fehler: {}", e)),
        }
    }

    fn sync_draft(&mut self, cfg: &Config) {
        if self.draft.is_none() {
            self.draft = Some(cfg.clone());
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let shared = self.shared.lock().unwrap();
        let status = shared.status.clone();
        let config = shared.config.clone();
        let sessions = shared.sessions.clone();
        let error = shared.error.clone();
        drop(shared);

        if let Some(c) = &config {
            self.sync_draft(c);
        }

        egui::TopBottomPanel::top("kopf").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.heading("StreamHub");
                ui.separator();
                match &status {
                    Some(s) if s.live => {
                        ui.colored_label(egui::Color32::from_rgb(90, 220, 90), "● LIVE");
                        ui.label(format!("seit {} · {}", s.started_at, hms(s.duration as f64)));
                    }
                    Some(_) => {
                        ui.colored_label(egui::Color32::GRAY, "● offline");
                    }
                    None => {
                        ui.colored_label(egui::Color32::from_rgb(220, 120, 120), "keine Verbindung");
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !error.is_empty() {
                        ui.colored_label(egui::Color32::from_rgb(230, 150, 150), &error);
                    }
                    if !self.msg.is_empty() && self.msg_time.elapsed().as_secs() < 3 {
                        ui.colored_label(egui::Color32::from_rgb(150, 220, 150), &self.msg);
                    }
                });
            });
            ui.add_space(4.0);
        });

        egui::SidePanel::left("tabs").resizable(false).default_width(170.0).show(ctx, |ui| {
            ui.add_space(8.0);
            for (t, name) in [
                (Tab::Status, "Status"),
                (Tab::Ziele, "Ziele"),
                (Tab::Kodierung, "Kodierung"),
                (Tab::Aufnahme, "Aufnahme"),
                (Tab::Sitzungen, "Sitzungen"),
                (Tab::Einstellungen, "Einstellungen"),
            ] {
                if ui.selectable_label(self.tab == t, name).clicked() {
                    self.tab = t;
                }
            }
            ui.separator();
            ui.small("Taktet alle 2 Sekunden");
        });

        egui::CentralPanel::default().show(ctx, |ui| match self.tab {
            Tab::Status => self.ui_status(ui, &status),
            Tab::Ziele => self.ui_ziele(ui, &status, &config),
            Tab::Kodierung => self.ui_kodierung(ui, &config),
            Tab::Aufnahme => self.ui_aufnahme(ui, &status, &config),
            Tab::Sitzungen => self.ui_sitzungen(ui, &sessions),
            Tab::Einstellungen => self.ui_einstellungen(ui, &config),
        });

        if self.tick.elapsed() >= Duration::from_millis(500) {
            self.tick = Instant::now();
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }
}

impl App {
    fn ui_status(&mut self, ui: &mut egui::Ui, st: &Option<Status>) {
        ui.heading("Überblick");
        ui.add_space(8.0);
        let Some(s) = st else {
            ui.label("Noch keine Daten – Server-Adresse und Token unter Einstellungen prüfen.");
            return;
        };
        egui::Grid::new("grid_status").num_columns(2).spacing([24.0, 10.0]).show(ui, |ui| {
            ui.label("Laufzeit");
            ui.heading(hms(s.duration as f64));
            ui.end_row();
            ui.label("Gestartet");
            ui.label(if s.started_at.is_empty() { "–".into() } else { s.started_at.clone() });
            ui.end_row();
            ui.label("Heute gesamt");
            ui.label(hms(s.today as f64));
            ui.end_row();
            ui.label("Follower");
            let fol = match (s.followers, s.goal) {
                (Some(f), Some(g)) => format!("{} / {}", f, g),
                (Some(f), None) => format!("{}", f),
                _ => "–".to_string(),
            };
            ui.label(fol);
            ui.end_row();
            ui.label("Szene");
            ui.label(s.scene.clone().unwrap_or_else(|| "–".into()));
            ui.end_row();
            ui.label("Musik");
            ui.label(match &s.nowplaying {
                Some(t) if !t.is_empty() => t.clone(),
                _ => "–".to_string(),
            });
            ui.end_row();
            ui.label("Server-Last");
            ui.label(format!("{:.2}", s.load));
            ui.end_row();
            ui.label("Speicher frei");
            ui.label(format!("{:.1} GB", s.disk_free_gb));
            ui.end_row();
            ui.label("Aufnahme");
            if s.record.running {
                ui.colored_label(egui::Color32::from_rgb(200, 120, 120), "läuft");
            } else {
                ui.label("aus");
            }
            ui.end_row();
            ui.label("Aktive Ziele");
            let akt: Vec<String> = s.destinations.iter().filter(|d| d.running).map(|d| d.name.clone()).collect();
            ui.label(if akt.is_empty() { "keine".into() } else { akt.join(", ") });
            ui.end_row();
        });
    }

    fn ui_ziele(&mut self, ui: &mut egui::Ui, st: &Option<Status>, cfg: &Option<Config>) {
        ui.heading("Ziele");
        ui.add_space(6.0);
        if let Some(s) = st {
            for d in &s.destinations {
                ui.horizontal(|ui| {
                    ui.monospace(format!("{:<12}", d.name));
                    if d.running {
                        ui.colored_label(egui::Color32::from_rgb(90, 220, 90), format!("läuft {}s", d.up));
                    } else if d.enabled {
                        ui.colored_label(egui::Color32::from_rgb(220, 140, 140),
                                          if d.err.is_empty() { "aus".to_string() } else { format!("aus: {}", d.err.chars().take(50).collect::<String>()) });
                    } else {
                        ui.label("ausgeschaltet");
                    }
                    ui.label(&d.url);
                    if ui.small_button(if d.enabled { "aus" } else { "an" }).clicked() {
                        self.post("/api/toggle", serde_json::json!({"name": d.name}));
                    }
                    if ui.small_button("entfernen").clicked() {
                        self.post("/api/del", serde_json::json!({"name": d.name}));
                    }
                });
            }
        }
        ui.separator();
        ui.label("Neues Ziel");
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.new_name).hint_text("Name").desired_width(120.0));
            ui.add(egui::TextEdit::singleline(&mut self.new_url).hint_text("rtmp://live.twitch.tv/app/DEIN_KEY").desired_width(420.0));
            if ui.button("Hinzufügen").clicked() && !self.new_name.is_empty() && !self.new_url.is_empty() {
                let body = serde_json::json!({"name": self.new_name.clone(), "url": self.new_url.clone(), "enabled": true});
                self.post("/api/dest", body);
                self.new_name.clear();
                self.new_url.clear();
            }
        });
        if let Some(c) = cfg {
            if let Some(ing) = Some(&c.ingest) {
                ui.add_space(12.0);
                ui.label("Eingang für OBS");
                ui.monospace(format!("Server: {}", ing.server));
                ui.monospace(format!("Schlüssel: {}", ing.key));
            }
        }
    }

    fn ui_kodierung(&mut self, ui: &mut egui::Ui, cfg: &Option<Config>) {
        ui.heading("Kodierung auf dem Server");
        ui.add_space(8.0);
        let Some(c) = cfg else { return };
        if self.draft.is_none() {
            self.draft = Some(c.clone());
        }
        let d = self.draft.as_mut().unwrap();
        ui.checkbox(&mut d.transcode.enabled, "Auf dem Server kodieren");
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label("Auflösung");
            for h in [1080u32, 900, 720] {
                ui.selectable_value(&mut d.transcode.height, h, format!("{}p", h));
            }
        });
        ui.horizontal(|ui| {
            ui.label("Bildrate");
            for f in [30u32, 60] {
                ui.selectable_value(&mut d.transcode.fps, f, format!("{}", f));
            }
        });
        ui.horizontal(|ui| {
            ui.label("Bitrate");
            ui.add(egui::Slider::new(&mut d.transcode.bitrate_kbps, 1000..=12000).suffix(" kbps"));
        });
        ui.horizontal(|ui| {
            ui.label("Preset");
            for p in ["ultrafast", "superfast", "veryfast", "faster"] {
                ui.selectable_value(&mut d.transcode.preset, p.to_string(), p);
            }
        });
        ui.add_space(10.0);
        if ui.button("Speichern").clicked() {
            let body = serde_json::json!({"transcode": d.transcode});
            self.post("/api/config", body);
        }
    }

    fn ui_aufnahme(&mut self, ui: &mut egui::Ui, st: &Option<Status>, cfg: &Option<Config>) {
        ui.heading("Aufnahme auf dem Server");
        ui.add_space(8.0);
        let Some(c) = cfg else { return };
        if self.draft.is_none() {
            self.draft = Some(c.clone());
        }
        let d = self.draft.as_mut().unwrap();
        let mut changed = false;
        changed |= ui.checkbox(&mut d.record.enabled, "Beim Streamen aufnehmen").changed();
        ui.horizontal(|ui| {
            ui.label("Neue Datei alle");
            changed |= ui.add(egui::DragValue::new(&mut d.record.chunk_minutes).range(10..=240)).changed();
            ui.label("Minuten");
        });
        if changed {
            let body = serde_json::json!({"record": {"enabled": d.record.enabled, "chunk_minutes": d.record.chunk_minutes}});
            self.post("/api/config", body);
        }
        ui.add_space(10.0);
        ui.separator();
        if let Some(s) = st {
            if s.record.running {
                ui.colored_label(egui::Color32::from_rgb(220, 140, 140), "Aufnahme läuft gerade");
            }
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                egui::Grid::new("recs").num_columns(3).spacing([18.0, 6.0]).show(ui, |ui| {
                    ui.strong("Datei");
                    ui.strong("Größe");
                    ui.strong("Zeit");
                    ui.end_row();
                    for r in &s.recordings {
                        ui.monospace(&r.name);
                        ui.label(format!("{:.1} MB", r.mb));
                        ui.label(&r.at);
                        ui.end_row();
                    }
                });
            });
        }
    }

    fn ui_sitzungen(&mut self, ui: &mut egui::Ui, sessions: &[Session]) {
        ui.heading("Sitzungen");
        ui.add_space(6.0);
        if sessions.is_empty() {
            ui.label("Noch keine Sitzung aufgezeichnet.");
            return;
        }
        let mut total = 0u64;
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("sess").num_columns(3).spacing([18.0, 6.0]).show(ui, |ui| {
                ui.strong("Beginn");
                ui.strong("Dauer");
                ui.strong("Ziele");
                ui.end_row();
                for s in sessions.iter().rev() {
                    total += s.sec;
                    ui.monospace(&s.start);
                    ui.label(hms(s.sec as f64));
                    ui.label(if s.ids.is_empty() { "–".into() } else { s.ids.join(", ") });
                    ui.end_row();
                }
            });
        });
        ui.separator();
        ui.label(format!("Summe: {}", hms(total as f64)));
    }

    fn ui_einstellungen(&mut self, ui: &mut egui::Ui, cfg: &Option<Config>) {
        ui.heading("Einstellungen");
        ui.add_space(8.0);
        let mut l = self.local.lock().unwrap().clone();
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label("Server");
            changed |= ui.add(egui::TextEdit::singleline(&mut l.server).desired_width(380.0)).changed();
        });
        ui.horizontal(|ui| {
            ui.label("Token");
            changed |= ui.add(egui::TextEdit::singleline(&mut l.token).password(true).desired_width(380.0)).changed();
        });
        if changed {
            *self.local.lock().unwrap() = l.clone();
            local_save(&l);
        }
        ui.add_space(10.0);
        ui.separator();
        if let Some(c) = cfg {
            ui.label("Chat-Weiterleitung");
            let mut y = c.youtube.clone();
            let mut ov = c.overlay.clone();
            let mut ch = false;
            ui.horizontal(|ui| {
                ui.label("YouTube API-Key");
                ch |= ui.add(egui::TextEdit::singleline(&mut y.api_key).desired_width(380.0)).changed();
            });
            ui.horizontal(|ui| {
                ui.label("YouTube Video-ID");
                ch |= ui.add(egui::TextEdit::singleline(&mut y.video_id).desired_width(380.0)).changed();
            });
            ui.horizontal(|ui| {
                ui.label("Overlay-Token");
                ch |= ui.add(egui::TextEdit::singleline(&mut ov.token).password(true).desired_width(380.0)).changed();
            });
            if ch {
                let body = serde_json::json!({"youtube": y, "overlay": ov});
                self.post("/api/config", body);
            }
        }
    }
}

fn hms(sec: f64) -> String {
    let s = sec.max(0.0) as u64;
    format!("{:02}:{:02}:{:02}", s / 3600, s % 3600 / 60, s % 60)
}

fn main() -> eframe::Result<()> {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([980.0, 660.0])
            .with_min_inner_size([760.0, 480.0])
            .with_title("StreamHub"),
        ..Default::default()
    };
    eframe::run_native("StreamHub", opts, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
