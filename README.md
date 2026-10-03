# Auralis v2

[![Version](https://img.shields.io/badge/version-2.6.27-6366f1.svg)](Cargo.toml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](Cargo.toml)
[![CI/CD](https://github.com/Intro0siddiqui/Auralis/actions/workflows/build.yml/badge.svg)](https://github.com/Intro0siddiqui/Auralis/actions/workflows/build.yml)

A lightweight, offline-first music player with integrated media downloading and P2P synchronization — rewritten from scratch in **Rust + Tauri + HTMX** to replace the original Kotlin/Compose Multiplatform application.

> **Status: Active Development.** Core architecture is in place and most features are implemented (database, audio playback, downloads, playlists, settings, P2P networking). Sync transfers use real libp2p request-response.

---

## Why a rewrite?

| Component | Auralis v1 (old) | Auralis v2 (new) |
| :--- | :--- | :--- |
| **Core language** | Kotlin (JVM) | **Rust** |
| **UI framework** | Compose Multiplatform | **HTMX + Vanilla HTML/CSS** |
| **Desktop runtime** | JVM (100MB+) | **Tauri (system WebView, < 50MB)** |
| **Mobile runtime** | Android (ExoPlayer) | **Tauri Android v2** |
| **Database** | SQLDelight (SQLite) | **SQLite via `rusqlite`** |
| **Media engine** | `yt-dlp` external | **`youtubei.js` resolver + pure-Rust `reqwest` streaming (no external binaries)** |

The rewrite trades the JVM's startup cost and Compose's runtime overhead for a tiny, single-binary distribution that targets both desktop and mobile from one codebase.

---

## Core Features

| Feature | Status | Notes |
| :--- | :--- | :--- |
| **Music Library** | Implemented | SQLite-backed; scanner extracts metadata via lofty + Android native MediaStore scanner with instant OS metadata ingestion |
| **Audio Playback** | Implemented | rodio 0.22 + Symphonia with target-conditional Opus codec (rusty-opus 64-bit SIMD + pure-Rust 32-bit decoder; mp3/mp4/flac/vorbis/wav/ogg/opus/webm) with queue, shuffle, repeat, seek, auto-advance watcher, background `MediaPlaybackService` |
| **YouTube Search & Streaming** | Implemented | In-app YouTube song/artist/album search, instant live stream playback with real-time player bar & full-screen UI sync, one-click downloading with format and quality preferences |
| **Media Downloading** | Implemented | `youtube.js` URL resolution (PO-token `&pot=` unconditional, `403` auto-retry) + `reqwest` streaming to `app_data_dir/downloads/<title>.<ext>`, progress tracking, pause/resume/cancel; Android MediaStore dual-save |
| **Playlist Management** | Implemented | Full CRUD with SQLite persistence |
| **P2P Networking** | Implemented | libp2p with mDNS, gossipsub, request-response |
| **Settings** | Implemented | SQLite-backed load/save |
| **P2P Sync Transfers** | Implemented | Real libp2p request-response transfer (best-effort) |
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
│   │   ├── download.html
│   │   ├── search.html
│   │   ├── sync.html
│   │   └── settings.html
│   └── icons/
│       └── auralis.svg
├── src/
│   ├── main.rs             # Binary entry point
│   ├── lib.rs              # Library root + Tauri builder + Android NDK context
│   ├── domain/             # Pure business logic (no I/O)
│   │   ├── models/         # Track, Album, Artist, Playlist, Settings, Sync, Download
│   │   ├── repositories/   # Repository traits
│   │   └── services/       # Service implementations
│   ├── infrastructure/     # External integrations
│   │   ├── database/       # SQLite + rusqlite
│   │   ├── filesystem/     # Track scanner (DesktopScanner + AndroidScanner native MediaStore ingestion) + metadata
│   │   ├── media/          # AudioPlayer (rodio/cpal/oboe + Opus decoder) + Downloader (reqwest streaming of resolved URLs)
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
├── icons/                  # App icon suite (128x128, 128x128@2x, icon.icns, icon.ico, etc.)
├── gen/                    # Generated schemas + Android project
├── scripts/
│   ├── build.sh
│   ├── dev.sh
│   └── test.sh
└── .github/workflows/
    └── build.yml           # CI/CD: Linux, macOS, Windows, Android (Tag-Gated)
```

The architecture is **Domain-Driven**: the `domain` layer is pure Rust types and trait definitions with no infrastructure dependencies. The `infrastructure` layer provides concrete implementations (SQLite, filesystem, etc.) that satisfy the domain's traits. The `commands` layer exposes Tauri-callable functions that wire everything together.

---

## Building

### Prerequisites
- **Rust** 1.89 or newer (lofty's MSRV)
- **Node.js** (only for the dev server / hot-reload)
- **Tauri v2 prerequisites** — see <https://v2.tauri.app/start/prerequisites/>
- Node.js is only needed for the JS E2E test scripts (`scripts/tests/*.js`). YouTube audio needs no external binary — the built-in `youtube.js` resolver plus Rust `reqwest` streaming handle everything on-device.

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

## YouTube Search & Live Audio Streaming

Auralis v2.6.26 brings native YouTube song search and instant live audio streaming:

- **Search & Download inside the Download Tab**: Direct in-app search for tracks, artists, and albums without leaving the application. Results display rich metadata including album art thumbnails, channel names, and durations.
- **Live Stream Playback**: Listen to tracks immediately via live audio streaming without waiting for downloads to finish:
  - Real-time player bar integration with play/pause, volume control, and seeking.
  - Full-screen player (`player-full`) modal synchronization with live duration and position tracking.
  - System `MediaSession` synchronization for desktop and mobile lockscreen/notification controls.
- **One-Click Downloading**: Download any search result or pasted URL with a single click. Configure format preferences (`mp3`, `m4a`, `opus`, `flac`, `wav`) and audio quality options (Best, 320kbps, 256kbps, 128kbps) before starting.
- **PO-Token & Multi-Client Resolution**: Integrated PO-token generation (`WebPoMinter` with `GenerateIT` protobuf) and multi-client fallback rotation (`TV`, `ANDROID_VR`, `IOS`, `ANDROID`) ensuring resilient playback without external Python or `yt-dlp` binaries.

---

## Media Downloading (Pure-Rust Engine)

Auralis v2 features a **pure-Rust media downloader** with zero mandatory external binaries:

- **YouTube resolution (`youtube.js`, vendored `youtubei.js`)**: Runs in the app's webview and resolves a direct `googlevideo` audio URL using PO-token-aware InnerTube clients (6 clients `TV`/`ANDROID_VR`→`IOS` with `effectiveOrderedClients`/`retryClients` rotation, `&pot=` unconditional via `po_token.js` `GenerateIT` protobuf, `n`/`signatureCipher` decipher), with client-matched headers.
- **Native Direct Audio Streaming (`reqwest`)**: Streams the resolved URL (and direct HTTPS audio files — `.mp3`, `.flac`, `.m4a`, `.wav`, `.aac`) natively in Rust to `app_data_dir/downloads/` (`sanitize_filename` + UUID dedup, `*.jpg` sidecar, scanned via `AndroidScanner`/`DesktopScanner`) with live byte progress tracking + `403` auto-retry (`downloads.js` `TV→ANDROID+pot→WEB_SAFARI`). On Android **dual-save** publishes a copy to `Download/Auralis/` via `MediaStore.Downloads` (`IS_PENDING` on API 29+, `MediaScanner` legacy) when `Settings.downloads.use_system_downloads` is enabled (default `true`); library scan stays sandboxed to avoid duplicate entries. No Python, no sidecar.
  - **MediaStore publish fix (v2.6.66)**: The `MediaStore.Downloads` insert used the column name `"display_name"` (without underscore), but the correct column name is `"_display_name"` (with underscore prefix) — `MediaStore.MediaColumns.DISPLAY_NAME` has been `"_display_name"` since API 1. This caused `IllegalArgumentException: Invalid column display_name` on 100% of downloads, making every downloaded file invisible in Files. The fix is a one-line change: `const COLUMN_DISPLAY_NAME: &str = "_display_name";` in `src/infrastructure/media/android_downloads.rs`, enforced by a `const _: () = assert!(..)` so the build fails if it ever reverts.
  - **Egress binding, and a retraction (v2.6.68 → v2.6.69)**: A googlevideo URL carries `ip=`, the **client's** address, and the CDN checks that the request egresses from it — proven by tampering with `ip=` alone and turning a 206 into a 403. v2.6.68 tried to exploit that with `resolve_to_addrs(host, ip)`, which is the opposite of what `ip=` means: it asked the phone to *be* the CDN, so every download failed to connect. v2.6.69 uses `local_address(ip)` — binding the source socket, which is what the parameter actually constrains — with a one-shot unpinned fallback for when a pinned address is not bindable (IPv6 privacy addresses rotate, RFC 4941).
    - **What was retracted:** the 403 was *not* an egress mismatch. Unpinned `curl` returns HTTP 206 on the very tracks that 403'd in the app, on the same phone and the same residential line. The binding check exists, but our requests were not tripping it. The pin is kept only because it is harmless once corrected and correct-by-construction; it is not credited with fixing the 403.
  - **Retry ladder (`downloads.js`)**: Walks url *classes* rather than re-asking clients — `['muxed', 'adaptive', 'opus']` — each rung gated on the per-client report so a class nobody can serve is never requested. `muxed` is first because it is the only class measured to serve on this network (HTTP 206), while adaptive audio-only (itag 140) and opus (itag 251) are measured **403 at byte 0**; adaptive is the better file and should return to first place the day it stops being refused. Transport failures (`error sending request`, DNS, connect) count as retry-worthy: they are a refusal by nobody, and the next rung is a different CDN hostname.

---

## Native Audio Playback & Opus Engine

Auralis provides low-latency, cross-platform audio playback built on `rodio` and Symphonia:

- **Target-Conditional Opus Codec**:
  - **64-bit Targets (`aarch64`, `x86_64`)**: SIMD-accelerated Opus decoding via `rusty-opus` with ARM NEON support for ultra-low power consumption.
  - **32-bit Targets (`armv7`, `i686`)**: Pure-Rust `opus-decoder` fallback guaranteeing portable cross-compilation across all platforms.
- **Universal Format Support**: Native playback for MP3, M4A/AAC, FLAC, OGG Vorbis, WAV, and WebM/Opus.
- **Playback Controls**: Full queue management, shuffle, repeat-all, repeat-one, seek, volume normalization, and background playback via an Android foreground `MediaPlaybackService`.

---

## Android MediaStore Native Scanner

Modern Android storage architecture requires efficient indexing:

- **Native MediaStore Ingestion**: Queries Android's `MediaStore.Audio` content provider directly over JNI via `AndroidScanner`, retrieving track metadata, album art, and audio durations instantly without slow filesystem walks.
- **Scoped Storage Compliance**: Seamlessly handles Android 10+ (API 29 to 36) Scoped Storage restrictions, supporting internal app-sandboxed libraries alongside public `Download/Auralis/` dual-saving.

---

## License

MIT — see the original Auralis v1 repository for details.
