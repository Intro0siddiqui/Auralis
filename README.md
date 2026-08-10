# Auralis v2

A lightweight, offline-first music player with integrated media downloading and P2P synchronization — rewritten from scratch in **Rust + Tauri + HTMX** to replace the original Kotlin/Compose Multiplatform application.

> **Status: Early prototype.** Core architecture is in place (domain-driven design, SQLite schema, Tauri app lifecycle), but most command handlers are stubs. See the [Missing Features](#missing--incomplete-features) section below for details.

---

## Why a rewrite?

| Component | Auralis v1 (old) | Auralis v2 (new) |
| :--- | :--- | :--- |
| **Core language** | Kotlin (JVM) | **Rust** |
| **UI framework** | Compose Multiplatform | **HTMX + Vanilla HTML/CSS** |
| **Desktop runtime** | JVM (100MB+) | **Tauri (system WebView, < 50MB)** |
| **Mobile runtime** | Android (ExoPlayer) | **Tauri Android v2** |
| **Database** | SQLDelight (SQLite) | **SQLite via `rusqlite`** |
| **Media engine** | `yt-dlp` external | **`yt-dlp` sidecar orchestrated by Rust** |

The rewrite trades the JVM's startup cost and Compose's runtime overhead for a tiny, single-binary distribution that targets both desktop and mobile from one codebase.

---

## Core Features

| Feature | Status | Notes |
| :--- | :--- | :--- |
| **Music Library** | Partial | DB schema & scanner exist; command handlers return empty results |
| **Audio Playback** | Partial | `rodio` integrated; no tracks actually loaded or played yet |
| **Media Downloading** | Partial | `yt-dlp` infra exists; commands are stubs pending wiring |
| **Playlist Management** | Partial | Models and repo traits exist; CRUD commands return stubs |
| **P2P Sync (QR Pairing)** | Partial | QR code + PIN generation works; libp2p networking is stubbed |
| **Settings** | Partial | DB persistence layer exists; UI wiring pending |
| **Smart Playlists** | Not started | Criteria model exists; resolution logic missing |

---

## Missing / Incomplete Features

### Critical Gaps

1. **P2P Networking (libp2p)** — `infrastructure/network.rs` contains stubs only. mDNS discovery, gossipsub broadcasts, and request-response transfers are not implemented. The `sync_service` uses simulated progress instead of real transfers.

2. **Playback Engine** — `infrastructure/media/player.rs` exists but `commands/playback.rs` returns `Err("play not yet implemented")` for all actions. The `AudioPlayer` struct is not wired to the domain services.

3. **Download Pipeline** — `infrastructure/media/downloader.rs` exists but `commands/downloads.rs` never invokes `yt-dlp`. Downloads return a `Pending` status and never execute.

4. **Library Scanner** — `infrastructure/filesystem/scanner.rs` exists but `commands/library.rs` returns empty track lists. File system enumeration and metadata extraction are not wired.

5. **Playlist Operations** — Most playlist commands (update, add/remove tracks, reorder, smart playlists) return `Err("not yet implemented")`.

### Build & Infrastructure Debt

6. **Unused Dependencies** — `reqwest` is imported but never used. `libp2p` is imported but only stubs exist.

7. **Bloated Features** — `tokio` uses `features = ["full"]` (should use `["rt-multi-thread", "macros", "sync", "process", "io-util", "time"]`). `rodio` uses default features (should trim to specific codecs). `image` uses default features (should use `["png"]` only).

8. **tauri.conf.json** — Has `"targets": "all"` causing duplicate builds. App identifier is `com.auralis.app` (should be `com.auralis.v2`).

9. **Android CI** — Builds unnecessary ABIs (armv7, i686). Has redundant `cargo tauri android init` step that can cause build conflicts.

### Project Structure Gaps

10. **Missing `requirements/` directory** — No feature specification documents.

11. **Missing Android icons** — Only SVG assets exist; no PNG mipmaps or proper Android icon structure.

12. **Missing CHANGELOG.md** — No changelog for v2.0.0 release.

13. **Missing test coverage** — `scripts/test.sh` exists but the project has very few tests. Most command handlers have no corresponding unit or integration tests.

---

## Project layout

```
auralis-v2/
├── Cargo.toml              # Rust package manifest
├── build.rs                # tauri-build hook
├── tauri.conf.json         # Tauri v2 configuration
├── capabilities/           # Tauri permission grants
│   └── default.json
├── ui/                     # Frontend (HTMX + vanilla HTML/CSS/JS)
│   ├── index.html
│   ├── downloads.html
│   ├── settings.html
│   ├── css/auralis.css
│   ├── js/auralis.js
│   ├── partials/           # HTMX partials for dynamic swaps
│   └── icons/              # App icons (SVG only — PNG missing)
├── templates/              # Askama HTML templates (compiled in)
│   ├── layout.html
│   ├── library.html
│   ├── albums.html
│   ├── artists.html
│   ├── downloads.html
│   ├── playlists.html
│   ├── playlist_detail.html
│   ├── sync.html
│   ├── settings.html
│   └── partials/
├── src/
│   ├── main.rs             # Binary entry point
│   ├── lib.rs              # Library root + Tauri builder
│   ├── domain/             # Pure business logic (no I/O)
│   │   ├── models/
│   │   ├── repositories/   # Repository traits
│   │   └── services/       # Service traits
│   ├── infrastructure/     # External integrations
│   │   ├── database/       # SQLite + rusqlite
│   │   ├── filesystem/     # Track scanner + metadata
│   │   ├── media/          # Audio player (rodio) + downloader (yt-dlp)
│   │   └── network.rs      # P2P discovery (libp2p) — stubs only
│   ├── commands/           # Tauri command handlers (mostly stubs)
│   │   ├── library.rs
│   │   ├── playback.rs
│   │   ├── downloads.rs
│   │   ├── playlists.rs
│   │   ├── sync.rs
│   │   ├── settings.rs
│   │   └── templates.rs
│   └── templates/          # Askama wrapper structs
└── scripts/
    ├── build.sh
    ├── dev.sh
    └── test.sh
```

The architecture is **Domain-Driven**: the `domain` layer is pure Rust types and trait definitions with no infrastructure dependencies. The `infrastructure` layer provides concrete implementations (SQLite, filesystem, etc.) that satisfy the domain's traits. The `commands` layer exposes Tauri-callable functions that wire everything together.

---

## Building

### Prerequisites
- **Rust** 1.75 or newer
- **Node.js** (only for the dev server / hot-reload)
- **Tauri v2 prerequisites** — see <https://v2.tauri.app/start/prerequisites/>
- **yt-dlp** on the system PATH (or bundled as a sidecar)

### Commands

```bash
# Build the production binary
bash scripts/build.sh

# Run the tests
bash scripts/test.sh

# Launch in development mode (hot-reload of UI)
bash scripts/dev.sh
```

Or directly via Cargo:

```bash
# Build the Rust backend + Tauri shell
cargo build --release

# Run the tests
cargo test --all-features

# Run with hot-reload
cargo tauri dev
```

---

## Frontend architecture

The UI is built on **HTMX 2.0** — no React, no Vue, no client-side framework bloat. Every interaction is an HTTP-style request to a Tauri command, which returns an HTML fragment that HTMX swaps into the DOM.

### Why HTMX over Compose?
- **Zero JavaScript framework runtime** — the browser only loads ~30KB of HTMX
- **Server-rendered** — the Rust backend renders HTML directly using Askama templates
- **Progressive enhancement** — the app degrades gracefully if JavaScript is disabled
- **Same codebase for desktop and mobile** — no separate Compose-for-Android layer

### How navigation works
```html
<!-- A link with hx-get tells HTMX to fetch HTML and swap it into #main -->
<a href="/library.html" hx-get="/api/templates/library" hx-push-url="true">Library</a>
```

The Tauri command `commands::templates::render_template` returns the Askama-rendered HTML for the requested page. Smaller updates (now-playing, download progress) are polled every 1–2 seconds via partials.

---

## Contributing

See [AGENTS.md](AGENTS.md) for detailed implementation guidelines and the roadmap for completing missing features.

---

## License

MIT — see the original Auralis v1 repository for details.