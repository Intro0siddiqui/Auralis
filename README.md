# Auralis v2

A lightweight, offline-first music player with integrated media downloading and P2P synchronization — rewritten from scratch in **Rust + Tauri + HTMX** to replace the original Kotlin/Compose Multiplatform application.

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
│   └── icons/
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
│   │   └── network.rs      # P2P discovery (libp2p)
│   ├── commands/           # Tauri command handlers
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

## License

MIT — see the original Auralis v1 repository for details.
