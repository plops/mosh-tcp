# Project Dependencies & GitHub Repositories

Dieses Dokument listet alle externen Rust-Abhängigkeiten von `mosh-tcp` inklusive der jeweiligen GitHub-Organisationen und Repositories auf (aktualisiert auf die neuesten Crate-Releases).

| Crate | Version | GitHub Organisation / Repo | Beschreibung |
| :--- | :--- | :--- | :--- |
| **`tokio`** | `1.53` | [`tokio-rs/tokio`](https://github.com/tokio-rs/tokio) | Asynchrone Runtime (TCP, Timeouts, Tasks) |
| **`tokio-util`** | `0.7` | [`tokio-rs/tokio`](https://github.com/tokio-rs/tokio) | Framing & Length-Prefixed Codec |
| **`bytes`** | `1.12` | [`tokio-rs/bytes`](https://github.com/tokio-rs/bytes) | Effiziente Byte-Puffer |
| **`portable-pty`** | `0.9` | [`wez/wezterm`](https://github.com/wez/wezterm) | Pseudo-Terminal (PTY) Abstraktion von WezTerm |
| **`crossterm`** | `0.29` | [`crossterm-rs/crossterm`](https://github.com/crossterm-rs/crossterm) | Raw-Mode Terminal-Steuerung & Eingaben |
| **`clap`** | `4.6` | [`clap-rs/clap`](https://github.com/clap-rs/clap) | CLI Argument Parser |
| **`serde`** | `1.0` | [`serde-rs/serde`](https://github.com/serde-rs/serde) | Serialisierungs-Framework |
| **`bincode`** | `1.3` | [`bincode-org/bincode`](https://github.com/bincode-org/bincode) | Binäres Paketformat |
| **`flate2`** | `1.1` | [`rust-lang/flate2-rs`](https://github.com/rust-lang/flate2-rs) | Gzip/Deflate Komprimierung für Frames |
| **`futures`** | `0.3` | [`rust-lang/futures-rs`](https://github.com/rust-lang/futures-rs) | Streams & Sinks Abstraktion |
| **`anyhow`** | `1.0` | [`dtolnay/anyhow`](https://github.com/dtolnay/anyhow) | Error Handling |
