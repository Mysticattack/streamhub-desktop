# StreamHub Desktop

Native Steuer-App (Rust + egui) fuer den StreamHub-Server.

Zeigt Live-Status und Stream-Laufzeit, verwaltet Multistream-Ziele, Kodierung,
Server-Aufnahme und die Sitzungs-Historie.

## Start

    ./streamhub

Beim ersten Start unter **Einstellungen** Server-Adresse und Token eintragen
(werden in `~/.config/streamhub-desktop/local.json` gespeichert).

## Bauen

    sudo apt install pkg-config libx11-dev libxkbcommon-dev libwayland-dev libgl1-mesa-dev
    cargo build --release

Das Binary liegt danach unter `target/release/streamhub`.
