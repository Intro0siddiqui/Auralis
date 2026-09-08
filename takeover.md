# Handover — Auralis v2 Notification + Opus Playback

## Objective
- Fix silent Opus playback (WebM/`A_OPUS` `1A 45 DF A3`), missing Android foreground notification / MediaSession, and audio-focus (YouTube keeps playing, Auralis talks over others). Verify via on-device `logcat`/`dumpsys` and ship versioned releases.

## Important Details
- Test file `https://d.uguu.se/jXSTGTDj.m4a` is EBML/WebM `google/video-file` `A_OPUS` `OpusHead` `opus 48000Hz stereo 180.301s`, byte-identical to `scratch/sample.m4a` (`md5 0d0e07f981a634dd0f5196658c7148cf`), link now expired.
- Devices: Realme 11x 5G Android 16 arm64, Xiaomi Pad 7 HyperOS/Android 15 Snapdragon arm64; Pixel emulator shows notification (real device diverged).
- Stack: `rusty-opus 0.9.1`, `symphonia 0.5.5`, `rodio 0.22.2`, `lofty 0.25`, Tauri v2, `targetSdk/compileSdk 36`, `jni 0.21.1`, `ndk-context 0.1`.
- Probe results: `lofty guess -> Some(Mpeg) -> ERR: failed to parse Mpeg`; `rodio m4a-hint/no-hint -> BUILD FAIL`; `symphonia webm-hint -> PROBE OK, opus, n_frames 180301`.
- No-PC debug: Shizuku + aShell via wireless debugging, `logcat -c`, `logcat -d -s ...`, `dumpsys activity services`, `dumpsys notification`.
- Local `cargo test` cannot link (proot `ring` `__stack_chk_guard` DSO); use `cargo check --lib` + `cargo clippy --all-targets --all-features -D warnings` + CI for Android builds.
- Unified symptom theory (confirmed by missing `AuralisMedia` logs): service never starts → no `startForeground` → no notification + no `requestAudioFocus()` (YouTube not paused) + no focus-loss handling.

## Work State

### Completed
- `src/infrastructure/media/downloader.rs:192` `is_ebml_container()` + `try_opus_fallback()` via `extract_opus_metadata` ±5s tolerance — hooks at EBML fast-path, `lofty.read()` failure, rodio probe failure.
- `src/infrastructure/media/opus.rs` `OpusDecoderEngine::RustyOpus` exact `nb_frames*samples_per_frame(toc)` via `rusty_opus::repacketizer`; `scratch/sample.m4a` `total=1440000 nonzero=1132365 (78.6%) peak=0.5419`; hardened test `>20%`; `trim_start/trim_end*channels` gapless drain.
- Android notification/theme `v2.6.28`: `CHANNEL_ID=auralis_playback_channel_v2` `IMPORTANCE_DEFAULT` `VISIBILITY_PUBLIC` `startForeground` try/catch fallback to `notify()`; `MainActivity.kt` `pendingPermissionRequest` retry in `onResume`, `disableWebViewForceDark()`.
- `v2.6.29 c096423` CI `WebSettings.FORCE_DARK_OFF` fix.
- `v2.6.30 9e1b3f5` silent Opus fix shipped.
- Subagent verified libopus `frame_size` is capacity (`opus.h` RFC 6716), `rusty-opus::decode` exact; leading 137× 3-byte `TOC 0xFC` DTX packets are YouTube pre-roll.
- `v2.6.31 49f39df` JNI sig `(Ljava/lang/Object;)V` for `requestRuntimePermissions`.
- PR #35 `baf1cfd/7c5d354` → `v2.6.32 763ddf2`: `Icon.createWithResource("android", android.R.drawable.*)` fixes `Resources$NotFoundException` in SystemUI.
- PR #36 `2316634/19d7e9c` → `v2.6.33 315c977`: `start()` wraps `startForegroundService` in try/catch with `NotificationManager.notify()` fallback, `buildNotificationStatic`/`loadArtBitmapStatic`.
- `1b272a4` + `v2.6.34 3540ef4`: JNI classloader-robust bridge — `SERVICE_CLASS_REF`/`ACTIVITY_CLASS_REF` `OnceLock<GlobalRef>`, `app_class()` fast-path + `class_via_app_loader()` via `getClassLoader().loadClass()` with `exception_clear()`, `precache_classes()` from `JNI_OnLoad` in `src/lib.rs`, `logcat_error()` mirroring to `android.util.Log.e` as `AuralisBridge`.
- `37d1393` + `v2.6.35` (2026-09-08): per-callsite `with_attached_env(op, f)` (`svc-start`/`svc-stop`/`perm-request`) + `exception_describe()` before `exception_clear()` so `System.err` stack appears in logcat. Bump `Cargo.toml`/`Cargo.lock`/`tauri.conf.json`/`package.json` `2.6.34→2.6.35`, `cargo check` OK, `git tag v2.6.35` pushed, CI `in_progress` at handover.

### Active
- `v2.6.35` not yet on device at last log `2026-09-08 15:28:17 IST` — log still shows generic `JNI call failed: Java exception was thrown` without op tag, proving old binary. Need fresh `System.err` trace to classify throw:
  - `ClassNotFoundException` → app_class fallback still failing
  - `NoSuchMethodError` → `start`/`requestRuntimePermissions` descriptor mismatch
  - `ForegroundServiceStartNotAllowedException` / `SecurityException` / `Bad notification` → Kotlin throw inside `MediaPlaybackService.start()` that escaped catch
- Latest on-device evidence (Xiaomi Pad 7, pid `23929`, uid `10416`):
  ```
  09-08 15:28:43.665 23929 24046 E AuralisBridge: JNI call failed: Java exception was thrown
  09-08 15:28:43.666 23929 24046 E AuralisBridge: JNI call failed: Java exception was thrown
  09-08 15:28:55.778 23929 24046 E AuralisBridge: JNI call failed: Java exception was thrown
  logcat -d -s AuralisMedia → (empty)
  dumpsys notification → AppSettings com.auralis.v2 importance=DEFAULT, NotificationChannel mId='auralis_playback_channel_v2' mImportance=3 mLastNotificationUpdateTimeMs=0
  dumpsys activity services com.auralis.v2 | grep i mediplayback → user typo `grep i` (no `-`) → `grep: mediplayback: No such file or directory` + `Broken pipe`
  ```

### Blocked
- No `System.err` stack until device installs `v2.6.35` (`gh release view v2.6.35` → `auralis-v2.6.35-android-arm64.apk`) and re-runs `logcat -c` → play → `logcat -d -s AuralisBridge,System.err`.
- `cargo check --target aarch64-linux-android` blocked by missing `aarch64-linux-android-clang` (`ring v0.17.14`).

## Next Move
1. Wait for `v2.6.35` CI success (`gh run list --limit 5`), install APK on Xiaomi Pad 7.
2. Run canonical capture (exact flags):
   ```bash
   logcat -c
   # play track in Auralis (or `adb shell am start -n com.auralis.v2/.MainActivity`)
   logcat -d -s AuralisBridge,System.err
   logcat -d | grep -iE "System\.err|AuralisBridge|AuralisMedia|svc-start|svc-stop|perm-request|Exception|NoSuchMethod|ClassNotFound|ForegroundServiceStartNotAllowed|Bad notification" | head -n 40
   dumpsys activity services com.auralis.v2 | grep -i MediaPlayback
   dumpsys notification | grep -i -A8 "auralis" | head -n 30
   ```
   Note: last handover’s `grep i mediplayback` is missing `-i`; correct is `grep -i MediaPlayback`.
3. Classify exception from `System.err` stack:
   - If `ClassNotFoundException` → fix `class_via_app_loader` dotted/slashed + `service_context` null guard.
   - If `NoSuchMethodError` → `javap -s` Kotlin `start`/`requestRuntimePermissions` descriptors, correct `call_static_method` sig.
   - If Kotlin internal throw (e.g. `ForegroundServiceStartNotAllowedException` escaping) → widen `try/catch` in `MediaPlaybackService.kt:start()` to wrap entire method body, not just `startForegroundService`.
4. Cut `v2.6.36` with targeted fix, re-test for `AuralisMedia` log + `mLastNotificationUpdateTimeMs != 0` + `dumpsys activity services` showing `MediaPlaybackService` + audio-focus duck/pause.

## Relevant Files
- `src/infrastructure/media/background_service.rs:300` `with_attached_env(op, f)` + `app_class()`/`class_via_app_loader()`/`precache_classes()`/`logcat_error()`/`service_context()` — JNI bridge
- `src/lib.rs:android_jni::INITIAL_VM` `JNI_OnLoad` + `init_android_context` `try_seed`
- `src/commands/playback.rs` `push_now_playing` `notify_playing/paused` `stop_service` emitters
- `scripts/android/MediaPlaybackService.kt:84` `start()`/`CHANNEL_ID`/`NOTIFICATION_ID=101`/`buildNotificationStatic`/`loadArtBitmapStatic`/`requestAudioFocus()`
- `scripts/android/MainActivity.kt:28` `requestRuntimePermissions(context: Any?)` `pendingPermissionRequest` `disableWebViewForceDark`
- `src/infrastructure/media/downloader.rs:192` `is_ebml_container` `try_opus_fallback` `sanitize_filename`
- `src/infrastructure/media/opus.rs` `OpusDecoderEngine` gapless trim
- `src/infrastructure/media/player.rs` `create_decoder` EBML sniff
- `.github/workflows/build.yml` manifest inject `FOREGROUND_SERVICE_MEDIA_PLAYBACK` `MediaPlaybackService` + Kotlin copy, `compileSdk/targetSdk 36`, `zipalign -P 16` `llvm-readelf p_align 0x4000`
- `Cargo.toml:3` `tauri.conf.json:4` `package.json:3` `Cargo.lock:278` version `2.6.35`
- `scratch/sample.m4a` reference for expired uguuse URL
- `takeover.md` (this file) — raw logs preserved below for audit

## Raw Logs (verbatim, newest first)

### 2026-09-08 15:28 IST (Xiaomi Pad 7, pid 23929, v2.6.34 still installed)
```
$ date
Tue Sep  8 15:28:17 IST 2026

$ logcat -c

$ logcat -d -s AuralisBridge
--------- beginning of main
09-08 15:28:43.665 23929 24046 E AuralisBridge: JNI call failed: Java exception was thrown
09-08 15:28:43.666 23929 24046 E AuralisBridge: JNI call failed: Java exception was thrown
09-08 15:28:55.778 23929 24046 E AuralisBridge: JNI call failed: Java exception was thrown

$ logcat -d -s AuralisMedia
(empty)

$ logcat -d | grep -iE "Bad notification | RemoteServiceException|auralis" | head -n 20
09-08 15:28:38.485 21632 21817 D MiuiMultiWindowUtils: getFreeformSuggestionList end result size:[..., com.auralis.v2, ...]
09-08 15:28:41.046 21632 21632 D Launcher.CellLayout: touch item:ShortcutInfo, id=122, itemType=0, pkgName=com.auralis.v2, className=com.auralis.v2.MainActivity
09-08 15:28:41.064  2443  2585 I AppStartScenario: notifyScenarioChanged: active=true param=Bundle[{hostingRecordName={com.auralis.v2/com.auralis.v2.MainActivity}, pid=23929, uid=10416, type=1, state=3}]
09-08 15:28:41.066  2443  2585 I ActivityManager: Start proc 23929:com.auralis.v2/u0a416 for prestart-top-activity {com.auralis.v2/com.auralis.v2.MainActivity}
...

$ dumpsys activity services com.auralis.v2 | grep i mediplayback | head -n 5
grep: mediplayback: No such file or directory
Failed to write while dumping service activity: Broken pipe

$ dumpsys notification | grep -i -A8 "auralis" | head -n 30  (from 2026-09-07, still representative — channel not updated)
AppSettings: com.auralis.v2 (10415) importance=DEFAULT userSet=true
NotificationChannel{mId='auralis_playback_channel_v2', mName=Aud..., mImportance=3, mBypassDnd=false, mLockscreenVisibility=-1000, mSound=null, mLights=false, mVibrationPattern=null, mUserLockedFields=0, mShowBadge=true, mDeleted=false, mDeletedTimeMs=-1, mGroup='null', mAudioAttributes=null, mBlockableSystem=false, mAllowBubbles=-1, mImportanceLockedDefaultApp=false, mOriginalImp=3, mParent=null, mConversationId=null, mDemoted=false, mImportantConvo=false, mLastNotificationUpdateTimeMs=0}
```

### 2026-09-07 15:19 IST (earlier run, same symptoms)
```
$ logcat -d -s AuralisMedia
(empty)
$ logcat -d | grep -iE "Bad notification | RemoteServiceException|auralis" | head -n 20
09-07 15:19:15.807 ... MiuiPadHome_ActivityManagerWrapper ... com.auralis.v2/com.auralis.v2.MainActivity
...
$ dumpsys activity services com.auralis.v2 | grep -i mediplayback | head -n 5
(empty — same grep typo as above: `grep i mediplayback` without dash)
$ dumpsys notification | grep -i -A8 "auralis" | head -n 30
AppSettings: com.auralis.v2 (10415) importance=DEFAULT userSet=true
NotificationChannel{mId='auralis_playback_channel_v2', mName=Aud..., mImportance=3, mLastNotificationUpdateTimeMs=0}
```

## Build & Version
- Current head: `37d1393` `v2.6.35` `fix(android): per-callsite JNI diagnostics + exception_describe` — pushed `main` + tag `v2.6.35`
- Previous: `3540ef4` `v2.6.34` JNI classloader-robust bridge, `315c977` `v2.6.33` fallback notify, `763ddf2` `v2.6.32` Icon fix, `49f39df` `v2.6.31` sig fix, `9e1b3f5` `v2.6.30` Opus
- Verify: `gh run list --limit 5`, `gh release view v2.6.35`, APK dex must contain `MediaPlaybackService`/`MainActivity`/`NativeBridge`; CI enforces `zipalign -P 16` + `llvm-readelf p_align 0x4000` for 64-bit `.so`.

## Agent Notes
- `takeover.md` previously contained only raw shell dumps; now structured per AGENTS.md §8 handover spec. Keep appending raw logs verbatim under `## Raw Logs` — do not delete.
- `cargo fmt --check` and `cargo clippy --all-targets --all-features -D warnings` pass at handover; `cargo check --lib` OK. Do not run `cargo test` locally (proot ring failure).
- The `grep i mediplayback` typo has appeared twice — next agent must use `grep -i MediaPlayback` and `logcat -d -s AuralisBridge,System.err` to avoid false `Broken pipe` and missing `System.err`.
