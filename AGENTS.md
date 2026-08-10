# AGENTS.md — Auralis v2 Development Guide

This guide describes the architecture, conventions, and implementation roadmap for Auralis v2. It is intended for both human developers and AI coding agents.

---

## 1. Project Overview

Auralis v2 is a Tauri-based desktop/mobile music player written in Rust. It uses HTMX for the frontend (no JS framework), Askama for server-side HTML templating, SQLite for persistence, and yt-dlp as a sidecar for downloads.

**Current State: Early Prototype** — The architecture is scaffolded but most command handlers are stubs returning empty data or `"not yet implemented"` errors.

---

## 2. Architecture

### Layer Structure

```
src/
├── domain/           # Pure business logic — no I/O, no external deps
│   ├── models/       # Track, Album, Artist, Playlist, Settings, Sync, Download
│   ├── repositories/ # Traits only (TrackRepository, PlaylistRepository, etc.)
│   └── services/     # Traits only (LibraryService, PlaybackService, etc.)
├── infrastructure/   # Concrete implementations of domain traits
│   ├── database/     # SQLite via rusqlite + migration schema
│   ├── filesystem/   # File scanner + metadata extraction (lofty)
│   ├── media/        # AudioPlayer (rodio) + Downloader (yt-dlp)
│   └── network.rs    # libp2p stubs (Discovery, SyncEngine)
├── commands/         # Tauri command handlers — bridge frontend ↔ services
├── templates/        # Askama structs + render() function
└── lib.rs            # App builder + command registration
```

**Dependencies flow inward**: `commands` → `domain` + `infrastructure`. The `domain` layer depends on nothing external. `infrastructure` depends on `domain` + third-party crates. `commands` depend on all layers.

### Key Conventions

- **Never use `unwrap()`/`panic!()` in production code** — all fallible operations must return `Result` or `Option`.
- **All Tauri commands return `Result<T, String>`** on the wire — internal errors are logged via `tracing` and converted to `String` for the frontend.
- **Templates use Askama** — return rendered HTML strings, not JSON.
- **State is managed via Tauri's `manage()`** — `Database`, `AudioPlayer`, and `Settings` are registered in the setup hook (`lib.rs:77`).
- **`#[allow(dead_code)]` is used** in service structs for fields reserved for future use — do not remove without understanding the intent.

---

## 3. Implementation Roadmap

### Phase 1: Foundation (Immediate Priority)

#### 3.1 Wire Database Repositories (`infrastructure/database/repositories.rs`)
- **Status**: Schema exists, repository traits exist, implementations are stubs.
- **Action**: Implement all `TrackRepository`, `PlaylistRepository`, `SettingsRepository`, `SyncRepository` trait methods against the SQLite connection.
- **Verify**: `cargo test --lib` passes. Database round-trips work.

#### 3.2 Wire Library Scanner (`infrastructure/filesystem/scanner.rs` → `commands/library.rs`)
- **Status**: File scanner exists but `scan_library_paths()` returns `ScanSummary { tracks_added: 0, ... }`.
- **Action**: Connect scanner to `TrackRepository`. Actually scan configured paths, extract metadata via `lofty`, insert tracks into DB.
- **Verify**: Scanning a directory with an MP3 adds ≥1 track to the database.

#### 3.3 Implement Library Commands (`commands/library.rs`)
- **Status**: `get_tracks`, `get_track`, `search_tracks` return empty/stub.
- **Action**: Inject `Arc<Database>` via Tauri state, call repository methods, return real data.
- **Verify**: `get_tracks` returns tracks after a scan. `search_tracks` returns matching results.

### Phase 2: Playback (High Priority)

#### 3.4 Wire Audio Player (`infrastructure/media/player.rs` → `commands/playback.rs`)
- **Status**: `AudioPlayer` struct exists using `rodio`. All playback commands return stubs.
- **Action**: Register `AudioPlayer` in Tauri state. Implement `play`, `pause`, `stop`, `seek`, `set_volume`, queue management.
- **Verify**: Playing a track produces audio. `get_now_playing` returns non-null after `play`.

### Phase 3: Downloads (High Priority)

#### 3.5 Implement Download Pipeline (`infrastructure/media/downloader.rs` → `commands/downloads.rs`)
- **Status**: `Downloader` struct exists but no commands invoke `yt-dlp`.
- **Action**: Connect downloader to Tauri state. Implement async download execution with progress tracking. Persist downloads to DB.
- **Verify**: `download_audio` spawns yt-dlp subprocess and tracks progress.

### Phase 4: Playlists (Medium Priority)

#### 3.6 Complete Playlist Commands (`commands/playlists.rs`)
- **Status**: `create_playlist` works in-memory but doesn't persist. Other CRUD ops are stubs.
- **Action**: Inject `PlaylistRepository`. Persist playlists and track relationships.
- **Verify**: Creating a playlist persists to DB. Restart retains playlists.

### Phase 5: P2P Sync (Medium Priority)

#### 3.7 Implement libp2p Networking (`infrastructure/network.rs`)
- **Status**: `Discovery` and `SyncEngine` are stubs with `#[allow(dead_code)]`.
- **Action**: Implement mDNS discovery, Noise transport, GossipSub for change broadcast, Request-Response for bulk transfers.
- **Verify**: Two instances on the same LAN discover each other. Sync status updates after exchange.

#### 3.8 Wire Sync Service (`domain/services/sync_service.rs` → `commands/sync.rs`)
- **Status**: `SyncService` has full in-memory logic but `commands/sync.rs` returns `Err("not yet implemented")`.
- **Action**: Inject `SyncService` into commands. Replace mock device creation with real network pairing.
- **Verify**: `complete_pairing` validates PIN and saves the device. `sync_with_device` performs actual transfer.

### Phase 6: Polish (Lower Priority)

#### 3.9 Settings UI Wiring (`commands/settings.rs`)
- **Status**: Stub returns `Settings::default()`.
- **Action**: Persist settings to DB via `SettingsRepository`. Expose settings mutation commands.

#### 3.10 Smart Playlists (`commands/playlists.rs::create_smart_playlist`)
- **Action**: Implement criteria-based track resolution using `TrackFilter`.

#### 3.11 Android Assets
- Add PNG mipmaps (`mipmap-mdpi/png`, `mipmap-hdpi/png`, etc.).
- Remove SVG-only icons from the Tauri config bundle paths.

---

## 4. Optimization Tasks

### 4.1 Dependency Cleanup (`Cargo.toml`)

Currently `reqwest` and `libp2p` are imported but unused/stubbed. `tokio` uses `["full"]` and `rodio`/`image` use default features.

```toml
# Before
tokio = { version = "1", features = ["full"] }
rodio = "0.17"
image = "0.25"
reqwest = { version = "0.12", default-features = false, features = ["json", "stream", "rustls-tls"] }
libp2p = { version = "0.54", features = ["tcp", "mdns", "noise", "yamux", "gossipsub", "request-response", "tokio"] }

# After (immediate)
tokio = { version = "1", features = ["rt-multi-thread", "macros", "sync", "process", "io-util", "time"] }
rodio = { version = "0.17", default-features = false, features = ["symphonia-mp3", "symphonia-flac", "symphonia-wav"] }
image = { version = "0.25", default-features = false, features = ["png"] }
# Remove reqwest entirely if HTTP fetching is not needed
# Keep libp2p but only if Phase 5 is implemented; otherwise remove
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

The CI workflow (`.github/workflows/build.yml`) runs on every push/PR to `main`:

| Job | Purpose |
|-----|---------|
| `build-linux` | Compiles release binary, packages tar.gz |
| `build-macos` | Compiles for x86_64 + aarch64 |
| `build-windows` | Compiles MSVC target, packages zip |
| `build-android` | Builds APK (see optimization notes above) |
| `test` | Runs `cargo test --all-features` |
| `lint` | Runs `cargo fmt --check` + `cargo clippy` |
| `release` | Creates GitHub Release with all artifacts |

**Key CI files to modify for optimization**:
- `.github/workflows/build.yml` — Android job (lines 117–263)
- `tauri.conf.json` — bundle section
- `Cargo.toml` — dependency features

---

## 8. Quick Start for Agents

1. **Read this file** completely.
2. **Pick a Phase** from the roadmap above. Start with Phase 1 (Foundation).
3. **Follow the verify criteria** for each task — if you can't verify, the implementation is incomplete.
4. **Run lint + tests** before finishing: `bash scripts/test.sh` and check for clippy warnings.
5. **Update this file** (`AGENTS.md`) with any new findings, blockers, or completed work.
6. **Do not commit** unless explicitly instructed — deliver changes via a diff or patch summary.

---

## 9. Key Files by Concern

| Concern | Primary Files |
|---------|--------------|
| **App setup** | `src/lib.rs:75-98` |
| **Command registration** | `src/lib.rs:99-151` |
| **Database schema** | `src/infrastructure/database/repositories.rs` |
| **Track model** | `src/domain/models/track.rs` |
| **Sync model** | `src/domain/models/sync.rs` |
| **Network stubs** | `src/infrastructure/network.rs:22-63` |
| **Android CI** | `.github/workflows/build.yml:117-263` |
| **Build config** | `Cargo.toml`, `tauri.conf.json` |
