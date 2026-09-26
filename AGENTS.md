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
rodio = { version = "0.22.2", default-features = false, features = ["playback", "mp3", "mp4", "symphonia-aac", "symphonia-alac", "symphonia-mkv", "symphonia-ogg", "flac", "vorbis", "wav"] }
rusty-opus = "0.9"     # 64-bit targets (aarch64, x86_64): pure-Rust Opus codec with AVX2 & ARM64 NEON SIMD kernels
opus-rs = "0.1"        # 32-bit targets (armv7, i686): pure-Rust Opus codec for compatibility and battery efficiency
symphonia = { version = "0.5.5", default-features = false, features = ["mkv", "ogg", "isomp4"] }
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
- `version` is `2.6.26` and must stay in sync with `Cargo.toml` + `Cargo.lock` (`package.json` too).
- CSP is `default-src 'self' tauri: data: blob: ipc: http://ipc.localhost; img-src 'self' data: blob: asset: https://i.ytimg.com https://*.ytimg.com; media-src 'self' data: blob: asset: ipc: http://ipc.localhost; style-src 'self' 'unsafe-inline'; script-src 'self' 'unsafe-inline' 'unsafe-eval'; connect-src 'self' ipc: http://ipc.localhost https://*.googlevideo.com https://*.ytimg.com https://i.ytimg.com https://www.youtube.com https://youtubei.googleapis.com https://*.youtube.com https://jnn-pa.googleapis.com https://www.google.com https://*.google.com; font-src 'self' data: https:;` — all third-party JS vendored under `ui/vendor/` (no CDN), `https:` kept for `youtubei`/`googlevideo`/`jnn-pa` `connect-src` (see `scripts/tests/youtube_resolver.test.js`). `unsafe-eval` is required for `youtube.js` `new Function` decipher (BotGuard) — noted as intentional.

### 4.3 Android CI Optimization (`.github/workflows/build.yml`) — ✅ DONE (2026-09-03, v2.6.26)

- APKs are built for **both 64-bit and 32-bit ABIs (`aarch64`, armv7, x86_64, i686) via `--split-per-abi`** (`cargo tauri android build --apk --target aarch64 armv7 x86_64 i686 --split-per-abi`, producing `auralis-v2.6.26-android-arm64.apk`, `-armv7.apk`, `-x86_64.apk`, `-x86.apk`); `x86_64` powers emulator E2E `pixel_6 api33 google_apis`.
- `cargo tauri android init` is guarded with `|| true` before build — harmless idempotent.
- NDK is pinned to **`27.2.12479018` (r27)** — 16KB-page-size capable; `compileSdk`/`targetSdk` sed'd to **36** in `build.gradle.kts`; `tauri-cli` pinned to **`2.11.4`** (via `npm install -g @tauri-apps/cli@2.11.4` + `~/.cargo` cache).
- `libc++_shared.so` is bundled for all ABIs (`arm64-v8a`, `armeabi-v7a`, `x86_64`, `x86`) via `.cargo/config.toml` (`-lc++_shared` per target) and copied into `jniLibs` during CI.
- Android permissions (`READ_MEDIA_AUDIO`, `READ/WRITE_EXTERNAL_STORAGE` maxSdk 32/29, `FOREGROUND_SERVICE`, `FOREGROUND_SERVICE_MEDIA_PLAYBACK`, `WAKE_LOCK`) plus **real `MediaPlaybackService`** (`scripts/android/MediaPlaybackService.kt`) and its `<service android:foregroundServiceType="mediaPlayback">` are injected at build time. Rust drives the service over JNI (`background_service.rs`); media buttons route back via `NativeBridge` → playback commands.
- YouTube resolver is **PO-token-aware (2026)**: `ui/js/youtube.js` mints `po_token` for **all clients** (`TV`/`ANDROID_VR`/`MWEB` included) via `po_token.js:86 generatePoTokenForVideo` `WebPoMinter` `6h visitorData-bound` `nativeFetchPo jnn-pa` `buildURL/getHeaders` protobuf, attaches `&pot=` unconditionally (`vendor youtubei.esm.mjs pot` guard removed), prefers `TV`/`ANDROID_VR` when token missing else `IOS`/`ANDROID` with `effectiveOrderedClients`/`retryClients` `exclude/force` for 403 rotation; `downloader.rs` injects `Referer`/`Origin` + client-matched `User-Agent` to avoid `googlevideo` `403` (`rr1---sn-gwpa-cived` Jio 2026 gates `TV` too). `downloads.js:30 _handle403AutoRetry` auto-retries `403` once `TV→ANDROID+pot→WEB_SAFARI` via `forceClient`/`excludeClient`.
- CI **enforces 16KB alignment**: `zipalign -c -P 16` + `llvm-readelf p_align==0x4000` on 64-bit `.so` libraries (`arm64-v8a`, `x86_64`), with standard 4KB alignment for 32-bit (`armeabi-v7a`, `x86`) — misaligned 64-bit build fails CI. `sccache` + `shared-key` + NDK cache enabled (~11m per release).
- E2E note: `test-android-e2e` verifies **player-working** via `e2e_player_test.js` — seeds `AudioTrack` from `/sdcard/{Music,Download}` (`SIDELoad` → `scan_library_paths` → `get_tracks` → `play`/`set_queue`/`Next`/`Prev` + DOM `data-role` tap), no YouTube network dependency. Desktop `desktop_download_player_e2e.js` seeds 2 tracks + verifies `set_queue` queue length `2` + modal hydration.

**Verify**: `gh release view v2.6.26` shows `arm64`, `armv7`, `x86_64`, and `x86` APKs + desktop artifacts.

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

### 4.6 YouTube resolver — client strategy + per-client diagnostics (v2.6.43)

**Truncated-download root cause (Sept 2026).** A transfer can finish at 100 % of the advertised bytes and still hold only part of the audio. This is *not* a dropped connection and resume cannot help. Evidence gathered Sept 2026:

| Source | Finding |
|---|---|
| yt-dlp #12551, #12218 | "incomplete download of audio (no error indicated)" — 100 % bytes, audio stops mid-file |
| LuanRT/GoogleVideo #52 | The ~60 s SABR cutoff is a *client library* limit, not a YouTube limit — SABR is a stateful sequential protocol |
| cobalt discussion #1374 | *"no exact plan on how to handle SABR… we're using youtube clients that don't have it enforced, but we have no clue for how long this will last"* |
| yt-dlp #16150 / #17348 | ⚠️ **CONTRADICTED BY OUR OWN MEASUREMENT (v2.6.51)** — reports `android_vr` returning only muxed `itag 18`. A device report (v2.6.50, residential Jio) had `ANDROID_VR` returning **22 adaptive / 4 audio with urls at itag 140, audio-only**, while `ANDROID` was the one returning nothing. This citation is what demoted `ANDROID_VR` to last resort for two versions. Kept visible rather than deleted: it is the evidence that the belief existed, and the belief looked cited enough to survive review. Do not re-derive ordering from it. |
| yt-dlp PO-Token Guide | `web` is **SABR-only**; a PO token is **platform-bound** (a Web/BotGuard token is invalid on `android`/`ios`) |

**Client order** (`ui/js/youtube.js`, **v2.6.51** — the v2.6.43 order below is history, do not restore it). Derived from three named groups rather than a hand-written array, because the order *is* the rule:

| group | clients | why |
|---|---|---|
| `SERVABLE_CLIENTS` | `MWEB, WEB, WEB_SAFARI` | the only clients a **Web/BotGuard** token is valid on (PO-Token Guide: `mweb`/`web`/`web_safari` need GVS) |
| `TOKEN_FREE_CLIENTS` | `ANDROID_VR, TV` | need **no** token. `ANDROID_VR` leads `TV` because `tv`'s cell reads *"All formats DRM'd if cookies (logged-in or active guest) aren't passed"* and we send none, while `android_vr` has no such caveat |
| `UNMINTABLE_CLIENTS` | `IOS, ANDROID` | need DroidGuard/iOSGuard, which we cannot mint. Kept, not deleted — `ios` **does** resolve with real audio urls; the requirement is on GVS, not format discovery |

With a token, `SERVABLE` leads; without one, `TOKEN_FREE` leads since nothing in `SERVABLE` can be served. **`web_safari` is web-family and was never tried** — the one web client available to us that the Jio edge had not already refused to resolve. It is an experiment: the client report is the evidence, not this table.

⚠️ **The token-free set is shrinking.** Enforcement is mid-rollout and every other client now requires a token. `bgutils-js` buys us the web family, which is the family a residential Jio line refused to *resolve* at all — so the real fallback is `ANDROID_VR`/`TV`, and their DRM caveats are load-bearing rather than incidental.

Muxed `itag 18` is **not** refused outright: `avoidLegacyProgressive` is scoped to the client that actually truncated, because the short stream is a SABR *window* — a property of the response, not of the container — so the same `itag` from a different client can be complete.

**Per-client reaction report** (v2.6.43). Release builds write nothing to logcat, so the resolver records one entry per attempted client in `clientReport` (`ui/js/youtube.js`): `phase` (`actions.execute` / `getInfo`), `status` (playability), `adaptive` / `progressive` counts, `adaptiveWithUrl`, `audioWithUrl`, `sabrStreamingUrl`, `reason` (`sabr-only`, `adaptive-urls-missing`, `legacy-progressive`, `no-usable-audio`, `error`, …), `ms`, and `chosen`. It is exposed on the resolved track as `client_report` / `client_report_text` / `selection`, archived in `window.__auralisClientReports` (last 20) by `downloads.js` `_recordClientReport`, rendered as a `clients: …` line in the download row, and copyable via the **Copy report** button. Resolve failures carry `err.client_report`, so a report exists even when nothing was downloaded.

**What the first real report showed (v2.6.43, track `BElct8HWkp8`, residential Jio).** `MWEB`, `TV` and `WEB` answered `UNPLAYABLE` with zero formats; `ANDROID` returned 25 adaptive formats with **no urls** and `serverAbrStreamingUrl` set (SABR-only, so the resolver fell back to muxed `itag 18`, which the CDN then 403'd at byte 0); only `IOS` (20 adaptive / 2 audio with urls) and `ANDROID_VR` (22 / 4) produced real audio urls. **Both of those delivered exactly 99 s of a 287 s track with every advertised byte received** — the window is a property of the response, not of the client, so rotating clients cannot fix it. The window is a property of the response, not of the client, which is why rotating clients could not fix it and why the fix moved to the transport layer. **Superseded in v2.6.51:** `ANDROID_VR` is no longer demoted — it and `TV` are `TOKEN_FREE_CLIENTS` and `ANDROID_VR` now leads. See the client-order table above.

**Range top-up (v2.6.44).** When the decoded-length gate fires, `downloader.rs` now asks for the bytes *after* what it holds instead of giving up: `range_topup` (`downloader.rs`, `TOPUP_CHUNK_BYTES` = 2 MiB, `MAX_TOPUP_ROUNDS` = 4) issues `range=start-end` requests — trying the googlevideo `range` query param plus the HTTP `Range` header, then the header alone, then the param alone — appends whatever arrives to the staging file and re-runs `verify_decoded_duration` after each round. Either the file is completed and saved, or the failure message names the exact status codes the edge returned (`416` = the object really does end there, i.e. a hard wall; `403` = gated), plus `bytes received / clen`, `itag`, `host` and `end_reason`. `with_range_param` replaces any `range=` the URL already carried. Helpers: `extract_url_param_str` (for `itag`).

**Data-driven client rotation (v2.6.44).** `downloads.js` reads `resolved.client_report` and only rotates into a client whose own record shows it can serve audio (`status === 'OK'` and `audioWithUrl > 0`). SABR-only (`audioWithUrl: 0`) and `UNPLAYABLE` clients are dead ends — rotating into one just burns a download on a 403 — so when every remaining client is one, the retry stops and the real error is surfaced.

**The truncation verdict was itself wrong (v2.6.45).** v2.6.44's error text settled it: `[received 21379314 bytes of 21379314 advertised (itag=18, end_reason=all-advertised-bytes-received)]` with `url+header: HTTP 416 | header: HTTP 416 | url: HTTP 400`. A `416` means the object really does end there, and 21.4 MB for a 287 s track is 596 kbps — exactly a muxed 360p progressive — while a real 99-second window would be ~7 MB. So the file was **complete** and rodio's `total_duration()` was wrong about it. The lesson: a decoder's opinion is not evidence.

**`src/infrastructure/media/forensics.rs` (v2.6.45)** answers the two questions separately, with no decoding assumptions:
- *Are all the media bytes there?* The MP4 sample table says so exactly: `stco`/`co64` chunk offsets + `stsc` samples-per-chunk + `stsz` sample sizes give the highest byte the file must reach (`Verdict::Complete` / `Truncated { missing_bytes }`); a fragmented file answers the same through its `sidx` (`sidx.end + first_offset + Σ referenced_size`). Anything unparsable yields `Verdict::Unknown` so callers keep their old behaviour. Also reports `table_secs` (what the container claims), `has_video_track`, `fragment_count`.
- *Is there audio in them?* `inspect_content` decodes the whole file and reports `audible_secs` (position of the last sample above ±16) — the signature of a server-side window is a large gap between `audible_secs` and the expected length even when the byte count is complete.

**The gate now trusts the container, not the decoder.** A short decoder verdict is only honoured when the container agrees: the file is kept when `verdict == Complete` **and** `table_secs` covers the track **and** there is audible audio for ~all of it; otherwise the range top-up runs and, if that fails, the error explains *which* problem it is — "the container itself only describes a short track, so the server sent a windowed object and reports it as complete" (rotate clients) versus "the container describes the full track but bytes are missing" (interrupted transfer). `forensics::{summary}` output rides along in the message.

**Race fix (v2.6.45).** `Promise.any` picked whichever client *answered first*, not the best one: on the device `ANDROID` won with a SABR-only response (muxed `itag 18` — exactly the rendition that gets truncated) while `IOS`/`ANDROID_VR` had real audio-only urls ready. A legacy-progressive-only result now waits `LEGACY_RESULT_DELAY_MS` (1200 ms) in both races, so a genuine audio url wins unless nothing else arrives.

**The player was the last thing truncating tracks (v2.6.46).** With v2.6.45 the download completes and the file lands in the library — the screenshot showed `Continue Listening 4:26` next to a player bar reading `1:32` for the same track. `player.rs` had an "auto-repair" step that treated a >5 s disagreement between the library duration and `source.total_duration()` as proof the *library* was wrong and overwrote it with the decoder's value. That is the same unreliable number the completeness gate had been trusting: for these MP4s rodio stops counting at the first fragment it can parse, so a 4:26 track becomes 1:32, the progress bar fills at 1:32 and `seek()` refuses anything past it. Playback itself was never cut (auto-advance uses `sink.empty()`, i.e. EOF), so the audio played out in full behind a bar that said otherwise. `reconcile_duration(db, decoded)` now lets the decoder only ever *raise* a duration: the library value comes from the container's sample table, which is the same thing the download gate verifies, and a decoder that claims less is a decoder that miscounted. Four unit tests cover the four cases.

**Cover art was structurally broken (v2.6.47).** Every library card renders artwork through `assetUrl()` → `convertFileSrc()`, i.e. an `asset://` url, and the CSP has always allowed `asset:` in `img-src` — but `app.security.assetProtocol` was **absent from `tauri.conf.json` entirely**, and Tauri v2 only serves the asset protocol when it is enabled *and* the path is in scope. So every `<img>` failed to load, the `onerror` fallback called `media_data_url`, and if that also failed the browser's broken-image icon stayed in the card with the reason only in a console no release build can reach. `assetProtocol` is now enabled with a scope covering `$APPDATA`, `$APPLOCALDATA`, `~/Music`, `~/Download` and `~/Documents` — which **also requires the `protocol-asset` feature on the `tauri` crate** (`tauri = { features = ["protocol-asset"] }`), otherwise the build script aborts with "the `tauri` dependency features ... does not match the allowlist defined under `tauri.conf.json`" (downloads and their `.jpg` sidecars live under `app_data_dir`), and `ui.js` `_artworkFailed` records the failure in `window.__auralisArtworkFailures` and swaps in the neutral music placeholder so a card never looks broken. Covered by a new test that asserts the protocol is enabled, scoped to the app data dir, and that the CSP still allows `asset:`.

**Nav race + dead resume + untagged downloads (v2.6.48).** Three defects found on-device and fixed together:
- *Nav:* the initial `hx-get="/partials/home.html"` lives **on** `<main id="content">`, so that request is owned by `#content` while a nav click's request is owned by the `<a>`. htmx's request queue is keyed per owning element, so both ran in parallel and the stale home response clobbered the freshly swapped view — "click Download Audio, land on Home, click again and it works". All 15 `#content` writers now carry `hx-sync="#content:replace"` (htmx: "abort the current request, if any, and replace it with this request"). A test walks `ui/**/*.html` per opening tag and fails on any `hx-target="#content"` without it.
- *Resume:* `player.rs::resume()` returned `Ok(())` when there was no sink or the sink was drained — rodio's `Player::play()` is documented "no effect if not paused" and a consumed source leaves `empty()` true forever. `stop()` takes the sink but leaves `current_track` set, so `player.js play()` kept choosing the resume path, `isPlaying` was set to `true` over silence, and **nothing could be played until the app was restarted**. `resume()` now returns `StateError` in both dead states (with no lock guard held across an `await`), the frontend arms a 700 ms proof-of-life watch settled by `playback:state`/`playback:progress` and replays the track if nothing starts, and the Android notification's play button (`background_service.rs`) replays instead of discarding the error.
- *Tags:* downloads were saved untagged, so the scanner fell back to the sanitized filename and `Unknown Artist`. New `src/infrastructure/media/tags.rs` writes title/artist/album into the finished file with lofty (mirroring the already-compiling `filesystem::metadata::write_metadata` call-for-call; MP4 write support is lofty's weakest path, so the call site only logs failures). `StreamDownload`/`DownloadJob` carry `artist`/`album`, `DownloadRequest` gained them with `#[serde(default)]`, and `buildDownloadPayload` sends `resolved.author`. Runs in `spawn_blocking` because lofty's MP4 save rewrites the file.

**Audit round 1 fixes (v2.6.49).** An independent audit agent reviewed the download/player pipeline against v2.6.47 and filed 24 findings in `issue.md` (untracked, agent-owned). Six were accepted and fixed here; the rest are deferred with reasons recorded in `issue.md` and §4.

- **NEW-01 — the range top-up could corrupt a file (the serious one).** `top_up` accepted any 2xx and appended the body without ever reading `Content-Range`, so an edge that ignored `Range` and answered `200` with the object from byte 0 had a second copy of the object's opening appended to the tail. That corruption still decodes, so it passed the decoded-duration check and could be saved as "recovered". Also: a `206` for the wrong window was accepted, nothing bounded the append to the advertised object length, and one chunk could overshoot even the per-call allowance. Now the response is only written when its headers prove it is the requested slice — `200` is rejected unless staging is empty (where "whole object" and "range from 0" are the same bytes), `Content-Range` is parsed and its start must equal the requested offset, and the clamp is applied per chunk, not just as a loop bound. Every header check runs *before* the file is opened, so a rejection cannot leave a byte. **`416` keeps a separate branch and separate wording**: it is an answer (the object ends here) rather than a refusal, and the caller shows that text to distinguish "no resume can fix this" from "the edge ignored us, rotate the client".
- **NEW-04 (half) — the gate demanded *audible* audio for ~90 % of a track,** so a track ending in silence was rejected and burned four top-up rounds. `ContentFacts::measured_secs` = `total_samples / sample_rate`, the length from actually iterating the sample stream, immune both to `total_duration()`'s under-reporting and to trailing silence. A derived method, not a field: `inspect_content` populates the struct field by field, so a stored value would go stale mid-construction. Surfaced via `summary()`, which the downloader calls in production, so `-D warnings` cannot fail on it. **The gate switchover is still owed** (needs `downloader.rs`).
- **PB-01 — `play_track` published state before playing,** so a missing/undecodable file left the UI pointing at a song that never started. Now `start_sink` does all fallible work and writes no identity; `commit_start` publishes only a start rodio has accepted. The incoming library duration is passed in rather than read from `track_duration` (which still describes the *previous* track — reading it there is how a 1:32 file inherits 4:26).
- **NEW-03, DL-05, DL-01, DL-02 — still owed,** all in `downloader.rs`.
- **DL-07 — `Download/Auralis/` was always empty.** The public copy is a MediaStore row inserted with `IS_PENDING=1`, invisible until cleared; eleven paths could leave it pending forever (copy failing at open/read/write/flush/close, the clearing `ContentValues` failing to build, the update throwing), and the update's return value was unchecked so a zero-row clear looked like success. The invariant is now enforced structurally: exactly one statement after the insert resolves the row, no `?`/`return` between. An unclearable row is **deleted**, because an invisible row still reserves its name and the next download collides with a ghost. Two latent JNI bugs fixed too: follow-up JNI calls were made with a Java exception still pending, which the JNI spec forbids and which can abort the VM.
- **Handoffs landed:** `preserve_track_state` stops a rescan from wiping favourites, play history and download provenance; Unicode-safe `sanitize_filename` (the old `String::truncate(200)` **panicked** on a multibyte boundary); `completion_path` so `download:completed` reports the Files-visible path; pause/cancel abort **and await**; state transitions validated with a new `DownloaderError::InvalidState`.

**Two CI-only failure modes worth remembering (v2.6.49).** Both fixes were invisible locally. (1) `f64::From<u64>` does not exist — the conversion would be lossy past 2^53 — so `f64::from(x.unwrap())` on a `u64` needs `as f64`. (2) The local `rustfmt` is **1.63** while CI runs current stable, and they disagree on a method chain inside a `let ... else`: 1.63 splits it, stable joins it. A local `rustfmt --check` passing is therefore **not** proof CI's `cargo fmt --check` will pass. When they conflict, restructure so both agree (bind the chain to a plain local first) instead of picking a side. Read the exact diff out of the CI log rather than guessing.

**A Web PO token was riding non-web clients' media URLs (v2.6.50).** A device download failed `403 Forbidden … start_byte=0, ct=text/plain, body: (empty)` from `rr5---sn-gwpa-cived`. The per-client report made it unambiguous: `MWEB`/`TV`/`WEB` UNPLAYABLE, `ANDROID` SABR-only and correctly skipped, then **`IOS` CHOSEN (`adaptiveWithUrl=20 audioWithUrl=2`)** and **`ANDROID_VR` CHOSEN (22/4)** — real audio URLs twice — then 403 before a single media byte. A resolver that worked and an edge that refused.

A PO token is **platform-bound**: the only token we can mint is a BotGuard/WEB one (`WebPoMinter`), and `youtube.js` documented that itself directly above code that ignored it — the append was `if (streamUrl && (opts.poToken || opts.po_token))`, with no client check, while the headers sent alongside were client-matched (`uaMap[winningClient]`). iOS UA on a URL carrying a Web token. **The rotation is why it looked unfixable**: every candidate got the *same* invalid token, so `ANDROID_VR → IOS` changed the UA and nothing else. It also explains why v2.6.43 got 99 s of a 287 s track instead of a refusal — minting did not reliably succeed then, so `pot` never appeared on the URL.

New `ui/js/modules/pot_scope.js` owns the decision. A token **we** minted (or read from our own cache) is Web-bound → only `MWEB`/`WEB`, and it is **stripped** if the vendored `Player.decipher` already put one on a non-web winner. A token the **user** supplied in Settings passes through untouched — it may legitimately be an iOS/Android/TV token. The InnerTube body `po_token` is unchanged; only the CDN-side `pot=` was wrong. Per the yt-dlp guide's table, `tv`/`android_vr` need no token and `tv_simply` needs one we cannot mint, so all three are excluded — attaching ours to `TV` would be downside with no upside.

**Two lessons recorded, because both were invisible locally.** (1) *A regression test can encode the defect.* `pot-for-TV (YAD 7C4-TAWg7QA)` asserted a minted token **is** attached to a `TV` URL — and never ran shipped code: it defined its own copy of the append logic and asserted against that, so it passed no matter what `youtube.js` did, while a live 403 shipped. It now pins the opposite (a documented fact, not an assumption). The allowlist is `['MWEB','WEB']` plus a test asserting every member appears in `orderedClients`, because an entry the resolver cannot emit is a trap that fails silently the day someone wires that client up. (2) *`staged_bytes` must stay the true on-disk length.* `top_up` appends with `append(true)`, which writes at the real EOF regardless of the `start` it is given, and grants a `200` the "no range needed" exception when `start == 0` — both sound **only** while `start` is the real length. A stat failure is now a hard error rather than `unwrap_or(0)`, which reported a non-empty file as empty and so re-admitted the whole-object append.

**`check-android` CI job (v2.6.50).** `build-android` is tag-only, so `#[cfg(target_os = "android")]` code compiled **zero times** between releases and then broke v2.6.49 three ways. A step inside a tag-only job cannot prevent a tag-only failure, so coverage now runs on every push: `cargo check --target aarch64-linux-android` + `armv7-linux-androideabi`, no APK/bundling/signing, wired into the `ci` aggregate. Two gotchas cost a run each: `cc-rs` (ring) ignores `CARGO_TARGET_*_LINKER` and resolves `<prefix>-clang` on `PATH` with no API suffix, and for 32-bit ARM it asks for `arm-linux-androideabi-clang` — neither Rust's `armv7-linux-androideabi` nor anything the NDK ships unsuffixed, so both are aliased inside the NDK's own `bin`. It was added reporting-only first and promoted only after a green main run, since it cannot be verified on the dev box.

**Audit round 1 review fixes (v2.6.51).** A second read by the audit agent of the code shipped in v2.6.49/v2.6.50 found one HIGH in each, plus a formatting-gate discovery worth more than either.

- **PB-01a (HIGH) — the duration mirror landed on the outgoing track.** `commit_start` re-stamps `queue[current_index]`, but `next` called `play_track` first and set the index *after*. So the mirror hit the track being left, and the track now playing never received its decoder-repaired duration in the queue — on **every** transition, auto-advance included. The guard that only mirrors when the decoder had an opinion was correct and could not help: the stamp *target* was the bug, not the timing. I had written a comment naming the hazard and shipped it anyway. `next`/`previous` now go through one `start_at_index` helper that sets the index first and restores the outgoing value on failure — the shape `commands::playback.rs::play` already used, lifted so the paths cannot drift. The outgoing index is passed in, not re-read, so the rollback cannot pick up a value written during the `await`.
- **DL-07 (HIGH) — JNI local refs grew with file size.** The copy loop made a Java `byte[]` per 64 KiB chunk and `JObject` has **no `Drop`** in `jni` 0.21, so dropping the binding freed nothing: a 100 MB publish pinned ~100 MB of Java heap with ~4800 refs live at once, never released on an already-attached thread. Every allocation is now released with `delete_local_ref` — the one call the spec permits **while an exception is pending**, so the error path is legal too. The `write` result is bound *before* the delete so an early return cannot skip it, and the chunk is **moved** into the delete so any later use is a compile error rather than a use-after-free.
- **DL-07 (MEDIUM) — the exception-drain helper itself broke the rule it enforced.** It did `exception_occurred` → `toString` → `get_string` → *then* `exception_clear`, and `CallObjectMethod`/`GetStringUTFChars` are not in the spec's permitted-while-pending list, so CheckJNI aborts the process and release is UB. The file's central safety mechanism was the one place the rule broke. Order is now describe → occurred → **clear** → only then `toString`; if the clear itself fails it returns immediately, because the only safe remaining action is none.
- **DL-07 (MEDIUM) — `service_context()` could abort the process.** `ndk_context::android_context()` is `unsafe { ANDROID_CONTEXT.expect(..) }`, so it panics *before* any `is_null()` check can run, and `panic = "abort"` makes that a process abort. `lib.rs` deliberately only warns when seeding fails, so the null check was dead code. No non-panicking accessor exists in ndk-context 0.1.1 and `catch_unwind` cannot help under `abort`, so the fix is to ask our own `SEEDED` flag: new `android_context_seeded()`, gating all three implementations — including `background_service.rs`, which is on the **playback-start** path, where an unseeded context killed the app instead of disabling a bridge.
- **Client ordering, corrected.** `TV` must not lead the no-token case: its guide cell reads *"All formats DRM'd if cookies (logged-in or active guest) aren't passed"*, we send none, and `ANDROID_VR` has no such caveat and is what a device report showed working. `IOS` was third but needs a token we cannot mint, so it is beside `ANDROID` — moved, **not deleted**, because `ios` did resolve twice with real audio urls and the guide's requirement is about GVS, not format discovery. The order is now three named groups (`SERVABLE` / `TOKEN_FREE` / `UNMINTABLE`) and the two branches differ only in whether `SERVABLE` leads, which makes the invariant assertable instead of the flattened array.
- **`web_safari` is now emitted** — it replaced `tv_simply` in yt-dlp's defaults, is web-family, and needs a GVS token we can mint, so it is the one web client never tried, on a network where `mweb` and `web` both resolve to nothing. Adding it exposed a trap: `uaMap` had no `WEB_SAFARI` entry, and `uaMap[winningClient] || uaMap['ANDROID']` would have put an **Android app UA** on a web_safari URL — the mismatch class fixed in v2.6.50, reintroduced one commit later. A test now asserts every emittable client has a `uaMap` entry.
- **The fmt gate was blind, and the cause is worth knowing.** rustfmt cannot break a string literal, so **one unbreakable over-long line makes it abandon the entire enclosing item**. A 141-char JNI signature meant `cached_copy_for_path` was never formatted at all, which is why a misindented line survived unnoticed. Moved to a module `const`; rustfmt then reported ~10 real violations, now fixed. Coverage verified by deliberately misformatting all 38 statements of that body and confirming rustfmt flags each. **`clear_pending_flag` and `publish_legacy` are still outside the gate for the same reason** — owed.
- **The ceiling, recorded so it is not re-litigated.** The token-free client set is only `tv`, `android_vr` and `web_embedded`, and it is shrinking as enforcement rolls out. `bgutils-js` buys us the web family — which on a Jio residential line refused to *resolve* at all. The real fallback is `ANDROID_VR`/`TV`, which is why their DRM caveats matter and why the ordering is load-bearing rather than cosmetic. `visionos` was declined: the guide does not list its token family, so adding it would guess, which is the same mistake as the `TVHTML5` trap in the opposite direction.

**Safety nets (keep all three):** byte accounting in `run_stream` (`downloader.rs`), decoded-duration verification (`completeness.rs` `verify_decoded_duration`, 90 % / 2 s slack) plus the range top-up it triggers, and the JS retry that rotates clients on `Truncated download`.

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
