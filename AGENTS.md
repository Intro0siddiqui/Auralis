# AGENTS.md — Auralis v2 Development Guide

This guide describes the architecture, conventions, and implementation roadmap for Auralis v2. It is intended for both human developers and AI coding agents.

---

## 1. Project Overview

Auralis v2 is a Tauri-based desktop/mobile music player written in Rust. It uses HTMX for the frontend (no JS framework), static HTML partials for server-side rendering, SQLite for persistence, and yt-dlp as a sidecar for downloads.

**Current State: Active Development** — Core architecture is in place and most features are implemented. Remaining work is primarily around real P2P data transfer (currently simulated) and polish.

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
│   ├── media/        # AudioPlayer (rodio) + Downloader (yt-dlp)
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
│   ├── sync.html, settings.html
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
| Download pipeline (`infrastructure/media/downloader.rs`) | ✅ yt-dlp subprocess with progress tracking |
| Download commands (`commands/downloads.rs`) | ✅ Real yt-dlp invocation |

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
| Real P2P data transfer | ⚠️ Simulated | `sync_with_device()` uses `tokio::time::sleep` to simulate progress |
| Library service scanner | ⚠️ Stubbed | `library_service.rs:scan_path()` returns empty — use `infrastructure/filesystem/scanner.rs` instead |
| Settings commands | ✅ Implemented | SQLite-backed load/save |
| Smart playlists | ⚠️ Partial | Criteria model exists; built-in "Recently Added" / "Most Played" not pre-defined |
| Android assets | ⚠️ Missing | No PNG mipmaps; only `32x32.png` and `icon.ico` exist |

---

## 4. Optimization Tasks

### 4.1 Dependency Cleanup (`Cargo.toml`)

```toml
# Current
tokio = { version = "1", features = ["full"] }
rodio = { version = "0.17", features = ["symphonia-aac"] }
image = "0.25"

# Recommended
tokio = { version = "1", features = ["rt-multi-thread", "macros", "sync", "process", "io-util", "time"] }
rodio = { version = "0.17", default-features = false, features = ["symphonia-mp3", "symphonia-flac", "symphonia-wav", "symphonia-aac"] }
image = { version = "0.25", default-features = false, features = ["png"] }
```

**Verify**: `cargo check` still compiles. Binary size reduced.

### 4.2 tauri.conf.json Cleanup

- Remove `"targets": "all"` from `bundle` section.
- Change identifier from `com.auralis.app` to `com.auralis.v2`.

### 4.3 Android CI Optimization (`.github/workflows/build.yml`)

- Reduce Android targets to `aarch64-linux-android` and `x86_64-linux-android` only.
- Remove the redundant `cargo tauri android init` step.
- Update the `apk` build command to specify targets: `cargo tauri android build --apk --target aarch64,x86_64`.

**Verify**: CI builds complete faster. APKs are produced for the correct architectures.

### 4.4 Linker Optimization

Enable `lld` or `mold` for faster linking:

```toml
# Build profile
[profile.release]
codegen-units = 1
opt-level = "z"
lto = true
strip = true
```

Add to `.cargo/config.toml`:

```toml
[target.x86_64-unknown-linux-gnu]
rustflags = ["-C", "link-arg=-fuse-ld=lld"]
```

---

## 5. Testing

### Running Tests

```bash
bash scripts/test.sh
# or
cargo test --all-features
```

### Test Gaps

The current test suite is minimal. Add tests for:

1. **Domain models** — `PairingInfo::generate`, `SyncChange::new`, `PairedDevice::mark_synced` have unit tests; extend to all models.
2. **Database layer** — Test repository CRUD operations with a temp SQLite DB.
3. **Command handlers** — Add integration tests using `#[tauri::test]` macro.
4. **File scanner** — Test scanning with a temp directory containing fixture files.

### Test Conventions

- Use `tempfile` for filesystem-based tests.
- Use `:memory:` SQLite for repository tests.
- Place unit tests in each `src/` module with `#[cfg(test)] mod tests`.
- Place integration tests in `tests/` directory (to be created).

---

## 6. Code Style

- **Rust**: Run `cargo fmt` before committing. `cargo clippy --all-targets` must pass with no warnings.
- **HTML/Templates**: Use 2-space indentation. HTMX attributes prefixed with `hx-`.
- **CSS**: Vanilla CSS, no preprocessors. Use CSS variables for theming.
- **Commits**: Follow conventional commits (`feat:`, `fix:`, `chore:`).

---

## 7. CI/CD Pipeline

The CI workflow (`.github/workflows/build.yml`) runs on every push/PR to `main` and on tags `v*`:

| Job | Purpose |
|-----|---------|
| `build-linux` | Compiles release binary, packages tar.gz |
| `build-macos` | Compiles for x86_64 + aarch64 |
| `build-windows` | Compiles MSVC target, packages zip |
| `build-android` | Builds signed APK (auto-generates keystore) |
| `test` | Runs `cargo test --all-features` |
| `lint` | Runs `cargo fmt --check` + `cargo clippy` |
| `release` | Creates GitHub Release with all artifacts (tags only) |

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
| **Android CI** | `.github/workflows/build.yml` |
| **Build config** | `Cargo.toml`, `tauri.conf.json` |
