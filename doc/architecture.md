# Software-Architektur von `mosh-tcp`

`mosh-tcp` ist ein latenztolerantes, bandbreitenoptimiertes Terminal-System (Client & Server) in **Rust**, das speziell für Verbindungen über Mobilfunk-Tethering, hohe Netzwerklatenzen und doppelte **CGNAT-Umgebungen** (ohne direkte UDP-Portweiterleitung) entwickelt wurde.

---

## 1. Architektur-Überblick & Zielsetzung

Bei klassischen SSH-Verbindungen führt jedes getippte Zeichen und jede Konsolenausgabe zu direkten TCP-Paketen. Über langsame Mobilfunkverbindungen mit hoher Latenz und schwankender Bandbreite führt das bei Programmen mit viel Bildschirmausgabe (z. B. `tmux`, `kiro-cli`, `seq`, Build-Server) zur vollständigen Überlastung des Netzwerks und zum Einfrieren der Eingabe.

`mosh-tcp` löst dieses Problem durch ein **geframeites State-Sync-Protokoll über TCP** mit den folgenden Kernprinzipien:

* **Bandbreiten-Deckelung (Token Bucket Rate Limiter):** Einstellbare maximale Netzwerkauslastung (z. B. max. 6 KB/s).
* **Frame Batching & Scrollback-Kappung (Frame Skipping):** PTY-Ausgaben werden in konfigurierbaren Intervallen (z. B. 20 ms / 50 FPS) zusammengefasst. Bei riesigen Datenmengen werden ältere Zwischenzustände verworfen, damit die Latenz nicht ansteigt.
* **Payload-Komprimierung:** Frames über 128 Bytes werden automatisch via Deflate komprimiert.
* **Predictive Local Echo:** Sofortiges lokales Feedback für getippte Zeichen mit automatischer Bereinigung bei Server-Rückmeldung.
* **Query Stripping:** Serverseitiges Herausfiltern von ANSI-Terminalabfragen (`\x1b[>c`, `\x1b]10;?`, `\x1b]11;?`), um Prompt-Salat bei `tmux` oder `vim` zu verhindern.

---

## 2. Komponenten-Architektur (Diagramm)

```mermaid
flowchart TD
    subgraph Client ["Client Machine (z.B. Linux Laptop)"]
        RawTerm["Raw Mode Terminal Input"] --> ClientLoop["Client Event Loop (src/client.rs)"]
        ClientLoop --> Predictor["Local Predictor Engine (src/predictive.rs)"]
        ClientLoop --> EncodedInput["Framed Packet (Packet::ClientInput)"]
        FrameRender["Stdout Frame Renderer"] <-- DecodedFrame["Decoded Frame (Packet::ServerFrame)"]
    end

    subgraph Network ["TCP Connection / SSH Tunnel"]
        EncodedInput -->|TCP Socket / Port 4000| ServerLoop
        ServerLoop -->|Rate-Limited Frames (max 6 KB/s)| DecodedFrame
    end

    subgraph Server ["Remote Server Machine"]
        ServerLoop["Server Manager (src/server.rs)"] --> PTYWriter["PTY Master Writer"]
        PTYWriter --> Shell["PTY Slave ($SHELL / bash / tmux)"]
        Shell --> PTYReader["PTY Master Reader"]
        PTYReader --> QueryFilter["Terminal Query Filter (strip_terminal_queries)"]
        QueryFilter --> PTYBuffer["PTY Buffer (max 16 KB)"]
        PTYBuffer --> TokenBucket["Token Bucket Rate Limiter & Compressor"]
        TokenBucket --> ServerLoop
        Telemetry["Telemetry Monitor (--stats)"] --- ServerLoop
    end
```

---

## 3. Detailbeschreibung der Module

### 3.1 `src/protocol.rs` – Binär-Protokoll & Framing
Definiert das Paketformat (`Packet` Enum) und den ländencodierten Framed-Codec (`PacketCodec`):
* `Packet::ClientInput { data: Vec<u8> }`: Rohdaten von der Tastatur.
* `Packet::ClientResize { rows, cols }`: Fenstergrößenänderungen (SIGWINCH).
* `Packet::ServerFrame { seq, data, compressed }`: Konsolidiertes Terminal-Ausgabeframe.
* `Packet::Ping` / `Packet::Pong`: Latenzmessung (RTT).

Das Framing nutzt `bincode` zur Serialisierung und `tokio_util::codec` mit einem 4-Byte Big-Endian Längenheader.

### 3.2 `src/server.rs` – PTY-Steuerung, Rate Limiting & Telemetrie
Das Herzstück der Serverkomponente:
1. **PTY System:** Öffnet ein Pseudo-Terminal mittels `portable-pty` und startet die Shell.
2. **Terminal Query Stripping:** Filtert serverseitig ANSI-Farbabfragen (`OSC 10/11`) und Device-Attribute-Abfragen (`DA / DA2`), bevor sie den Client erreichen.
3. **Token Bucket Rate Limiter:**
   * Ein Eimer füllt sich mit Tokens entsprechend der maximalen Bandbreite (z. B. 6.144 Bytes/s bei `--max-kbps 6`).
   * Übersteigt die PTY-Puffergröße 16 KB (z. B. bei riesigen Text-Dumps), wird der Puffer gekappt, älterer Text verworfen und nur die neuesten 8 KB behalten.
4. **Telemetrie-Monitor (`--stats`):** Misst serverseitig in Echtzeit PTY-Eingangsrate, Netz-Ausgangsrate, Kompressionsrate, verworfene kB, RTT-Latenz und Frames.

### 3.3 `src/client.rs` – Raw Mode, Event-Loop & Filtering
1. **Terminal Raw Mode:** Schaltet das lokale Terminal in den Raw-Modus (`crossterm`).
2. **ResponseFilter:** Ein Sicherheitsfilter auf Client-Seite, der eventuell durchgerutschte Terminal-Antwort-Strings (`0;2501;1c]10;rgb:...`) abfängt und verwirft.
3. **Frame Renderer:** Liest empfangene `ServerFrame`-Pakete, dekomprimiert sie bei Bedarf und gibt sie auf `stdout` aus.

### 3.4 `src/predictive.rs` – Predictive Local Echo Engine
Ermöglicht sofortiges Feedback beim Tippen bei hohen Latenzen:
* Zeichnet getippte Zeichen sofort unterstrichen auf den Bildschirm.
* **`clear_predictions`:** Sobald ein echtes `ServerFrame` vom Server eintrifft, werden die lokal vorhergesagten Zeichen per Backspace (`\x08 \x08`) gelöscht, sodass die Antwort des Servers die Vorschau nahtlos überschreibt.

---

## 4. Leistungs- & Größenoptimierung

* **Rust Edition 2024:** Nutzung modernster Rust-Standards.
* **Cargo Release Profil (`profile.release`):**
  * `opt-level = "z"` (Optimierung auf Dateigröße).
  * `lto = true` & `codegen-units = 1` (Link-Time Optimization).
  * `panic = "abort"` & `strip = true` (Entfernen aller Debug-Symbole & Panic-Tabellen).
* **UPX Executable Packing:** Das fertige Binary wird mittels `./build_release.sh` per UPX LZMA von **2,3 MB auf ~305 kB** komprimiert.
