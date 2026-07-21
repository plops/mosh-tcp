# Software-Architektur von `mosh-tcp`

`mosh-tcp` ist ein latenztolerantes, bandbreitenoptimiertes Terminal-System (Client & Server), das speziell für Verbindungen über Mobilfunk-Tethering, hohe Netzwerklatenzen und doppelte **CGNAT-Umgebungen** (ohne direkte UDP-Portweiterleitung) entwickelt wurde.

---

## 1. Architektur-Überblick & Zielsetzung

Bei klassischen SSH-Verbindungen führt jedes getippte Zeichen und jede Konsolenausgabe zu direkten TCP-Paketen. Über langsame Mobilfunkverbindungen mit hoher Latenz und schwankender Bandbreite führt das bei Programmen mit viel Bildschirmausgabe (z. B. `tmux`, `kiro-cli`, `seq`, Build-Server) zur vollständigen Überlastung des Netzwerks und zum Einfrieren der Eingabe.

`mosh-tcp` löst dieses Problem durch ein **entkoppeltes Client/Server-System mit geframeitem TCP-State-Sync** und den folgenden Kernprinzipien:

* **Multi-Client Ökosystem (Rust, C, C++ Trennlinie):**
  * `mosh-tcp-client-c`: Ein ultrakompakter POSIX C99/C11 Client (~26 KB raw, **14 KB UPX LZMA**) für Embedded Devices, OpenWrt Router und Micro-Gäste.
  * `mosh-tcp-client-cpp`: Ein moderner C++20 Client (~40 KB raw, **19 KB UPX LZMA**) mit RAII `TerminalGuard`, `std::span` Bounds-Safety und `std::variant` Type-Safety.
  * `mosh-tcp-client`: Der Rust Standalone-Client (~514 KB raw, **219 KB UPX LZMA**) mit 2D Virtual Terminal Cell Predictive Local Echo.
  * `mosh-tcp-server`: Ein asynchroner Tokio-Server mit PTY-Anbindung (`portable-pty`), 20ms-Frame-Synthese, Bandbreiten-Limiter und VT100-State-Parser.
* **Bandbreiten-Deckelung (Token Bucket Rate Limiter):** Einstellbare maximale Netzwerkauslastung (z. B. max. 6 KB/s).
* **Frame Batching & Scrollback-Kappung (Frame Skipping):** PTY-Ausgaben werden in 20 ms Intervallen (50 FPS) gebündelt. Bei Datenströmen über 16 KB wird der Puffer automatisch gekappt.
* **Payload-Komprimierung:** Deflate-Komprimierung für Frames über 128 Bytes. Die C- und C++-Clients nutzen einen integrierten, zero-dependency `puff.c`-Decompressor (Mark Adler Inflate Engine + Gzip Header Parser).
* **2D Grid Predictive Local Echo (Rust Client):** Sofortige Vorhersage von Tastatureingaben auf einem 2D VT100-Zellgitter mit automatischer Bestätigung/Verwerfung bei Server-Frames.
* **Terminal Query Stripping:** Serverseitiges Herausfiltern von ANSI-Terminalabfragen (`\x1b[>c`, `\x1b]10;?`, `\x1b]11;?`), um Prompt-Salat bei `tmux` oder `vim` zu verhindern.

---

## 2. Komponenten-Architektur (Diagramm)

```mermaid
flowchart TD
    subgraph Clients ["Client Implemenationen"]
        subgraph CClient ["mosh-tcp-client-c (POSIX C99, ~14 KB UPX)"]
            CTerm["termios Raw Mode / SIGWINCH"] --> CLoop["poll() Multiplexer (clients/c/mosh_tcp_client.c)"]
            CLoop --> CPuff["puff.c Deflate/Gzip Inflate Engine"]
        end

        subgraph CPPClient ["mosh-tcp-client-cpp (Modern C++20, ~19 KB UPX)"]
            CPPTerm["RAII TerminalGuard / std::span"] --> CPPLoop["std::variant Deserializer (clients/cpp/mosh_tcp_client.cpp)"]
            CPPLoop --> CPPPuff["puff.c Deflate/Gzip Inflate Engine"]
        end

        subgraph RustClient ["mosh-tcp-client (Rust 2024, ~219 KB UPX)"]
            RustTerm["crossterm Raw Mode & Mouse Engine"] --> RustLoop["Event Loop (src/client.rs)"]
            RustLoop --> Predictor["2D Cell Predictor Engine (src/predictive.rs)"]
        end
    end

    subgraph Network ["TCP Socket Connection / SSH Tunnel"]
        CClient -->|Length-Prefixed Binary Packets| ServerLoop
        CPPClient -->|Length-Prefixed Binary Packets| ServerLoop
        RustClient -->|Length-Prefixed Binary Packets| ServerLoop
        ServerLoop -->|20ms Rate-Limited Frames (50 FPS)| CClient
        ServerLoop -->|20ms Rate-Limited Frames (50 FPS)| CPPClient
        ServerLoop -->|20ms Rate-Limited Frames (50 FPS)| RustClient
    end

    subgraph Server ["mosh-tcp-server (Tokio Async, PTY Manager)"]
        ServerLoop["Server Manager (src/server.rs)"] --> PTYWriter["PTY Master Writer"]
        PTYWriter --> Shell["PTY Slave ($SHELL / bash / tmux)"]
        Shell --> PTYReader["PTY Master Reader"]
        PTYReader --> QueryFilter["ANSI Query Filter (src/ansi.rs)"]
        QueryFilter --> PTYBuffer["PTY Buffer & Token Bucket"]
        PTYBuffer --> TokenBucket["GzEncoder / Deflate Frame Generator"]
        TokenBucket --> ServerLoop
        Telemetry["Telemetry Monitor (--stats)"] --- ServerLoop
    end
```

---

## 3. Detailbeschreibung der Module

### 3.1 `src/protocol.rs` – Zero-Dependency Binär-Protokoll & Framing
Definiert das Paketformat (`Packet` Enum) und den leichtgewichtigen 1-Byte Tag Binary Codec:
* `Tag 1` - `Packet::ClientInput { data: Vec<u8> }`: Tastatureingaben.
* `Tag 2` - `Packet::ClientResize { rows, cols }`: Fenstergrößenänderung (SIGWINCH).
* `Tag 3` - `Packet::Ping { timestamp }` / `Tag 4` - `Packet::Pong { timestamp }`: RTT-Latenzmessung.
* `Tag 5` - `Packet::ServerFrame { seq, data, compressed }`: Konsolidiertes Terminal-Ausgabeframe.

Das Framing nutzt einen 4-Byte Big-Endian Längenheader (`[uint32_t len][payload]`), der von allen Client-Implementierungen (Rust, C, C++) identisch interpretiert wird.

### 3.2 `src/server.rs` – PTY-Steuerung, Rate Limiting & Session-State
Das Herzstück der Serverkomponente:
1. **PTY System:** Öffnet ein Pseudo-Terminal mittels `portable-pty` und führt die Nutzershell aus.
2. **Session-State Konsolidierung:** Zusammenfassung von Terminal-Zustand, PTY-Puffern und Telemetrie in einer thread-sicheren `ServerSessionState`-Struktur.
3. **Token Bucket Rate Limiter:** Garantiert, dass die Netzwerkauslastung das konfigurierte Limit (z. B. 6 KB/s) nicht überschreitet.
4. **Atomic VT100 Frame Generator:** Verhindert ANSI-Escape-Sequenz-Korruption bei großen Datenströmen (> 16 KB) durch Erzeugung atomarer Bildschirmframes (`\x1b[H\x1b[2J`).

### 3.3 `src/client.rs` & `src/bin/mosh_tcp_client.rs` – Synchroner Rust Client
1. **Multi-Threading Architektur:**
   - Thread 1: Tastatureingaben & Crossterm-Events -> `std::sync::mpsc::channel`
   - Thread 2: Netzwerk-Sender (Liest Channel -> schreibt in Socket)
   - Main Thread: Netzwerk-Empfänger (Blockiert auf Socket -> rendert Frames)
2. **SGR 1006 Extended Mouse Engine:** Übersetzt Mausereignisse (Klick, Drag, Scroll) in SGR 1006 Sequenzen (`\x1b[<{button};{col};{row}M/m`).

### 3.4 `clients/c/` – Standalone POSIX C99/C11 Client
1. **`mosh_tcp_client.c`:** POSIX C99/C11 Implementation mit `socket()`, `termios` Raw Mode, `poll()` Input-Multiplexing und `SIGWINCH`/`SIGINT`/`SIGTERM` Signal-Handlern.
2. **`puff.c` & `puff.h`:** Mark Adler Inflate Decompressor + Gzip Stream Header Parser für **zero-dependency** Compilation ohne `-lz`.
3. **Target Binary:** `mosh-tcp-client-c` (**14 KB UPX LZMA**).

### 3.5 `clients/cpp/` – Standalone Modern C++20 Client
1. **`mosh_tcp_client.cpp`:** C++20 Implementation mit RAII `TerminalGuard` für automatische Terminal-Wiederherstellung im Destruktor, `std::span` für bounds-checked Zero-Copy Deserialisierung und `std::variant` / `std::visit` für typsicheres Pattern Matching.
2. **Target Binary:** `mosh-tcp-client-cpp` (**19 KB UPX LZMA**).

### 3.6 `src/predictive.rs` & `src/ansi.rs` – 2D Grid Predictive Echo Engine
* Hält ein 2D-Zellraster (`PredictorState`) des lokalen Terminals.
* Zeichnet getippte Zeichen sofort unterstrichen am vorhergesagten Cursor-Ort.
* Verarbeitet Cursor-Bewegungen, Alt-Tasten, Mouse-Events und Paste-Modi.
* **Cursor Confirmation:** Eintreffende `ServerFrame`-Signale vergleichen den Serverzustand mit den Vorschauen und setzen Vorhersagen nahtlos zurück.

---

## 4. Leistungs- & Größenmetriken

| Komponente / Target | Sprache / Standard | Uncompressed ELF Size | UPX LZMA Executable | Hauptabhängigkeiten | Target-Umgebung |
| :--- | :--- | :---: | :---: | :--- | :--- |
| **`mosh-tcp-client-c`** | POSIX C99 / C11 | 26 KB | **14 KB** | libc (zero external deps) | OpenWrt Router, Embedded, Micro-VMs |
| **`mosh-tcp-client-cpp`** | Modern C++20 | 40 KB | **19 KB** | libstdc++ / libc | Minimal Container, Low-RAM Linux |
| **`mosh-tcp-client`** | Rust 2024 Edition | 504 KB | **219 KB** | `crossterm`, `vt100`, `vte`, `std` | Linux Desktop, Laptops |
| **`mosh-tcp-server`** | Rust (Tokio Async) | 682 KB | **284 KB** | `tokio`, `portable-pty`, `flate2`, `vt100` | Remote Linux Server |

---

## 5. GitHub Release Pipeline & Artefakte

Der automatisierte Release-Workflow (`.github/workflows/release.yml`) erzeugt bei neuen Versionstags (`v*`) vier optimierte Release-Artefakte:

1. **`mosh-tcp-client-c-linux-amd64`**: Ultrakompaktes POSIX C99 Standalone-Client-Binary (**14 KB UPX LZMA**).
2. **`mosh-tcp-client-cpp-linux-amd64`**: Modernes C++20 Standalone-Client-Binary (**19 KB UPX LZMA**).
3. **`mosh-tcp-client-linux-amd64`**: Standalone Rust Client-Binary (**219 KB UPX LZMA**).
4. **`mosh-tcp-linux-amd64.tar.gz`**: Komprimiertes Archiv mit allen Client-Binaries (`mosh-tcp-client`, `mosh-tcp-client-c`, `mosh-tcp-client-cpp`), Server (`mosh-tcp-server`) und Dokumentation.
