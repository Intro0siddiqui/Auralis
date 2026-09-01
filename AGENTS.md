# AGENTS.md — Auralis v2 Development Guide

This guide describes the architecture, conventions, and implementation roadmap for Auralis v2. It is intended for both human developers and AI coding agents.

---

## 1. Project Overview

Auralis v2 is a Tauri-based desktop/mobile music player written in Rust. It uses HTMX for the frontend (no JS framework), static HTML partials for server-side rendering, SQLite for persistence, and a streaming downloader that fetches a resolved audio URL via `reqwest`. URL resolution (YouTube, etc.) is performed in the frontend by `youtube.js` (`ui/js/youtube.js`), so no `yt-dlp` / `ffmpeg` / `rusty_ytdl` sidecars are required.

**Current State: Active Development — v2.5.18 shipped** — Core architecture is in place and most features are implemented. Background playback is **wired end-to-end** (foreground `MediaPlaybackService` + MediaSession on Android, notification/lockscreen controls routed back into Rust via JNI; see `infrastructure/media/background_service.rs` + `scripts/android/MediaPlaybackService.kt`). YouTube resolver is PO-token aware for all clients (2026) and downloads dual-save to visible `Download/Auralis/` via MediaStore + internal `app_data_dir/downloads` (v2.5.11). Player is queue-aware with `set_queue` + hydration + fallback Next/Prev (v2.5.12) and navigation is free of `viewTransition` races (v2.5.16) / precise `activeView` guard (v2.5.17) / Download form `preventDefault` + `htmx:restored/pageshow` rebind (v2.5.18 fixes `Download→Home` redirect at `00:40.5`). Remaining work is polish + partial smart-playlist presets; macOS/Windows signing remain CI/cert gaps. For verified 2026 platform-compliance (16 KB alignment ✅ enforced via `zipalign -P 16` + `llvm-readelf p_align 0x4000`, targetSdk 36 ✅, background media service ✅ with activity-dead limitation), see `PROJECT.md` §11.

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
│   ├── media/        # AudioPlayer (rodio) + Downloader (reqwest streaming of resolved URLs)
│   └── network.rs    # libp2p: mDNS, gossipsub, request-response, Noise transport
├── commands/         # Tauri command handlers — bridge frontend ↔ services
├── templates/        # Partial server — reads ui/partials/ and caches them
└── lib.rs            # App builder + command registration
```

**Dependencies flow inward**: `commands` → `domain` + `infrastructure`. The `domain` layer depends on nothing external. `infrastructure` depends on `domain` + third-party crates. `commands` depend on all layers.

### Frontend (Soft Glass Audio)

The frontend lives in `ui/` and uses **HTMX 1.9** for SPA-like navigation (bundled locally, no CDN at runtime):

```
ui/
├── index.html          # App shell (sidebar + content + player bar + mobile nav)
├── styles/
│   ├── tokens.css      # Design variables (--glass-*, --neu-*, --blur-*, --radius-*)
│   ├── base.css        # CSS reset + app-shell grid layout
│   ├── components.css  # .glass, .glass-weak, .glass-strong, .neu, .neu-inset, .neu-glass, .card, .track-row
│   └── responsive.css  # Mobile/tablet/desktop breakpoints + safe-area insets (notches/bars)
├── js/
│   ├── bridge.js       # Module entry — composes js/modules/* onto Bridge.prototype, exposes window.Auralis.bridge
│   ├── modules/        # ES modules: core, library, views, scan-ui, player, downloads, ui (bridge methods) — views delegates `data-role="play-card"/"play-row"` `click+touchend` + `touch-action:manipulation`; player does `set_queue(track_ids,current_id)` before `play` + `hydrateState` + `#content`-scoped `_syncPlayerBar`
│   ├── player.js       # PlayerController: progress bar, seeking, MediaSession API, hardware keys, keyboard shortcuts — `play()` is async: `currentTrack? resume : get_queue → playTrack : get_tracks limit1 → playTrack` (v2.5.10 fresh-start fix); `next()/previous()` async library fallback `get_tracks limit200` `(idx+1)%len` (v2.5.12 queue fix); `wireFullScreenElements` re-wires + `hydrateState` on `MutationObserver`/`htmx:afterSwap`/player-bar click (v2.5.12 modal hydration)
│   └── youtube.js      # YouTubeResolver: vendored youtubei.js wrapper (getInfo/search/getPlaylist → resolved objects) — PO-token mint for all clients before `actions.execute`, unconditional `&pot=`, `UA/Referer/Origin` per `winningClient`, SABR legacy `formats[18]` fallback, `effectiveOrderedClients` + `retryClients` for 403 rotation
├── vendor/             # Locally bundled third-party assets (htmx, lucide, youtubei.js esm + node shims)

**Playback events**: Rust emits `playback:state_changed` / `playback:track_changed` / `playback:queue_updated` / `playback:progress`; `js/modules/core.js` re-emits them to the frontend as `playback:state` / `playback:track` / `playback:queue` / `playback:progress`. The progress bar is **event-driven** (no fake timer) — `PlayerController` snaps optimistically on seek and is corrected by the 250ms progress events.
├── partials/           # HTMX fragments served by the Rust backend
│   ├── nav.html, home.html, library.html, albums.html
│   ├── artists.html, playlists.html, player-full.html
│   ├── download.html, search.html, sync.html, settings.html
└── icons/              # auralis.svg
```

**Design language**: Glassmorphism (`.glass`, `.glass-weak`, `.glass-strong` with `backdrop-filter: blur()`) + Neumorphism (`.neu`, `.neu-inset`, `.neu-glass` with dual box-shadows).

**How navigation works**: `index.html` loads `#content` via `hx-get="/partials/home"` on page load. Sidebar links use `hx-get="/partials/<view>" hx-target="#content" hx-swap="innerHTML"` (no `transition:true` — `document.startViewTransition` races caused superimposed `Download + Settings` `00:18` + spontaneous `Welcome` jumps `00:13`, removed v2.5.16 `nav.html:9-44` + `home.html` + `views.js:199`). Active pane is tracked by `views.js activeView` + `#content`-scoped guard `content.querySelector('.page-downloads, .page-settings, …')` (v2.5.16 strict → v2.5.17 precise: allows initial empty `#content` but aborts when `#content` already shows a different page, fixes `Download→Home` redirect) / Download form `preventDefault` + `htmx:restored/pageshow` rebind (v2.5.18).

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
| Audio player (`infrastructure/media/player.rs` + `commands/playback.rs:505 set_queue`) | ✅ rodio (0.22) with queue, shuffle, repeat, seek via rodio's native `try_seek`; real position tracked in `AudioPlayer`; **auto-advance watcher** (`spawn_playback_watcher` in `commands/playback.rs`) advances the queue on track end and emits `playback:progress` every 250ms; queue is seeded via `set_queue(track_ids,current_id)` (v2.5.12) so `Next/Prev` works without a prior in-memory queue + `hydrateState` keeps bar + fullscreen + locked-screen in sync; **download output** is `app_data_dir/downloads/<sanitized title>.<ext>` (`src/lib.rs:322` `Downloader::new(download_dir)` + `downloader.rs:192` `sanitize_filename` + dedup 8-char UUID) with `*.jpg` sidecar — scanned on Android via `AndroidScanner::scan_sandboxed_dir` (`app_data_dir/music` + `downloads`), on desktop via `DesktopScanner` (`dirs::audio_dir`/`download_dir` + `app_data_dir/music`/`downloads`) |
| Background playback (`infrastructure/media/background_service.rs` + `scripts/android/MediaPlaybackService.kt`) | ✅ Android: JNI-driven foreground service (notification + MediaSession). Rust pushes track metadata/state on every playback change; notification/lockscreen buttons route back through `Java_com_auralis_v2_NativeBridge_command` into the same commands as the UI, so the frontend stays in sync via `playback:*` events. No-op on desktop |
| Playback commands (`commands/playback.rs`) | ✅ All commands wired to AudioPlayer |

### Phase 3: Downloads — ✅ COMPLETE

| Task | Status |
|------|--------|
| Download pipeline (`infrastructure/media/downloader.rs` + `android_downloads.rs publish_to_downloads`) | ✅ `reqwest` streaming of a resolved audio URL, HTTP-Range pause/resume + cancel, progress tracking — saves to `app_data_dir/downloads/` (sanitized `title.ext` + UUID dedup, see `Downloader`); `commands/downloads.rs` injects `Referer`/`Origin` + client-matched `UA`; when `Settings.use_system_downloads` (default `true`) also publishes copy to `Download/Auralis/` via `MediaStore` `IS_PENDING` (API 29+) / legacy `Environment.getExternalStoragePublicDirectory` + `MediaScanner` (26-28), `WARN-only` verification, `player.rs cached_copy_for_path` fallback (v2.5.11) |
| Download commands (`commands/downloads.rs`) | ✅ Frontend `youtube.js` resolves the URL; Rust streams bytes + emits `download:progress`/`download:completed` |

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

### 4.1 Dependency Cleanup (`Cargo.toml`) — ✅ DONE (refreshed 2026-08-25)

The dependency set was audited and upgraded in Aug 2026. Patch refresh 2026-08-25: `futures 0.3.33→0.3.34` (`cargo update -p futures --precise 0.3.34`), `async-trait 0.1.91→0.1.92`, `uuid 1.24.0→1.25.0`, `thiserror 2.0.19→2.0.20` — `cargo check --lib` OK. Current key entries:
```toml
tokio = { version = "1", default-features = false, features = ["rt-multi-thread", "macros", "sync", "fs", "io-util", "time"] } # lock 1.53.1 (latest)
rusqlite = { version = "0.40", features = ["bundled"] }   # chrono/uuid features removed — datetimes & UUIDs are stored as TEXT; lock 0.40.2
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls-webpki-roots", "stream", "gzip", "brotli", "deflate"] } # see deferred-upgrades note below; lock has 0.12.28 + 0.13.4 (0.13 via transitive dep)
rodio = { version = "0.22.2", default-features = false, features = ["playback", "mp3", "mp4", "flac", "vorbis", "wav"] }  # vorbis = ogg/vorbis only; no opus feature — webm/opus downloads will DecodeError, prefer m4a 140 over webm/opus in youtube.js or add "opus"
lofty = "0.25"  # lock 0.25.1
libp2p = { version = "0.56", features = ["tcp", "mdns", "noise", "yamux", "gossipsub", "request-response", "tokio", "macros", "json"] } # lock 0.56.0
image = { version = "0.25", default-features = false, features = ["png", "jpeg", "ico"] }  # only PngEncoder is used (QR); jpeg/ico decoders look trimmable; lock 0.25.10
rand = "0.10"          # lock 0.10.2; rand::prelude::*; rand::rng(); rng.random_range(...)
thiserror = "2"        # lock 2.0.20
toml = "1"             # lock 1.1.4+spec-1.1.0
dirs = "6"             # lock 6.0.0
base64 = "0.23"        # lock 0.23.1
futures = "0.3"        # lock 0.3.34
async-trait = "0.1"    # lock 0.1.92
uuid = { version = "1", features = ["v4", "serde"] } # lock 1.25.0
```

Notes from the audit/upgrade pass:
- `tauri-plugin-shell` was registered in `lib.rs` but never used anywhere (no Rust API calls, no frontend refs) — candidate for removal.
- **Deferred upgrades**: `reqwest` 0.12 → 0.13 (rustls becomes default with aws-lc provider; webpki-roots feature removed; would need Android TLS re-validation and may complicate NDK CI) and `jni` 0.21 → 0.22 (breaking API in executors/local-frame handling; touches the hand-rolled JNI bridge). Revisit deliberately.
- MSRV: `rust-version` in Cargo.toml must stay ≥ lofty's MSRV (**1.89**).
- Every libp2p feature declared is used in `infrastructure/network.rs`; rodio codec features match `AudioFormat` exactly.

### 4.2 tauri.conf.json Cleanup — ✅ DONE

- `bundle.targets` is `["deb", "app", "dmg", "msi", "nsis"]` (no `"all"`).
- `identifier` is `com.auralis.v2` (was `com.auralis.app`).
- `version` is `2.6.20` and must stay in sync with `Cargo.toml` + `Cargo.lock` (`package.json` too).
- CSP is `default-src 'self' tauri: data: blob: ipc: http://ipc.localhost; img-src 'self' data: blob: asset: https://i.ytimg.com https://*.ytimg.com; media-src 'self' data: blob: asset: ipc: http://ipc.localhost; style-src 'self' 'unsafe-inline'; script-src 'self' 'unsafe-inline' 'unsafe-eval'; connect-src 'self' ipc: http://ipc.localhost https://*.googlevideo.com https://*.ytimg.com https://i.ytimg.com https://www.youtube.com https://youtubei.googleapis.com https://*.youtube.com https://jnn-pa.googleapis.com https://www.google.com https://*.google.com; font-src 'self' data: https:;` — all third-party JS vendored under `ui/vendor/` (no CDN), `https:` kept for `youtubei`/`googlevideo`/`jnn-pa` `connect-src` (see `scripts/tests/youtube_resolver.test.js`). `unsafe-eval` is required for `youtube.js` `new Function` decipher (BotGuard) — noted as intentional.

### 4.3 Android CI Optimization (`.github/workflows/build.yml`) — ✅ DONE (2026-08-31, v2.6.20)

- APKs are built for **both 64-bit and 32-bit ABIs (`aarch64`, `armv7`, `x86_64`, `i686`) via `--split-per-abi`** (`cargo tauri android build --apk --target aarch64 armv7 x86_64 i686 --split-per-abi`, producing `auralis-v2.6.20-android-arm64.apk`, `-armv7.apk`, `-x86_64.apk`, `-x86.apk`); `x86_64` powers emulator E2E `pixel_6 api33 google_apis`.
- `cargo tauri android init` is guarded with `|| true` before build — harmless idempotent.
- NDK is pinned to **`27.2.12479018` (r27)** — 16KB-page-size capable; `compileSdk`/`targetSdk` sed'd to **36** in `build.gradle.kts`; `tauri-cli` pinned to **`2.11.4`** (via `npm install -g @tauri-apps/cli@2.11.4` + `~/.cargo` cache).
- `libc++_shared.so` is bundled for all ABIs (`arm64-v8a`, `armeabi-v7a`, `x86_64`, `x86`) via `.cargo/config.toml` (`-lc++_shared` per target) and copied into `jniLibs` during CI.
- Android permissions (`READ_MEDIA_AUDIO`, `READ/WRITE_EXTERNAL_STORAGE` maxSdk 32/29, `FOREGROUND_SERVICE`, `FOREGROUND_SERVICE_MEDIA_PLAYBACK`, `WAKE_LOCK`) plus **real `MediaPlaybackService`** (`scripts/android/MediaPlaybackService.kt`) and its `<service android:foregroundServiceType="mediaPlayback">` are injected at build time. Rust drives the service over JNI (`background_service.rs`); media buttons route back via `NativeBridge` → playback commands.
- YouTube resolver is **PO-token-aware (2026)**: `ui/js/youtube.js` mints `po_token` for **all clients** (`TV`/`ANDROID_VR`/`MWEB` included) via `po_token.js:86 generatePoTokenForVideo` `WebPoMinter` `6h visitorData-bound` `nativeFetchPo jnn-pa` `buildURL/getHeaders` protobuf, attaches `&pot=` unconditionally (`vendor youtubei.esm.mjs pot` guard removed), prefers `TV`/`ANDROID_VR` when token missing else `IOS`/`ANDROID` with `effectiveOrderedClients`/`retryClients` `exclude/force` for 403 rotation; `downloader.rs` injects `Referer`/`Origin` + client-matched `User-Agent` to avoid `googlevideo` `403` (`rr1---sn-gwpa-cived` Jio 2026 gates `TV` too). `downloads.js:30 _handle403AutoRetry` auto-retries `403` once `TV→ANDROID+pot→WEB_SAFARI` via `forceClient`/`excludeClient`.
- CI **enforces 16KB alignment**: `zipalign -c -P 16` + `llvm-readelf p_align==0x4000` on 64-bit `.so` libraries (`arm64-v8a`, `x86_64`), with standard 4KB alignment for 32-bit (`armeabi-v7a`, `x86`) — misaligned 64-bit build fails CI. `sccache` + `shared-key` + NDK cache enabled (~11m per release).
- E2E note: `test-android-e2e` verifies **player-working** via `e2e_player_test.js` — seeds `AudioTrack` from `/sdcard/{Music,Download}` (`SIDELoad` → `scan_library_paths` → `get_tracks` → `play`/`set_queue`/`Next`/`Prev` + DOM `data-role` tap), no YouTube network dependency. Desktop `desktop_download_player_e2e.js` seeds 2 tracks + verifies `set_queue` queue length `2` + modal hydration.

**Verify**: `gh release view v2.6.20` shows `arm64`, `armv7`, `x86_64`, and `x86` APKs + desktop artifacts.

### 4.5 Downloads — where files live (v2.5.18, dual-save since v2.5.11)
- **Saved to:** `app_data_dir/downloads/<sanitized title>.<ext>` — `src/lib.rs:322` `download_dir = app_data_dir.join("downloads")` → `Downloader::new(download_dir)`. `downloader.rs:192 sanitize_filename` strips path separators/control chars/`..` + `ALLOWED_EXTS` check, appends 8-char UUID suffix on collision, saves thumbnail sidecar `<audio>.jpg`. **Android dual-save (v2.5.11, still v2.5.18):** when `Settings.downloads.use_system_downloads` (default `true`, `settings.rs` + `settings.html` toggle) is on, `downloader.rs` also publishes a copy to `Download/Auralis/<name>` via `infrastructure/media/android_downloads.rs` `publish_to_downloads` (`MediaStore.Downloads` `IS_PENDING` on API 29+, legacy `Environment.getExternalStoragePublicDirectory` + `MediaScanner` on 26-28, non-fatal fallback keeps internal path; `player.rs` `cached_copy_for_path` reads public copy if internal missing).
- **Android path:** `app_data_dir` is Tauri internal storage (`/data/data/com.auralis.v2/` → `files/downloads/`), **not** `Music/` pillar — scanned via `AndroidScanner::scan_sandboxed_dir` (`app_data_dir/music` + `downloads`) triggered by `scan_library_paths` after `download:completed`. **Public copy** at `/storage/emulated/0/Download/Auralis/` is Files-visible (like a browser download); `library.rs` scan stays sandboxed (`default_paths` `app_data_dir` only) to avoid duplicate DB entries (internal + public) — `test-android-e2e` verifies public copy separately via `ls`/`content query` + MediaStore check. Not visible in `DocumentsUI > Android/data` without `All files access` (fuse `Permission denied` on `HyperOS` scoped storage — use `Files → All files access` or `library:scan_log` toast `1 added`).
- **Desktop path:** `DesktopScanner::scan_library_paths_with_progress` scans `dirs::audio_dir` + `dirs::download_dir` + `app_data_dir/music` + `app_data_dir/downloads`.
- **`Settings.download_path` (`settings.rs:105 dirs::audio_dir()`) is legacy default UI hint, not the actual save dir — downloader ignores it.**
- **Import bypass:** `commands/library.rs:320 import_audio_file` writes `app_data_dir/music/<name>` via `AndroidScanner::ingest_buffer` for Android 14/16 Scoped Storage base64 path.

### 4.4 Linker Optimization — ⚠️ PARTIAL

`[profile.release]` already sets `codegen-units = 1`, `opt-level = "z"`, `lto = "fat"`, `strip = true`, `panic = "abort"`.

`lld`/`mold` is **not** wired in `.cargo/config.toml` (the host `aarch64-unknown-linux-gnu` target only adds `-lc`). To speed up local host linking, add:
```toml
[target.x86_64-unknown-linux-gnu]
rustflags = ["-C", "link-arg=-fuse-ld=lld"]
```
(Note: this dev machine is **Void Linux (aarch64) under proot in Termux** — `cargo check --lib` works, but linking fails: `cargo build` hits the missing `webkit2gtk-4.1`, and test binaries fail with a `__stack_chk_guard` DSO error from ring (proot loader layout). Use CI for builds/tests; locally only `cargo check` is practical.)

---

## 5. Testing

### Running Tests

```bash
bash scripts/test.sh
# or
cargo test --all-features          # also valid: cargo check --all-targets / cargo clippy --all-targets --all-features
node --test scripts/tests/youtube_resolver.test.js  # 27 tests
node scripts/tests/desktop_real_e2e.js              # real release binary IPC e2e
xvfb-run node scripts/tests/desktop_download_player_e2e.js  # player-seed E2E: import_audio_file tinyWAV → scan → set_queue → play/Next/Prev + modal hydration
# Android emulator (needs KVM + x86_64 APK):
bash scripts/android/run_emulator_test.sh            # drives scripts/android/e2e_player_test.js over CDP 9222 (seeds /sdcard/Music → scan → play, WARN-only MediaStore)
```

> **Local (proot/Termux) caveat**: on this dev machine `cargo test` cannot run — the test binary fails to link (`__stack_chk_guard` DSO error from ring, proot loader layout). Run tests via CI; locally stick to `cargo check --all-targets` + `cargo clippy --all-targets --all-features -- -D warnings` + `cargo fmt --check` + `node --check` / `node --test`.

### Test Coverage

Unit tests exist across domain models (`PairingInfo::generate`, `SyncChange::new`, `PairedDevice::mark_synced`, `track`, `album`, `artist`, `download`, `playlist`, `settings`), infrastructure (`scanner`, `network`), and templates. Real-binary IPC end-to-end verification is handled by `scripts/tests/desktop_real_e2e.js` + `scripts/tests/desktop_download_player_e2e.js`. Android E2E is `scripts/android/e2e_player_test.js` (renamed `R 100%` from `e2e_download_test.js` v2.5.13, supports both names via `run_emulator_test.sh:131`) — asserts `seedSdcardCopyHostSide` via `adb shell cp/run-as/cat` → `stat>10KB` → `scan_library_paths` → `get_tracks` → `set_queue`+`play`/`next`/`previous` (library fallback) → DOM delegated tap → `MediaStore WARN-only` (`content query … is_pending 0` + `ls Download/Auralis`).

### Test Conventions

- Use `:memory:` SQLite for repository tests.
- Place unit tests in each `src/` module with `#[cfg(test)] mod tests`.
- End-to-end IPC testing is conducted against the real release binary via WebDriver.

---

## 6. Code Style

- **Rust**: Run `cargo fmt` before committing. `cargo clippy --all-targets` must pass with no warnings.
- **HTML/Templates**: Use 2-space indentation. HTMX attributes prefixed with `hx-`.
- **CSS**: Vanilla CSS, no preprocessors. Use CSS variables for theming.
- **Commits**: Follow conventional commits (`feat:`, `fix:`, `chore:`).

---

## 7. CI/CD Pipeline

The CI workflow (`.github/workflows/build.yml`, `tauri-cli 2.11.4`, `NDK 27.2.12479018`, `compileSdk/targetSdk 36`) runs on `push` to `main` + tags `v*`, `pull_request` to `main`, `workflow_dispatch`, and `schedule: 0 3 * * *` (`concurrency: cancel-in-progress`):

| Job | Purpose | When |
|-----|---------|------|
| `lint` | `cargo fmt --check` + `cargo clippy --all-targets --all-features -D warnings` + `cargo audit` | every run |
| `build-linux` | `cargo build --release` + `cargo test --all-features` + `node --test scripts/tests/youtube_resolver.test.js` (27 tests) + `xvfb` `desktop_real_e2e.js` + `desktop_download_player_e2e.js` (2-track `set_queue`+`Next`/`Prev`+modal checks) | every run |
| `build-macos` | Compiles + bundles `.dmg` (x86_64 + aarch64) | tag only (`if: startsWith(github.ref, 'refs/tags/v')`) |
| `build-windows` | Compiles MSVC target, bundles `.msi` / `.exe` | tag only |
| `build-android` | Builds signed `aarch64` + `x86_64` APKs `--split-per-abi` (auto-generates keystore, sets targetSdk 36, injects permissions + `MediaPlaybackService.kt`, bundles `libc++_shared.so` per ABI, **verifies 16KB alignment** via `zipalign -P 16` + `llvm-readelf p_align==0x4000`) | tag only |
| `test-android-e2e` | `reactivecircus/android-emulator-runner@v2` `pixel_6 api33 google_apis x86_64 KVM` + `run_emulator_test.sh` + `scripts/android/e2e_player_test.js` (was `e2e_download_test.js` → `R 100%` v2.5.13) — seeds `sdcard` `AudioTrack`Copy → `scan` → `set_queue` queue `2` → `play`/`Next`/`Prev` + DOM `play-row` `click+touchend` + `MediaStore WARN-only` check | every run (needs `build-linux`) |
| `ci` (status) | Aggregates `needs: [lint, build-linux, build-android, test-android-e2e]` for branch protection | every run |
| `release` | Creates GitHub Release from all `release-*` artifacts (`softprops/action-gh-release`) | tag only (`needs: [build-linux, build-macos, build-windows, build-android, lint]`) |

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
| **App setup** | `src/lib.rs` (also registers `set_queue` command, `download_dir = app_data_dir/downloads` :322) |
| **Command registration** | `src/lib.rs` (invoke_handler! includes `set_queue`, `get_queue`, `play`, `pause`, `next`, `previous`) |
| **Database schema** | `src/infrastructure/database/repositories.rs` |
| **Track model** | `src/domain/models/track.rs` |
| **Sync model** | `src/domain/models/sync.rs` |
| **Network implementation** | `src/infrastructure/network.rs` |
| **Playback queue** | `src/commands/playback.rs:505 set_queue` + `src/infrastructure/media/player.rs` + `ui/js/player.js` + `ui/js/modules/player.js` |
| **Downloads / MediaStore** | `src/infrastructure/media/downloader.rs` + `src/infrastructure/media/android_downloads.rs` + `src/commands/downloads.rs` + `ui/js/modules/downloads.js` + `verify_downloads_mediastore.sh` |
| **Navigation / views** | `ui/index.html` + `ui/partials/nav.html` + `ui/partials/home.html` + `ui/js/modules/views.js` (activeView + `#content` guard, hx-swap `innerHTML` no transition) |
| **Android CI / Release** | `.github/workflows/build.yml` (jobs `lint` + `build-linux` + `build-macos/windows` tag-only + `build-android --split-per-abi` + `test-android-e2e` + `ci` + `release`; triggers `push main/tags v*` + `PR main` + `workflow_dispatch` + `schedule`) |
| **Android E2E / Device** | `scripts/android/e2e_player_test.js` + `scripts/android/run_emulator_test.sh` + `scripts/android/MediaPlaybackService.kt` + `scripts/android/verify_downloads_mediastore.sh` |
| **Desktop E2E IPC** | `scripts/tests/desktop_real_e2e.js` + `scripts/tests/desktop_download_player_e2e.js` + `scripts/tests/youtube_resolver.test.js` |
| **Build config** | `Cargo.toml`, `Cargo.lock`, `tauri.conf.json`, `package.json`, `.cargo/config.toml` (also `gen/android/app/build.gradle.kts` compileSdk 36) |
