# mosh-tcp

Ein in **Rust** geschriebenes, latenztolerantes Terminal-Tool (Client & Server) für extrem verzögerte Netzwerkverbindungen (z. B. Smartphone-Tethering, Mobilfunk) und **CGNAT-Umgebungen**.

## Warum `mosh-tcp`?

* **Mosh** nutzt gewöhnlich UDP. Bei doppeltem CGNAT (sowohl serverseitig als auch clientseitig) scheitern UDP-Verbindungen oft ohne komplexes Hole-Punching oder VPN-Relays.
* **`mosh-tcp`** nutzt ein **geframeites TCP-Protokoll**. Dadurch kann die Verbindung einfach über normale TCP-Verbindungen, Reverse-Tunnel oder **SSH-Port-Forwarding** (`ssh -L 4000:localhost:4000`) getunnelt werden.

## Hauptmerkmale

1. **Frame Rate Limiting (z. B. 20ms / 50 FPS):**
   * Statt jedes einzelne Byte einzeln über das Netzwerk zu schicken (was bei Befehlen wie `cat` oder `cargo build` das Netzwerk mit tausenden kleinen Paketen überflutet), sammelt der Server alle PTY-Ausgaben innerhalb eines Zeitfensters (z.B. 20ms) und sendet sie gebündelt in einem einzigen Frame.
2. **Payload-Komprimierung:**
   * Frames, die größer als 128 Bytes sind, werden automatisch mit Deflate/Gzip komprimiert, um Bandbreite bei langsamen Mobilfunkverbindungen zu sparen.
3. **Predictive Local Echo:**
   * Tastatureingaben werden auf Client-Seite sofort unterstrichen gerendert, noch bevor die Bestätigung des Servers eingetroffen ist.
4. **PTY & Signal-Handling:**
   * Vollwertige Pseudo-Terminal (PTY) Anbindung via `portable-pty`.
   * Fenstergrößenänderungen (SIGWINCH / Resize) werden dynamisch vom Client an den Server übertragen.

---

## Bauen (Build)

```bash
cd /workspace/src/mosh-tcp
cargo build --release
```

Das fertige Binary befindet sich in `target/release/mosh-tcp`.

---

## Verwendung

### 1. Server auf dem Remote-Server starten

```bash
./target/release/mosh-tcp server --bind 0.0.0.0:4000 --fps 50
```

Optionen:
* `--bind <IP:PORT>`: IP und Port für den Server (Standard: `0.0.0.0:4000`).
* `--fps <FPS>`: Bildwiederholrate in Frames pro Sekunde (Standard: `50` = 20ms pro Frame).
* `--shell <SHELL>`: Pfad zur gewünschten Shell (Standard: `$SHELL` oder `/bin/bash`).

---

### 2. Verbinden vom Client (Linux Laptop)

#### Option A: Direkt über TCP (z.B. im selben VPN oder mit öffentlicher Server-IP)
```bash
./target/release/mosh-tcp client --connect <SERVER_IP>:4000
```

#### Option B: Über SSH-Tunnel (Empfohlen bei CGNAT auf Server- oder Client-Seite)
1. **SSH-Tunnel aufbauen:**
   ```bash
   ssh -N -L 4000:localhost:4000 user@dein-remote-server.de
   ```
2. **`mosh-tcp` lokal verbinden:**
   ```bash
   ./target/release/mosh-tcp client --connect 127.0.0.1:4000
   ```

Optionen für den Client:
* `--connect <IP:PORT>`: Ziel-Adresse des Servers (Standard: `127.0.0.1:4000`).
* `--no-predict`: Deaktiviert das lokale Predictive Echo.
* **Beenden:** Mit `Ctrl + Q` kann der Client jederzeit beendet werden.

---

## Projektstruktur

* `src/main.rs`: CLI Argument Parsing mit `clap`.
* `src/server.rs`: PTY-Spawn (`portable-pty`), 20ms Timer Loop, Frame-Akkumulation & Komprimierung.
* `src/client.rs`: Raw-Mode Terminal-Steuerung (`crossterm`), Event-Loop & Frame-Rendering.
* `src/protocol.rs`: Binäres Paket-Format (`bincode` & `tokio-util` Length-Prefixed Codec).
* `src/predictive.rs`: Lokales Predictive Echo Engine.
