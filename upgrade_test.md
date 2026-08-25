# Upgrade Test — remember for next YTDL fix

## Why this file
CI `build-linux` + `build-android` `pixel_6 api33` currently only verifies synthetic path (`test.wav` `127.0.0.1` + `outfoxing.mp3` fallback `1831788 bytes`) via `scripts/tests/desktop_download_player_e2e.js` / `scripts/android/e2e_download_test.js`. `scripts/tests/youtube_resolver.test.js` (24 tests) mocks `nativeFetch` / `hasDirectOrDecipherableAudio` — no live `youtubei/v1/player` + `googlevideo` `rr1---sn-gwpa-cived` `403` / `&pot=` / `&n=` coverage. That let `YAD (Яд)` `7C4-TAWg7QA` / `Sx8z0U0lkjQ` `2409:40c4:35b:b681:8000::` slip on `v2.5.7`.

## Resume fixing YTDL functionality (next tag v2.5.9 — lint/clippy fix shipped 2026-08-24)
- [x] Mint `po_token` for **all** clients (`TV`, `ANDROID_VR` included) — `ui/js/modules/po_token.js:86` `generatePoTokenForVideo(videoId)` `WebPoMinter` `ENGAGEMENT_TYPE_UNBOUND` `contentBinding=videoId` `6h` cache, `visitorData`-bound, `nativeFetch` for `jnn-pa.googleapis.com` (`AIzaSyDyT5W...`) + `interpreter_url` (protobuf via `bgutils` `buildURL`/`getHeaders`, not bare JSON).
- [x] Attach `&pot=` unconditionally — remove `sabr!=1` guard in `ui/vendor/youtubei.esm.mjs` `searchParams.set('pot', po_token)` so `TV`/`MWEB`/`WEB` also carry `pot` (ignored when not needed, required on `sn-gwpa-cived` `2026-02` Jio CGNAT).
- [x] Auto-retry on `403` — `src/infrastructure/media/downloader.rs:214` + `src/commands/downloads.rs:54` `download:diagnostic` + `ui/js/youtube.js:365` on `HTTP 403 Forbidden [rr1---] body:(empty)` re-`resolve` with next `orderedClient` (`TV` → `ANDROID+pot` → `WEB_SAFARI` `SABR` `SabrStream` `formats[18]` legacy `videoplayback?range=`) before surfacing toast / `Copy error`.
- [x] SABR quality — `ui/js/youtube.js:293` `hasLegacyProgressiveFallback` keep `audio-only` `140` `m4a` over muxed `18` `mp4`, prefer `webm/opus`, set `ext` via `extFromMime`.
- [x] Unit: assert `pot` in `videoplayback` URL for `TV` when `po_token.js` returns token (`youtube_resolver.test.js` `searchParams.has('pot')`).

## Upgrade test plan (add without blocking releases yet)
- [x] New job `live-youtube-e2e` in `.github/workflows/build.yml` (or `workflow_dispatch` + `schedule: cron nightly`) that hits live `POST https://www.youtube.com/youtubei/v1/player?key=AIzaSyAO_FJ2SlqU8Q4STEHLGCilw_Y9_11qcW8` `{"videoId":"7C4-TAWg7QA","context":{"client":{"clientName":"ANDROID","clientVersion":"20.10.38"}}}` and `Sx8z0U0lkjQ`, asserts `playabilityStatus:OK`, then drives `download_audio(url, headers{UA,Referer,Origin,pot})` → `HTTP 200` `content-type audio/*` `>10KB`, `list_downloads` → `scan_library_paths` → `get_tracks` → `play` → `get_now_playing is_playing` (same as `desktop_download_player_e2e.js` but with real googlevideo, not `test.wav`). Retry 2×, `continue-on-error` until `BgUtils` stable, then gate `release`.
- [x] Desktop: extend `desktop_real_e2e.js` with optional `YOUTUBE_LIVE=1` path that does `resolve` + `download` + `play` for `7C4-TAWg7QA`; Android: extend `e2e_download_test.js` similarly on emulator (host `127.0.0.1:9222` CDP, already uses `ZYEz2EKwrQ4` mocked — add live branch).

## Verify
```
node --check ui/js/youtube.js && node --check ui/js/modules/po_token.js
cargo fmt && cargo check --all-targets && cargo check --target aarch64-linux-android
node --test scripts/tests/youtube_resolver.test.js   # 24 + new pot-for-TV
gh run view <id> --log-failed | grep -E "winningClient|PoToken|pot|403"
adb logcat -s chromium | grep -E "actions.execute|winningClient|PoToken"   # on Pad 7
# incognito: https://youtu.be/7C4-TAWg7QA must play (not LOGIN_REQUIRED)
# on-device: Settings → Downloads → YouTube cookie (paste document.cookie if needed), retry YAD SLOWED, expect Copy error gone, rr1--- 200
```

## References
- `tauri.conf.json:33` CSP must allow `https://jnn-pa.googleapis.com` + `https://*.googlevideo.com` + `https://*.google.com`
- `AGENTS.md:148,161,224` + `PROJECT.md:352` version `2.5.7` → `2.5.8`, `TAURI_CLI_VERSION 2.11.4`, `NDK 27.2.12479018`, `compileSdk/targetSdk 36`
- `v2.5.7` tag `a4c6bc3` base; next `v2.5.8` cherry-picks pot-unconditional + 403-retry.

- [x] v2.5.9 fix: clippy identity map + PendingResponseMap + live job without cargo test (no webkit) + semicolon
- [x] v2.5.10: player `play()` fresh-start no-op fix — `ui/js/player.js:445 async play()` `currentTrack ? resume : get_queue → playTrack : get_tracks limit1 → playTrack` (agent), `rodio` `webm/opus` note (vorbis ≠ opus, prefer `m4a` or add `opus` feature), docs sync `2.5.10` (`tauri.conf.json` CSP `jnn-pa`/`googlevideo`/`ytimg`, `AGENTS.md §4.5` download path `app_data_dir/downloads`)
- [ ] v2.5.11: Download/Auralis MediaStore dual-save — `android_downloads.rs` `publish_to_downloads` (`MediaStore.Downloads` `IS_PENDING` on API 29+, legacy `Environment.getExternalStoragePublicDirectory` + `MediaScanner` on 26-28) when `use_system_downloads` true (`settings.rs` `DownloadSettings.use_system_downloads` default true, `settings.html`/`views.js` toggle), `downloader.rs` + `player.rs` `cached_copy_for_path` fallback, `library.rs` sandboxed scan note; **MediaStore + filesystem verification:** `cargo check --all-targets && cargo check --target aarch64-linux-android` passes, on-device `download_audio` → `ls /storage/emulated/0/Download/Auralis/` + `content query --uri content://media/external/file --where "relative_path like '%Download/Auralis%'"` shows entry + `ls /data/data/com.auralis.v2/files/downloads/` internal copy still exists (no duplicate DB entry via `get_tracks` count), `play` fallback via `cached_copy_for_path` if internal missing, toggle `use_system_downloads=false` keeps only internal; docs sync `2.5.11` (`Cargo.toml`/`Cargo.lock`/`tauri.conf.json`/`package.json` + `AGENTS.md §4.5`/`PROJECT.md §9`/`README` dual-save)
