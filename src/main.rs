//! StreamHub Desktop - native Steuerung fuer den StreamHub-Server.
//!
//! Zeigt Live-Status und Laufzeit, verwaltet Ziele, Kodierung, Aufnahme und
//! zeigt die Sitzungs-Historie. Spricht die HTTP-API des Servers.

mod obs;

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
    #[serde(default)]
    client_id: String,
    #[serde(default)]
    client_secret: String,
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
    #[serde(default)]
    ingest_v: Ingest,
    record: RecordCfg,
    ingest: Ingest,
    overlay: Overlay,
    youtube: Youtube,
    destinations: Vec<DestCfg>,
}

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
struct TwInfo {
    #[serde(default)]
    title: String,
    #[serde(default)]
    game: String,
    #[serde(default)]
    language: String,
    #[serde(default)]
    ok: bool,
}

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
struct YtInfo {
    #[serde(default)]
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    privacy: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    ok: bool,
}

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
struct StreamInfo {
    #[serde(default)]
    twitch: TwInfo,
    #[serde(default)]
    youtube: YtInfo,
    #[serde(default)]
    ok: bool,
}

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
struct InfoResp {
    #[serde(default)]
    info: StreamInfo,
}

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
struct ActItem {
    t: String,
    kind: String,
    text: String,
}

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
struct Activity {
    #[serde(default)]
    items: Vec<ActItem>,
}

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
struct DestStatus {
    name: String,
    enabled: bool,
    running: bool,
    up: u64,
    err: String,
    url: String,
    #[serde(default)]
    metrics: std::collections::HashMap<String, String>,
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
    #[serde(default)]
    obs_password: String,
}

impl Default for Local {
    fn default() -> Self {
        Local {
            server: "https://grafana.allo.bet/hub".to_string(),
            token: String::new(),
            obs_password: String::new(),
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
    obs_streaming: Option<bool>,
    status: Option<Status>,
    config: Option<Config>,
    sessions: Vec<Session>,
    activity: Vec<ActItem>,
    info: Option<StreamInfo>,
    error: String,
    updated: f64,
}

type SharedRef = Arc<Mutex<Shared>>;

fn spawn_poller(ctx: egui::Context, local: Arc<Mutex<Local>>, shared: SharedRef) {
    std::thread::spawn(move || {
        let mut tick: u32 = 0;
        loop {
        let (base, token, obspw) = {
            let l = local.lock().unwrap();
            (l.server.clone(), l.token.clone(), l.obs_password.clone())
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
        if let Ok(a) = api_get::<Activity>(&base, &token, "/api/activity") {
            next.activity = a.items;
        }
        if let Ok(i) = api_get::<InfoResp>(&base, &token, "/api/streaminfo") {
            next.info = Some(i.info);
        }
        next.error = err;
        next.updated = Instant::now().elapsed().as_secs_f64();
        let prev_obs = shared.lock().unwrap().obs_streaming;
        if !obspw.is_empty() && tick % 3 == 0 {
            next.obs_streaming = obs::obs_is_streaming(&obspw);
        } else {
            next.obs_streaming = prev_obs;
        }
        tick = tick.wrapping_add(1);
        {
            let mut g = shared.lock().unwrap();
            *g = next;
        }
        ctx.request_repaint();
        std::thread::sleep(Duration::from_secs(2));
        }
    });
}

// ---------------------------------------------------------------- Tabs

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Status,
    Infos,
    Chat,
    Szenen,
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
    scenes: Vec<String>,
    yt_id: String,
    yt_secret: String,
    yt_state: String,
    inf_tw_title: String,
    inf_tw_game: String,
    inf_yt_title: String,
    inf_yt_desc: String,
    inf_loaded: bool,
    big: bool,
    chat_platform: bool,   // false = Twitch, true = YouTube
    chat_text: String,
    current_scene: String,
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
            scenes: Vec::new(),
            yt_id: String::new(),
            yt_secret: String::new(),
            yt_state: String::new(),
            inf_tw_title: String::new(),
            inf_tw_game: String::new(),
            inf_yt_title: String::new(),
            inf_yt_desc: String::new(),
            inf_loaded: false,
            big: false,
            chat_platform: false,
            chat_text: String::new(),
            current_scene: String::new(),
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

    fn yt_connect(&mut self) {
        let (base, token) = {
            let l = self.local.lock().unwrap();
            (l.server.clone(), l.token.clone())
        };
        let url = format!("{}/api/yt/start?token={}", base.trim_end_matches('/'), urlencode(&token));
        match ureq::get(&url).timeout(Duration::from_secs(10)).call() {
            Ok(resp) => {
                let txt = resp.into_string().unwrap_or_default();
                let v: serde_json::Value = serde_json::from_str(&txt).unwrap_or_default();
                if v.get("ok").and_then(|o| o.as_bool()) == Some(true) {
                    if let Some(u) = v.get("url").and_then(|u| u.as_str()) {
                        let _ = std::process::Command::new("xdg-open").arg(u).spawn();
                        self.yt_state = "Browser geoeffnet - bitte zustimmen".into();
                    }
                } else {
                    self.yt_state = v.get("error").and_then(|e| e.as_str()).unwrap_or("Fehler").to_string();
                    self.note(format!("YouTube: {}", self.yt_state));
                }
            }
            Err(e) => self.note(format!("YouTube: {}", e)),
        }
    }

    fn start_stop_obs(&mut self, pw: &str, start: bool) {
        if pw.is_empty() {
            self.note("OBS-Passwort fehlt (Einstellungen)");
            return;
        }
        let req = if start { "StartStream" } else { "StopStream" };
        match obs::obs_request(pw, req) {
            Ok(_) => self.note(if start { "Stream gestartet" } else { "Stream gestoppt" }),
            Err(e) => self.note(format!("OBS: {}", e)),
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
        let activity = shared.activity.clone();
        let info = shared.info.clone();
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
                if ui.button(if self.big { "Übersicht schließen" } else { "Übersicht" }).clicked() {
                    self.big = !self.big;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(self.big));
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

        if self.big {
            egui::TopBottomPanel::bottom("steuer")
                .resizable(true)
                .default_height(300.0)
                .min_height(120.0)
                .show(ctx, |ui| {
                    ui.add_space(4.0);
                    ui.label("Einstellen");
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        self.ui_steuerung(ui, &status, &config);
                    });
                });
            egui::CentralPanel::default().show(ctx, |ui| {
                self.ui_dashboard(ui, &status, &activity, &config);
            });
            if self.tick.elapsed() >= Duration::from_millis(500) {
                self.tick = Instant::now();
                ctx.request_repaint_after(Duration::from_millis(500));
            }
            return;
        }

        egui::SidePanel::left("tabs").resizable(false).default_width(170.0).show(ctx, |ui| {
            ui.add_space(8.0);
            for (t, name) in [
                (Tab::Status, "Status"),
                (Tab::Infos, "Infos"),
                (Tab::Chat, "Chat"),
                (Tab::Szenen, "Szenen"),
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
            Tab::Status => self.ui_status(ui, &status, &activity),
            Tab::Infos => self.ui_infos(ui, &info),
            Tab::Chat => self.ui_chat(ui),
            Tab::Szenen => self.ui_szenen(ui),
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
    fn ui_status(&mut self, ui: &mut egui::Ui, st: &Option<Status>, activity: &[ActItem]) {
        ui.heading("Überblick");
        ui.add_space(6.0);
        let obs_state = self.shared.lock().unwrap().obs_streaming;
        ui.horizontal(|ui| {
            let lbl = match obs_state {
                Some(true) => "OBS sendet gerade",
                Some(false) => "OBS bereit",
                None => "OBS nicht erreichbar (Passwort in den Einstellungen prüfen)",
            };
            ui.label(lbl);
            ui.separator();
            let (base_local, pw) = {
                let l = self.local.lock().unwrap();
                (l.server.clone(), l.obs_password.clone())
            };
            let _ = base_local;
            if ui.button("Stream starten").clicked() {
                self.start_stop_obs(&pw, true);
            }
            if ui.button("Stream stoppen").clicked() {
                self.start_stop_obs(&pw, false);
            }
        });
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

        if !s.destinations.is_empty() {
            ui.add_space(10.0);
            ui.separator();
            ui.label("Kennzahlen der Ziele");
            egui::Grid::new("kpi").num_columns(5).spacing([18.0, 4.0]).show(ui, |ui| {
                ui.strong("Ziel");
                ui.strong("Bitrate");
                ui.strong("fps");
                ui.strong("verworfen");
                ui.strong("Tempo");
                ui.end_row();
                for d in &s.destinations {
                    ui.label(&d.name);
                    ui.label(d.metrics.get("bitrate").cloned().unwrap_or_else(|| "-".into()));
                    ui.label(d.metrics.get("fps").cloned().unwrap_or_else(|| "-".into()));
                    ui.label(d.metrics.get("drop_frames").cloned().unwrap_or_else(|| "-".into()));
                    ui.label(d.metrics.get("speed").cloned().unwrap_or_else(|| "-".into()));
                    ui.end_row();
                }
            });
        }

        ui.add_space(10.0);
        ui.separator();
        ui.label("Aktivitaet");
        egui::ScrollArea::vertical().max_height(150.0).show(ui, |ui| {
            for a in activity.iter().rev().take(20) {
                ui.horizontal(|ui| {
                    ui.monospace(&a.t);
                    ui.weak(&a.kind);
                    ui.label(&a.text);
                });
            }
        });
    }



    fn ui_infos(&mut self, ui: &mut egui::Ui, info: &Option<StreamInfo>) {
        ui.heading("Stream-Infos");
        ui.add_space(6.0);
        if let Some(i) = info {
            if !self.inf_loaded {
                self.inf_tw_title = i.twitch.title.clone();
                self.inf_tw_game = i.twitch.game.clone();
                self.inf_yt_title = i.youtube.title.clone();
                self.inf_yt_desc = i.youtube.description.clone();
                self.inf_loaded = true;
            }
        }
        ui.label("Twitch");
        ui.horizontal(|ui| {
            ui.label("Titel");
            ui.add(egui::TextEdit::singleline(&mut self.inf_tw_title).desired_width(520.0));
        });
        ui.horizontal(|ui| {
            ui.label("Kategorie");
            ui.add(egui::TextEdit::singleline(&mut self.inf_tw_game).desired_width(300.0));
        });
        ui.add_space(10.0);
        ui.label("YouTube");
        ui.horizontal(|ui| {
            ui.label("Titel");
            ui.add(egui::TextEdit::singleline(&mut self.inf_yt_title).desired_width(520.0));
        });
        ui.horizontal(|ui| {
            ui.label("Beschreibung");
            ui.add(egui::TextEdit::multiline(&mut self.inf_yt_desc).desired_width(520.0).desired_rows(3));
        });
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.button("Speichern").clicked() {
                let body = serde_json::json!({
                    "twitch_title": self.inf_tw_title,
                    "twitch_game": self.inf_tw_game,
                    "youtube_title": self.inf_yt_title,
                    "youtube_description": self.inf_yt_desc
                });
                self.post("/api/streaminfo", body);
                self.inf_loaded = false;
            }
            if ui.button("Neu laden").clicked() {
                self.inf_loaded = false;
            }
        });
        ui.weak("Twitch-Titel/Kategorie brauchen die Freigabe channel:manage:broadcast.");
        if let Some(i) = info {
            ui.add_space(8.0);
            ui.label(format!("YouTube-Status: {} / {}", i.youtube.state, i.youtube.privacy));
        }
    }


    fn ui_dashboard(&mut self, ui: &mut egui::Ui, st: &Option<Status>, activity: &[ActItem],
                    cfg_now: &Option<Config>) {
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            ui.heading("Übersicht");
            if ui.button("Vollbild").clicked() {
                let fs = ui.ctx().input(|i| i.viewport().fullscreen.unwrap_or(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fs));
            }
            ui.separator();
            match st {
                Some(s) if s.live => {
                    ui.colored_label(egui::Color32::from_rgb(90, 220, 90), "● LIVE");
                    ui.label(format!("{}   (Start {})", hms(s.duration as f64), s.started_at));
                }
                Some(_) => {
                    ui.colored_label(egui::Color32::GRAY, "● offline");
                }
                None => {
                    ui.colored_label(egui::Color32::from_rgb(220, 120, 120), "keine Verbindung");
                }
            }
        });
        ui.separator();

        ui.columns(3, |cols| {
            // Spalte 1: Kennzahlen
            let ui = &mut cols[0];
            ui.label("Laufzeit");
            ui.heading(hms(st.as_ref().map(|s| s.duration as f64).unwrap_or(0.0)));
            ui.label("Heute gesamt");
            ui.heading(hms(st.as_ref().map(|s| s.today as f64).unwrap_or(0.0)));
            ui.add_space(6.0);
            if let Some(s) = st {
                ui.label(format!(
                    "Follower: {}",
                    match (s.followers, s.goal) {
                        (Some(f), Some(g)) => format!("{} / {}", f, g),
                        (Some(f), None) => format!("{}", f),
                        _ => "-".to_string(),
                    }
                ));
                ui.label(format!("Szene: {}", s.scene.clone().unwrap_or_else(|| "-".into())));
                ui.label(match &s.nowplaying {
                    Some(t) if !t.is_empty() => format!("Musik: {}", t),
                    _ => "Musik: -".to_string(),
                });
                ui.label(format!("Server: Last {:.2}, {:.0} GB frei", s.load, s.disk_free_gb));
                ui.add_space(8.0);
                ui.label("Ziele");
                for d in &s.destinations {
                    ui.horizontal(|ui| {
                        ui.monospace(format!("{:<10}", d.name));
                        if d.running {
                            ui.colored_label(egui::Color32::from_rgb(90, 220, 90), "läuft");
                        } else if d.enabled {
                            ui.colored_label(egui::Color32::from_rgb(200, 200, 120), "wartet");
                        } else {
                            ui.weak("aus");
                        }
                        ui.label(d.metrics.get("bitrate").cloned().unwrap_or_else(|| "-".into()));
                        ui.weak(format!("drop {}", d.metrics.get("drop_frames").cloned().unwrap_or_else(|| "-".into())));
                    });
                }
            }

            // Spalte 2: Chat senden
            let ui = &mut cols[1];
            ui.label("Chat schreiben");
            ui.horizontal(|ui| {
                if ui.selectable_label(!self.chat_platform, "Twitch").clicked() {
                    self.chat_platform = false;
                }
                if ui.selectable_label(self.chat_platform, "YouTube").clicked() {
                    self.chat_platform = true;
                }
            });
            ui.add_space(6.0);
            ui.add(
                egui::TextEdit::multiline(&mut self.chat_text)
                    .hint_text("Nachricht")
                    .desired_rows(3)
                    .desired_width(f32::INFINITY),
            );
            let ziel = if self.chat_platform { "YouTube" } else { "Twitch" };
            if ui.button(format!("An {} senden", ziel)).clicked() && !self.chat_text.trim().is_empty() {
                let plat = if self.chat_platform { "youtube" } else { "twitch" };
                let body = serde_json::json!({"text": self.chat_text, "platform": plat});
                let (base, token) = {
                    let l = self.local.lock().unwrap();
                    (l.server.clone(), l.token.clone())
                };
                match api_post(&base, &token, "/api/chat", &body) {
                    Ok(_) => {
                        self.note(format!("an {} gesendet", plat));
                        self.chat_text.clear();
                    }
                    Err(e) => self.note(format!("Chat: {}", e)),
                }
            }
            if !self.msg.is_empty() && self.msg_time.elapsed().as_secs() < 4 {
                ui.colored_label(egui::Color32::from_rgb(150, 220, 150), &self.msg);
            }

            // Spalte 3: Aktivitaet
            let ui = &mut cols[2];
            ui.label("Aktivität");
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for a in activity.iter().rev().take(40) {
                    ui.horizontal(|ui| {
                        ui.monospace(&a.t);
                        ui.weak(&a.kind);
                        ui.label(&a.text);
                    });
                }
            });
        });

    }


    fn ui_steuerung(&mut self, ui: &mut egui::Ui, st: &Option<Status>, cfg: &Option<Config>) {
        ui.columns(3, |cols| {
            // --- Spalte 1: Stream + Szenen
            let ui = &mut cols[0];
            ui.label("Stream");
            let pw = self.local.lock().unwrap().obs_password.clone();
            let obs_state = self.shared.lock().unwrap().obs_streaming;
            ui.horizontal(|ui| {
                if ui.button("Start").clicked() {
                    self.start_stop_obs(&pw, true);
                }
                if ui.button("Stopp").clicked() {
                    self.start_stop_obs(&pw, false);
                }
                ui.weak(match obs_state {
                    Some(true) => "sendet",
                    Some(false) => "bereit",
                    None => "OBS?",
                });
            });
            ui.add_space(8.0);
            ui.label("Szene");
            if self.scenes.is_empty() && !pw.is_empty() {
                if let Ok(r) = obs::obs_request(&pw, "GetSceneList") {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&r) {
                        if let Some(list) = v["d"]["responseData"]["scenes"].as_array() {
                            self.scenes = list
                                .iter()
                                .filter_map(|x| x.get("sceneName").and_then(|n| n.as_str()).map(String::from))
                                .collect();
                        }
                        if let Some(cur) = v["d"]["responseData"]["currentProgramSceneName"].as_str() {
                            self.current_scene = cur.to_string();
                        }
                    }
                }
            }
            for name in self.scenes.clone() {
                let aktiv = name == self.current_scene;
                if ui.selectable_label(aktiv, &name).clicked() && !aktiv {
                    let body = serde_json::json!({"sceneName": name});
                    if obs::obs_request_json(&pw, "SetCurrentProgramScene", body).is_ok() {
                        self.current_scene = name.clone();
                        self.note(format!("Szene: {}", name));
                    }
                }
            }

            // --- Spalte 2: Ziele + Aufnahme
            let ui = &mut cols[1];
            ui.label("Ziele");
            if let Some(s) = st {
                for d in &s.destinations {
                    ui.horizontal(|ui| {
                        ui.monospace(format!("{:<10}", d.name));
                        if d.running {
                            ui.colored_label(egui::Color32::from_rgb(90, 220, 90), "läuft");
                        } else if d.enabled {
                            ui.colored_label(egui::Color32::from_rgb(200, 200, 120), "wartet");
                        } else {
                            ui.weak("aus");
                        }
                        if ui.small_button(if d.enabled { "aus" } else { "an" }).clicked() {
                            self.post("/api/toggle", serde_json::json!({"name": d.name}));
                        }
                    });
                }
            }
            ui.add_space(8.0);
            if let Some(c) = cfg {
                if self.draft.is_none() {
                    self.draft = Some(c.clone());
                }
                let mut rec_body: Option<serde_json::Value> = None;
                let mut enc_body: Option<serde_json::Value> = None;
                {
                    let d = self.draft.as_mut().unwrap();
                    if ui.checkbox(&mut d.record.enabled, "Aufnahme auf dem Server").changed() {
                        rec_body = Some(serde_json::json!({"record": {
                            "enabled": d.record.enabled, "chunk_minutes": d.record.chunk_minutes}}));
                    }
                ui.add_space(6.0);
                ui.label("Kodierung");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut d.transcode.height, 1080, "1080p");
                    ui.selectable_value(&mut d.transcode.height, 900, "900p");
                    ui.selectable_value(&mut d.transcode.height, 720, "720p");
                });
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut d.transcode.fps, 60, "60 fps");
                    ui.selectable_value(&mut d.transcode.fps, 30, "30 fps");
                });
                ui.add(egui::Slider::new(&mut d.transcode.bitrate_kbps, 2000..=9000).suffix(" kbps"));
                    if ui.button("Kodierung speichern").clicked() {
                        enc_body = Some(serde_json::json!({"transcode": d.transcode}));
                    }
                }
                if let Some(b) = rec_body {
                    self.post("/api/config", b);
                }
                if let Some(b) = enc_body {
                    self.post("/api/config", b);
                }
            }

            // --- Spalte 3: Infos
            let ui = &mut cols[2];
            ui.label("Stream-Infos");
            ui.horizontal(|ui| {
                ui.label("Twitch-Titel");
                ui.add(egui::TextEdit::singleline(&mut self.inf_tw_title).desired_width(f32::INFINITY));
            });
            ui.horizontal(|ui| {
                ui.label("Kategorie");
                ui.add(egui::TextEdit::singleline(&mut self.inf_tw_game).desired_width(f32::INFINITY));
            });
            ui.horizontal(|ui| {
                ui.label("YouTube-Titel");
                ui.add(egui::TextEdit::singleline(&mut self.inf_yt_title).desired_width(f32::INFINITY));
            });
            if ui.button("Infos speichern").clicked() {
                let body = serde_json::json!({
                    "twitch_title": self.inf_tw_title,
                    "twitch_game": self.inf_tw_game,
                    "youtube_title": self.inf_yt_title
                });
                self.post("/api/streaminfo", body);
            }
        });
    }

    fn ui_chat(&mut self, ui: &mut egui::Ui) {
        ui.heading("Chat");
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.selectable_label(!self.chat_platform, "Twitch").clicked() {
                self.chat_platform = false;
            }
            if ui.selectable_label(self.chat_platform, "YouTube").clicked() {
                self.chat_platform = true;
            }
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let hint = if self.chat_platform { "Nachricht an YouTube-Chat" } else { "Nachricht an Twitch-Chat" };
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.chat_text)
                    .hint_text(hint)
                    .desired_width(460.0),
            );
            let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (ui.button("Senden").clicked() || enter) && !self.chat_text.trim().is_empty() {
                let plat = if self.chat_platform { "youtube" } else { "twitch" };
                let body = serde_json::json!({"text": self.chat_text, "platform": plat});
                let (base, token) = {
                    let l = self.local.lock().unwrap();
                    (l.server.clone(), l.token.clone())
                };
                match api_post(&base, &token, "/api/chat", &body) {
                    Ok(_) => {
                        self.note(format!("an {} gesendet", plat));
                        self.chat_text.clear();
                    }
                    Err(e) => self.note(format!("Chat: {}", e)),
                }
            }
        });
        ui.add_space(10.0);
        ui.weak("Twitch-Senden braucht eine Freigabe mit chat:edit - sag Bescheid, dann hole ich sie.");
    }

    fn ui_szenen(&mut self, ui: &mut egui::Ui) {
        ui.heading("Szenen");
        ui.add_space(6.0);
        let pw = self.local.lock().unwrap().obs_password.clone();
        if self.scenes.is_empty() {
            if pw.is_empty() {
                ui.label("OBS-Passwort fehlt (Einstellungen).");
                return;
            }
            if let Ok(r) = obs::obs_request(&pw, "GetSceneList") {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&r) {
                    if let Some(list) = v["d"]["responseData"]["scenes"].as_array() {
                        self.scenes = list
                            .iter()
                            .filter_map(|s| s.get("sceneName").and_then(|n| n.as_str()).map(String::from))
                            .collect();
                    }
                    if let Some(cur) = v["d"]["responseData"]["currentProgramSceneName"].as_str() {
                        self.current_scene = cur.to_string();
                    }
                }
            }
        }
        if self.scenes.is_empty() {
            ui.label("Keine Szenen gefunden – läuft OBS?");
            return;
        }
        for name in self.scenes.clone() {
            let ist_aktiv = name == self.current_scene;
            let text = if ist_aktiv { format!("{}  (läuft)", name) } else { name.clone() };
            if ui.selectable_label(ist_aktiv, text).clicked() && !ist_aktiv {
                let body = serde_json::json!({"sceneName": name});
                match obs::obs_request_json(&pw, "SetCurrentProgramScene", body) {
                    Ok(_) => {
                        self.current_scene = name.clone();
                        self.note(format!("Szene: {}", name));
                    }
                    Err(e) => self.note(format!("OBS: {}", e)),
                }
            }
        }
        ui.add_space(8.0);
        if ui.button("Liste neu laden").clicked() {
            self.scenes.clear();
        }
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
                        if d.err.is_empty() {
                            ui.colored_label(egui::Color32::from_rgb(205, 200, 120),
                                             "wartet auf Stream");
                        } else {
                            ui.colored_label(egui::Color32::from_rgb(220, 140, 140),
                                             format!("aus: {}", d.err.chars().take(60).collect::<String>()));
                        }
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
            ui.label("Plattform:");
            if ui.button("Twitch").clicked() {
                self.new_name = "Twitch".into();
                self.new_url = "rtmp://live.twitch.tv/app/".into();
            }
            if ui.button("YouTube").clicked() {
                self.new_name = "YouTube".into();
                self.new_url = "rtmp://a.rtmp.youtube.com/live2/".into();
            }
            if ui.button("TikTok").clicked() {
                self.new_name = "TikTok".into();
                self.new_url = "rtmp://push.tiktokcdn.com/live/".into();
            }
            ui.small("nur noch den Schlüssel anhängen");
        });
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
        ui.add_space(12.0);
        ui.separator();
        ui.label("YouTube verbinden");
        if let Some(c) = cfg {
            if self.yt_id.is_empty() {
                self.yt_id = c.youtube.client_id.clone();
                self.yt_secret = c.youtube.client_secret.clone();
            }
        }
        ui.horizontal(|ui| {
            ui.label("Client-ID");
            ui.add(egui::TextEdit::singleline(&mut self.yt_id).desired_width(340.0));
        });
        ui.horizontal(|ui| {
            ui.label("Client-Secret");
            ui.add(egui::TextEdit::singleline(&mut self.yt_secret).password(true).desired_width(340.0));
        });
        ui.horizontal(|ui| {
            if ui.button("Speichern").clicked() {
                let body = serde_json::json!({"youtube": {"client_id": self.yt_id, "client_secret": self.yt_secret}});
                self.post("/api/config", body);
            }
            if ui.button("Mit YouTube verbinden").clicked() {
                self.yt_connect();
            }
            if !self.yt_state.is_empty() {
                ui.label(&self.yt_state);
            }
        });
        ui.small("Einmalig: Google-Client anlegen, Daten hier eintragen, dann verbinden.");

        if let Some(c) = cfg {
            if let Some(ing) = Some(&c.ingest) {
                ui.add_space(12.0);
                ui.label("Eingang für OBS (quer)");
                ui.monospace(format!("Server: {}", ing.server));
                ui.monospace(format!("Schlüssel: {}", ing.key));
                ui.add_space(6.0);
                ui.label("Eingang für OBS (vertikal)");
                ui.monospace(format!("Server: {}", c.ingest_v.server));
                ui.monospace(format!("Schlüssel: {}", c.ingest_v.key));
                ui.small("Im Aitum-Multistream-Dock: zweite Ausgabe mit diesen Werten, Leinwand = Aitum Vertical");
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
        ui.horizontal(|ui| {
            ui.label("OBS-Passwort");
            let before = l.obs_password.clone();
            ui.add(
                egui::TextEdit::singleline(&mut l.obs_password)
                    .password(true)
                    .desired_width(380.0),
            );
            if l.obs_password != before {
                *self.local.lock().unwrap() = l.clone();
                local_save(&l);
            }
        });
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
