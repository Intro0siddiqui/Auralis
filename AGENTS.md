# AGENTS.md — Auralis v2 Development Guide

This guide describes the architecture, conventions, and implementation roadmap for Auralis v2. It is intended for both human developers and AI coding agents.

---

## 1. Project Overview

Auralis v2 is a Tauri-based desktop/mobile music player written in Rust. It uses HTMX for the frontend (no JS framework), static HTML partials for server-side rendering, SQLite for persistence, and a pure-Rust `rusty_ytdl` downloader (with an optional `yt-dlp` CLI fallback) for media downloads.

**Current State: Active Development** — Core architecture is in place and most features are implemented. Remaining work is primarily around polish and a few partial features (built-in smart-playlist presets). For the verified 2026 platform-compliance status (16 KB alignment ✅, but the Android background-media foreground-service *type* and macOS/Windows signing are current gaps), see `PROJECT.md` §11.

---

## 2. Architecture

### Layer Structure

```
src/
├── domain/           # Pure business logic — no I/O, no external deps
│   ├── models/       # Track, Album, Artist, Playlist, Settings, Sync, Download
│   ├── repositories/ # Repository traits
│   └── services/     # Service implementations (LibraryService, PlaybackService, etc.)
├── infrastructure/   # Concrete implementations of domain traits
│   ├── database/     # SQLite via rusqlite + migration schema
│   ├── filesystem/   # File scanner + metadata extraction (lofty)
│   ├── media/        # AudioPlayer (rodio) + Downloader (rusty_ytdl / optional yt-dlp)
│   └── network.rs    # libp2p: mDNS, gossipsub, request-response, Noise transport
├── commands/         # Tauri command handlers — bridge frontend ↔ services
├── templates/        # Partial server — reads ui/partials/ and caches them
└── lib.rs            # App builder + command registration
```

**Dependencies flow inward**: `commands` → `domain` + `infrastructure`. The `domain` layer depends on nothing external. `infrastructure` depends on `domain` + third-party crates. `commands` depend on all layers.

### Frontend (Soft Glass Audio)

The frontend lives in `ui/` and uses **HTMX 2.0** for SPA-like navigation:

```
ui/
├── index.html          # App shell (sidebar + content + player bar + mobile nav)
├── styles/
│   ├── tokens.css      # Design variables (--glass-*, --neu-*, --blur-*, --radius-*)
│   ├── base.css        # CSS reset + app-shell grid layout
│   ├── components.css  # .glass, .glass-weak, .glass-strong, .neu, .neu-inset, .neu-glass, .card, .track-row
│   └── responsive.css  # Mobile/tablet/desktop breakpoints + safe-area insets (notches/bars)
├── js/
│   ├── bridge.js       # Dynamic data bridge (get_tracks, scan, play, downloads, HTMX swap routing)
│   └── player.js       # Progress bar, seeking, MediaSession API, hardware keys, keyboard shortcuts
├── partials/           # HTMX fragments served by the Rust backend
│   ├── nav.html, home.html, library.html, albums.html
│   ├── artists.html, playlists.html, player-full.html
│   ├── download.html, search.html, sync.html, settings.html
└── icons/              # auralis.svg
```

**Design language**: Glassmorphism (`.glass`, `.glass-weak`, `.glass-strong` with `backdrop-filter: blur()`) + Neumorphism (`.neu`, `.neu-inset`, `.neu-glass` with dual box-shadows).

**How navigation works**: `index.html` loads `#content` via `hx-get="/partials/home"` on page load. Sidebar links use `hx-get="/partials/<view>" hx-target="#content"`.

**The Rust backend serves these partials** via `commands/templates.rs` → `render_template(name)` which reads `ui/partials/{name}.html` and returns it as-is.

### Key Conventions

- **Never use `unwrap()`/`panic!()` in production code** — all fallible operations must return `Result` or `Option`.
- **All Tauri commands return `Result<T, String>`** on the wire — internal errors are logged via `tracing` and converted to `String` for the frontend.
- **Templates are static HTML partials** — the backend reads `ui/partials/*.html` and returns them as-is for HTMX swaps.
- **State is managed via Tauri's `manage()`** — `Database`, `AudioPlayer`, `Settings`, `SyncService`, `Discovery`, `SyncEngine` are registered in the setup hook.
- **`#[allow(dead_code)]` is used** in service structs for fields reserved for future use — do not remove without understanding the intent.

---

## 3. Implementation Roadmap

### Phase 1: Foundation — ✅ COMPLETE

| Task | Status |
|------|--------|
| Database repositories (`infrastructure/database/repositories.rs`) | ✅ Fully implemented — 871 lines of real SQL |
| Library scanner (`infrastructure/filesystem/scanner.rs`) | ✅ Glob + lofty metadata extraction |
| Library commands (`commands/library.rs`) | ✅ All commands return real data from SQLite |

### Phase 2: Playback — ✅ COMPLETE

| Task | Status |
|------|--------|
| Audio player (`infrastructure/media/player.rs`) | ✅ rodio with queue, shuffle, repeat, seek |
| Playback commands (`commands/playback.rs`) | ✅ All commands wired to AudioPlayer |

### Phase 3: Downloads — ✅ COMPLETE

| Task | Status |
|------|--------|
| Download pipeline (`infrastructure/media/downloader.rs`) | ✅ `rusty_ytdl` native stream + optional `yt-dlp` fallback, progress tracking |
| Download commands (`commands/downloads.rs`) | ✅ Real native download + optional yt-dlp invocation |

### Phase 4: Playlists — ✅ COMPLETE

| Task | Status |
|------|--------|
| Playlist commands (`commands/playlists.rs`) | ✅ Full CRUD with SQLite persistence |

### Phase 5: P2P Networking — ✅ COMPLETE

| Task | Status |
|------|--------|
| libp2p networking (`infrastructure/network.rs`) | ✅ 865 lines: mDNS, gossipsub, request-response, Noise transport |
| Sync service (`domain/services/sync_service.rs`) | ✅ DB persistence, QR/PIN pairing |
| Sync commands (`commands/sync.rs`) | ✅ All commands wired to SyncService |

### Phase 6: Remaining Work

| Task | Status | Notes |
|------|--------|-------|
| Real P2P data transfer | ✅ Implemented | `sync_with_device()` performs a real libp2p request-response transfer (best-effort) |
| Library scanner | ✅ Implemented | `infrastructure/filesystem/scanner.rs` (glob + lofty); Android 16 SAF / system media-picker import added |
| Settings commands | ✅ Implemented | SQLite-backed load/save |
| Smart playlists | ⚠️ Partial | Criteria model exists; built-in "Recently Added" / "Most Played" not pre-defined |
| Android assets | ✅ Done | PNG mipmaps present under `icons/android/mipmap-*`; custom obsidian logo applied (v2.0.31) |

---

## 4. Optimization Tasks

### 4.1 Dependency Cleanup (`Cargo.toml`) — ✅ DONE

The recommended feature set is already in `Cargo.toml`:
```toml
tokio = { version = "1", default-features = false, features = ["rt-multi-thread", "macros", "sync", "process", "io-util", "time"] }
rodio = { version = "0.17", default-features = false, features = ["symphonia-aac", "symphonia-mp3", "symphonia-flac", "symphonia-wav"] }
image = { version = "0.25", default-features = false, features = ["png", "jpeg", "ico"] }
```

`cargo check` compiles and the binary is optimized via `[profile.release]` (`lto = "fat"`, `opt-level = "z"`, `strip = true`).

### 4.2 tauri.conf.json Cleanup — ✅ DONE

- `bundle.targets` is `["deb", "app", "dmg", "msi", "nsis"]` (no `"all"`).
- `identifier` is `com.auralis.v2` (was `com.auralis.app`).
- `version` is `2.0.31` and must stay in sync with `Cargo.toml` + `Cargo.lock`.

### 4.3 Android CI Optimization (`.github/workflows/build.yml`) — ⚠️ PARTIAL

- The APK is built for **`aarch64` only** (`cargo tauri android build --apk --target aarch64`); `x86_64-linux-android` was not added.
- `cargo tauri android init` is still run (guarded with `|| true`) before the build — not removed, but harmless.
- `libc++_shared.so` is bundled for the `arm64-v8a` ABI via `.cargo/config.toml` (`-lc++_shared`) and copied into `jniLibs` during CI.
- Android permissions (`READ_MEDIA_AUDIO`, `READ/WRITE_EXTERNAL_STORAGE`, `MANAGE_EXTERNAL_STORAGE`, foreground-service media) are injected into `AndroidManifest.xml` at build time.

**Verify**: CI produces a single `aarch64` APK. Add `x86_64` only if emulator testing is needed.

### 4.4 Linker Optimization — ⚠️ PARTIAL

`[profile.release]` already sets `codegen-units = 1`, `opt-level = "z"`, `lto = "fat"`, `strip = true`, `panic = "abort"`.

`lld`/`mold` is **not** wired in `.cargo/config.toml` (the host `aarch64-unknown-linux-gnu` target only adds `-lc`). To speed up local host linking, add:
```toml
[target.x86_64-unknown-linux-gnu]
rustflags = ["-C", "link-arg=-fuse-ld=lld"]
```
(Note: host `cargo build` cannot link on this machine regardless — `webkit2gtk-4.1` is missing. Only CI builds the host binary.)

---

## 5. Testing

### Running Tests

```bash
bash scripts/test.sh
# or
cargo test --all-features
cargo test --test integration -- --nocapture   # end-to-end scan / ingestion
```

### Test Gaps

`tests/integration.rs` exists and is run in CI (`build-linux`/`build-macos`/`build-windows`/`test` jobs). Unit tests exist for some domain models (`PairingInfo::generate`, `SyncChange::new`, `PairedDevice::mark_synced`). Extend coverage to:

1. **Domain models** — extend unit tests to all models.
2. **Database layer** — Test repository CRUD operations with a temp SQLite DB.
3. **Command handlers** — Add integration tests using `#[tauri::test]` macro.
4. **File scanner** — Test scanning with a temp directory containing fixture files.

### Test Conventions

- Use `tempfile` for filesystem-based tests.
- Use `:memory:` SQLite for repository tests.
- Place unit tests in each `src/` module with `#[cfg(test)] mod tests`.
- Place integration tests in `tests/integration.rs`.

---

## 6. Code Style

- **Rust**: Run `cargo fmt` before committing. `cargo clippy --all-targets` must pass with no warnings.
- **HTML/Templates**: Use 2-space indentation. HTMX attributes prefixed with `hx-`.
- **CSS**: Vanilla CSS, no preprocessors. Use CSS variables for theming.
- **Commits**: Follow conventional commits (`feat:`, `fix:`, `chore:`).

---

## 7. CI/CD Pipeline

The CI workflow (`.github/workflows/build.yml`) runs on every push/PR to `main` and on tags `v*` (no `workflow_dispatch` — releases are triggered only by tag pushes):

| Job | Purpose |
|-----|---------|
| `build-linux` | Compiles release binary, packages tar.gz |
| `build-macos` | Compiles + bundles `.dmg` (x86_64 + aarch64) |
| `build-windows` | Compiles MSVC target, bundles `.msi` / `.exe` |
| `build-android` | Builds signed `aarch64` APK (auto-generates keystore, injects Android permissions, bundles `libc++_shared.so`) |
| `test` | Runs `cargo test --all-features` (+ `cargo test --test integration` on the OS matrix) |
| `lint` | Runs `cargo fmt --check` + `cargo clippy --all-targets --all-features` |
| `release` | Creates GitHub Release from all `release-*` artifacts (tags only; `needs` the build/test/lint jobs) |

**Key CI files to modify for optimization**:
- `.github/workflows/build.yml` — Android job
- `tauri.conf.json` — bundle section
- `Cargo.toml` — dependency features

---

## 8. Quick Start for Agents

1. **Read this file** completely.
2. **Pick a Phase** from the roadmap above. Start with Phase 6 (Remaining Work).
3. **Follow the verify criteria** for each task — if you can't verify, the implementation is incomplete.
4. **Run lint + tests** before finishing: `bash scripts/test.sh` and check for clippy warnings.
5. **Update this file** (`AGENTS.md`) with any new findings, blockers, or completed work.
6. **Do not commit** unless explicitly instructed — deliver changes via a diff or patch summary.

---

## 9. Key Files by Concern

| Concern | Primary Files |
|---------|--------------|
| **App setup** | `src/lib.rs` |
| **Command registration** | `src/lib.rs` (invoke_handler!) |
| **Database schema** | `src/infrastructure/database/repositories.rs` |
| **Track model** | `src/domain/models/track.rs` |
| **Sync model** | `src/domain/models/sync.rs` |
| **Network implementation** | `src/infrastructure/network.rs` |
| **Android CI / Release** | `.github/workflows/build.yml` (tag-gated; builds Linux/macOS/Windows/Android + GitHub Release) |
| **Integration tests** | `tests/integration.rs` |
| **Build config** | `Cargo.toml`, `Cargo.lock`, `tauri.conf.json`, `.cargo/config.toml` |
