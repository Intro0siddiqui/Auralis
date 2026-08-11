# Auralis v2

A lightweight, offline-first music player with integrated media downloading and P2P synchronization — rewritten from scratch in **Rust + Tauri + HTMX** to replace the original Kotlin/Compose Multiplatform application.

> **Status: Active Development.** Core architecture is in place and most features are implemented (database, audio playback, downloads, playlists, settings, P2P networking). Sync transfers are still simulated.

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
| **Music Library** | Implemented | SQLite-backed; scanner extracts metadata via lofty |
| **Audio Playback** | Implemented | rodio with queue, shuffle, repeat, seek |
| **Media Downloading** | Implemented | yt-dlp subprocess with progress tracking |
| **Playlist Management** | Implemented | Full CRUD with SQLite persistence |
| **P2P Networking** | Implemented | libp2p with mDNS, gossipsub, request-response |
| **Settings** | Implemented | SQLite-backed load/save |
| **P2P Sync Transfers** | Simulated | DB persistence works; actual transfer uses simulated progress |
| **Smart Playlists** | Partial | Criteria model exists; built-in "Recently Added" / "Most Playlists" not pre-defined |

---

## Project layout

```
auralis-v2/
├── Cargo.toml              # Rust package manifest
├── build.rs                # tauri-build hook
├── tauri.conf.json         # Tauri v2 configuration
├── capabilities/           # Tauri permission grants
│   └── default.json
├── ui/                     # Soft Glass Audio frontend (HTMX + vanilla HTML/CSS)
│   ├── index.html          # App shell (sidebar + content + player bar + mobile nav)
│   ├── styles/
│   │   ├── tokens.css      # Design variables (glass/neu shadows, blur, radii)
│   │   ├── base.css        # CSS reset + app-shell layout grid
│   │   ├── components.css  # Glass, Neu, Buttons, Cards, Track rows
│   │   └── responsive.css  # Mobile/tablet/desktop breakpoints
│   ├── js/
│   │   ├── bridge.js       # Tauri event listeners + player bar updates
│   │   └── player.js       # Progress/seek logic + keyboard shortcuts
│   ├── partials/           # HTMX fragments served by the Rust backend
│   │   ├── nav.html
│   │   ├── home.html
│   │   ├── library.html
│   │   ├── albums.html
│   │   ├── artists.html
│   │   ├── playlists.html
│   │   ├── player-full.html
│   │   ├── sync.html
│   │   └── settings.html
│   └── icons/
│       ├── auralis.svg
│       └── plus.svg
├── src/
│   ├── main.rs             # Binary entry point
│   ├── lib.rs              # Library root + Tauri builder
│   ├── domain/             # Pure business logic (no I/O)
│   │   ├── models/         # Track, Album, Artist, Playlist, Settings, Sync, Download
│   │   ├── repositories/   # Repository traits
│   │   └── services/       # Service implementations
│   ├── infrastructure/     # External integrations
│   │   ├── database/       # SQLite + rusqlite
│   │   ├── filesystem/     # Track scanner + metadata
│   │   ├── media/          # AudioPlayer (rodio) + Downloader (yt-dlp)
│   │   └── network.rs      # libp2p: mDNS, gossipsub, request-response
│   ├── commands/           # Tauri command handlers
│   │   ├── library.rs
│   │   ├── playback.rs
│   │   ├── downloads.rs
│   │   ├── playlists.rs
│   │   ├── sync.rs
│   │   ├── settings.rs
│   │   └── templates.rs    # Serves ui/partials/ as HTMX responses
│   └── templates/mod.rs    # Reads ui/partials/ and caches them
├── icons/                  # App icons (32x32.png, icon.ico)
├── gen/                    # Generated schemas + Android project
├── scripts/
│   ├── build.sh
│   ├── dev.sh
│   └── test.sh
└── .github/workflows/
    └── build.yml           # CI/CD: Linux, macOS, Windows, Android
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

The UI is built on **HTMX 2.0** — no React, no Vue, no client-side framework bloat. The design system is **Soft Glass Audio** — a hybrid of glassmorphism (`.glass`, `.glass-weak`, `.glass-strong` with `backdrop-filter: blur()`) and neumorphism (`.neu`, `.neu-inset`, `.neu-glass` with dual box-shadows).

### Why HTMX over Compose?
- **Zero JavaScript framework runtime** — the browser only loads ~30KB of HTMX
- **Server-driven** — the Rust backend serves static HTML partials from `ui/partials/`
- **Progressive enhancement** — the app degrades gracefully if JavaScript is disabled
- **Same codebase for desktop and mobile** — no separate Compose-for-Android layer

### How navigation works
```html
<!-- index.html loads content via HTMX on page load -->
<main id="content" class="content"
      hx-get="/partials/home"
      hx-trigger="load"
      hx-swap="innerHTML">
</main>
```

The Tauri command `commands::templates::render_template` reads the corresponding HTML file from `ui/partials/` and returns it as-is for HTMX to swap into the DOM. Smaller updates (now-playing, download progress) are handled via Tauri events in `js/bridge.js`.

---

## Contributing

See [AGENTS.md](AGENTS.md) for detailed implementation guidelines and the roadmap.

---

## License

MIT — see the original Auralis v1 repository for details.
