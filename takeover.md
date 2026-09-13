# Handover — Auralis v2 Notification + Opus Playback

## Objective
- Fix missing Android foreground notification / MediaSession and audio-focus (YouTube keeps playing, Auralis talks over others). Verify on-device via `logcat`/`dumpsys` and ship working release.

## Root Cause Analysis & Resolution (v2.6.36)

### 1. Root Cause of Missing Notification (`NoSuchMethodError`)
The v2.6.35 diagnostic logs captured on device revealed:
```text
W System.err: java.lang.NoSuchMethodError: no static method "Lcom/auralis/v2/MediaPlaybackService;.start(Landroid/content/Context;Ljava/lang/String;Ljava/lang/String;IIZLjava/lang/String;)V"
E AuralisBridge: JNI svc-start failed: Java exception was thrown
W System.err: java.lang.NoSuchMethodError: no static method "Lcom/auralis/v2/MainActivity;.requestRuntimePermissions(Ljava/lang/Object;)V"
E AuralisBridge: JNI perm-request failed: Java exception was thrown
```
By downloading and parsing `classes.dex` from `auralis-v2.6.35-android-arm64.apk`, we confirmed that **Android's R8 / ProGuard code shrinker stripped all companion static methods** (`MediaPlaybackService.start`, `stop`, `MainActivity.requestRuntimePermissions`, and `MediaStoreScanner.queryAllAudio`) because they were only invoked dynamically from Rust via JNI reflection.

### 2. Why the Previous Attempt Failed to Release
Commit `149b989` added `@Keep` annotations and `proguard-rules.pro`, but also introduced invalid Rust code in `src/infrastructure/media/background_service.rs` (a redundant `svc-start-fallback` block and a temporary lifetime borrow error in `with_attached_env`). Because of this, the Android compilation failed in CI with:
- `error[E0277]: the trait bound GlobalRef: Desc<'_, JClass<'_>> is not satisfied`
- `error[E0308]: ? operator has incompatible types`
- `error[E0515]: cannot return value referencing temporary value`
The CI workflow failed, so **no v2.6.36 APK release was ever produced or published**, leaving the device running the broken v2.6.35 binary.

### 3. Resolution Applied
1. **R8 / ProGuard Protection**:
   - Added `@androidx.annotation.Keep` to `MediaPlaybackService` class & companion methods (`start`, `stop`, `createNotificationChannel`, `loadArtBitmapStatic`, `buildNotificationStatic`), `NativeBridge`, `MainActivity` & `requestRuntimePermissions`, and `MediaStoreScanner`.
   - Created `scripts/android/proguard-rules.pro` with explicit `-keep` rules for all JNI target classes and companion objects.
   - Updated `.github/workflows/build.yml` and `.github/workflows/android-test.yml` to install `proguard-rules.pro` into `gen/android/app/proguard-rules.pro`.
2. **Rust JNI Fix**:
   - Removed the broken redundant `svc-start-fallback` block from `background_service.rs:notify` (the fallback is already executed safely inside Kotlin's `MediaPlaybackService.start()` method).
   - Fixed the temporary lifetime issue in `with_attached_env` when extracting `exc.toString()`.
   - `cargo check --lib` verified clean on host.

### 4. Downloader Stream Truncation & Search Download Button Fix (v2.6.38)

#### A. Stream Truncation (Recursive Range Downloader)
- **Root Cause**:
  1. Googlevideo terminates/closes TCP connections after ~10MB chunks.
  2. Previously, `downloader.rs` overwrote `total_bytes` with `res.content_length()`, mistaking the 10MB chunk size for the total file length.
  3. `attempt` was shared across chunk continuations and capped at `MAX_STREAM_RETRIES` (5), aborting any download needing >5 chunks.
  4. HTTP 416 on resume previously deleted the staging file instead of treating verified audio as a completed download.
  5. `expected_duration_secs` was missing from `buildDownloadPayload`, leaving `validate_audio_file` with no duration constraint.
- **Fix**:
  1. Preserved `total_bytes` using `total.max(resp_tot)` and extracted `clen` and `dur` query parameters from YouTube stream URLs as fallback.
  2. Separated `consecutive_errors` from successful chunks: receiving bytes resets consecutive errors to 0, allowing unlimited recursive range requests (`Range: bytes={current_downloaded}-`) across chunk boundaries until all bytes are streamed.
  3. HTTP 416 on verified staging file is recognized as stream completion.
  4. Extracted and passed `expected_duration_secs` through `youtube.js` and `buildDownloadPayload` so `validate_audio_file` strictly verifies audio integrity against actual track duration.

#### B. Internal YouTube Search Download Button
- **Root Cause**:
  1. `downloadSearchResult` was missing `await this.ensureSettings()`, causing PO-tokens and cookies to be absent on fresh start.
  2. In `finally`, the button immediately reverted to "Download" after `download_audio` queued in background (<50ms), leaving no persistent feedback on mobile while `#downloads-list` was scrolled out of view.
  3. Mobile touch events on Android WebView (`onclick` vs `ontouchend` and bubbling to `.track-row`) lacked proper event delegation.
  4. If in-memory `_lastSearchResults` was lost across navigation, the handler silently failed.
- **Fix**:
  1. Added resilient fallback item extraction from DOM `data-video-id`, `data-video-url`, and `data-title` attributes on `.download-yt-btn`.
  2. Added delegated `click` and `touchend` handlers with `touch-action: manipulation` and synchronous debounce guard (`dlBtn.disabled = true;`).
  3. Loaded settings before resolving (`await this.ensureSettings()`) to ensure PO-tokens and download options are populated.
  4. Replaced transient `finally` button reset with persistent `<i data-lucide="check"></i> Added` button state (`btn-secondary`, disabled) on success and re-enable only on failure.
  5. Scrolled `#downloads-list` into view and surfaced clear toast notifications.

## Work State

### Completed
- `src/infrastructure/media/background_service.rs` cleaned up and compiles without errors.
- `@androidx.annotation.Keep` annotations added to all Android JNI classes and methods.
- `scripts/android/proguard-rules.pro` created and wired into both Android build workflows.
- `src/infrastructure/media/downloader.rs` recursive chunked range streaming implemented and tested.
- `ui/js/youtube.js` `clen`/`dur` query fallback and duration propagation implemented.
- `ui/js/modules/downloads.js` search download button event delegation, fallback resolution, and persistent UI feedback implemented.
- Version synced to `2.6.38` across `Cargo.toml`, `Cargo.lock`, `tauri.conf.json`, `package.json`.

### Active
- Tag and trigger CI release for `v2.6.38`.
- Install `auralis-v2.6.38-android-arm64.apk` once CI finishes and verify notification appearance and full-length downloads from YouTube search.

## Next Move
1. Commit and push changes to `main`, push tag `v2.6.38` to trigger release build.
2. Monitor CI run to ensure `build-android` succeeds and the release APK is uploaded.
3. On device, install the new APK and verify:
   ```bash
   logcat -c
   # play a track in Auralis
   logcat -d -s AuralisBridge,AuralisMedia,System.err
   dumpsys activity services com.auralis.v2 | grep -i MediaPlayback
   dumpsys notification | grep -i -A8 "auralis"
   ```

## Relevant Files
- `src/infrastructure/media/background_service.rs` — JNI bridge into Kotlin service
- `scripts/android/MediaPlaybackService.kt` — Foreground media service & notification manager
- `scripts/android/MainActivity.kt` — Activity lifecycle & permission dispatcher
- `scripts/android/proguard-rules.pro` — ProGuard/R8 keep rules
- `.github/workflows/build.yml` — Android release CI workflow
- `.github/workflows/android-test.yml` — Android test CI workflow

## Raw Logs (verbatim)

### 2026-09-09 20:36 IST (Xiaomi Pad 7, failed attempt before v2.6.36 fix)
```
$ logcat -c

$ logcat -d-s AuralisBridge, System.err
size/num main               system             crash              kernel             Total
Total    32101725324/23896121110622140330/6690244583846/172          0/0                42723949500/305863828
Now      3346019/25606      1153057/6388       0/0                0/0                4499076/31994
Logspan  55.659             55.596                                                   55.659
Overhead 1005130            656624                                                   1682498

Chattiest UIDs in main log buffer:                           Size   +/-  Pruned
UID   PACKAGE                                               BYTES           NUM
10161 com.miui.home                                       1505254
1000  system                                              1256150
  PID/UID   COMMAND LINE                                       "
   2279/1000  /system/bin/surfaceflinger                     417784
    4333/1000  com.android.systemui                           331354
     2056/1000 ...vendor.qti.hardware.display.composer-service 259058
      2443/1000  system_server                                  159237
       2087/1000 ...endor.qti.hardware.servicetrackeraidl-service 24878
       1041  audioserver                                          370709
       10183 com.google.android.inputmethod.latin                  69727
       10173 com.google.android.googlequicksearchbox               50549


       Chattiest UIDs in system log buffer:                         Size   +/-  Pruned
       UID   PACKAGE                                               BYTES           NUM
       1000  system                                              1015920
         PID/UID   COMMAND LINE                                       "
          2443/1000  system_server                                  658490
           4333/1000  com.android.systemui                           355719
           10414 in.hridayan.ashell                                    57165
           10161 com.miui.home                                         36752
           10173 com.google.android.googlequicksearchbox               20636
           10183 com.google.android.inputmethod.latin                  11510
           10417 com.auralis.v2                                        10422


           $ logcat -d | grep -iE "System\.err|AuralisBridge | AuralisMedia|svc-start | NoSuchMethod | ClassNot Found | Foreground ServiceStartNotAllowed | Bad notification | RemoteServiceException" | head -n 60
           09-09 20:36:32.003 21632 21817 W System.err: android.content.pm.PackageManager$NameNotFoundException: com.termux.api
           09-09 20:36:32.003 21632 21817 W System.err: 	at android.app.ApplicationPackageManager.getPackageInfoAsUser(ApplicationPackageManager.java:283)
           09-09 20:36:32.003 21632 21817 W System.err: 	at android.app.ApplicationPackageManager.getPackageInfo(ApplicationPackageManager.java:243)
           09-09 20:36:32.003 21632 21817 W System.err: 	at android.app.ApplicationPackageManager.getPackageInfo(ApplicationPackageManager.java:237)
           09-09 20:36:32.003 21632 21817 W System.err: 	at android.util.MiuiMultiWindowAdapter.calFreeformSuggestionList(MiuiMultiWindowAdapter.java:1207)
           09-09 20:36:32.003 21632 21817 W System.err: 	at android.util.MiuiMultiWindowUtils.getFreeformSuggestionList(MiuiMultiWindowUtils.java:3291)
           09-09 20:36:32.003 21632 21817 W System.err: 	at java.lang.reflect.Method.invoke(Native Method)
           09-09 20:36:32.003 21632 21817 W System.err: 	at com.miui.launcher.utils.ReflectUtils.invokeObject(ReflectUtils.java:116)
           09-09 20:36:32.003 21632 21817 W System.err: 	at com.miui.launcher.utils.ReflectUtils.callStaticMethod(ReflectUtils.java:76)
           09-09 20:36:32.003 21632 21817 W System.err: 	at com.miui.home.launcher.RecentsAndFSGestureUtils.getSuggestionList(RecentsAndFSGestureUtils.java:320)
           09-09 20:36:32.003 21632 21817 W System.err: 	at com.miui.home.launcher.RecentsAndFSGestureUtils.lambda$updateFreeformSuggestionList$1(RecentsAndFSGestureUtils.java:303)
           09-09 20:36:32.003 21632 21817 W System.err: 	at com.miui.home.launcher.RecentsAndFSGestureUtils.$r8$lambda$LZin6DRXj7G-7_LtcHxGXZMOL58(RecentsAndFSGestureUtils.java:0)
           09-09 20:36:32.003 21632 21817 W System.err: 	at com.miui.home.launcher.RecentsAndFSGestureUtils$$ExternalSyntheticLambda0.apply(R8$$SyntheticClass:0)
           09-09 20:36:32.003 21632 21817 W System.err: 	at com.miui.home.library.utils.AsyncTaskExecutorHelper$4.doInBackground(AsyncTaskExecutorHelper.java:138)
           09-09 20:36:32.003 21632 21817 W System.err: 	at com.miui.home.library.utils.AsyncTaskExecutorHelper$4.doInBackground(AsyncTaskExecutorHelper.java:133)
           09-09 20:36:32.003 21632 21817 W System.err: 	at android.os.AsyncTask$3.call(AsyncTask.java:397)
           09-09 20:36:32.003 21632 21817 W System.err: 	at java.util.concurrent.FutureTask.run(FutureTask.java:328)
           09-09 20:36:32.003 21632 21817 W System.err: 	at java.util.concurrent.ThreadPoolExecutor.runWorker(ThreadPoolExecutor.java:1100)
           09-09 20:36:32.003 21632 21817 W System.err: 	at java.util.concurrent.ThreadPoolExecutor$Worker.run(ThreadPoolExecutor.java:624)
           09-09 20:36:32.003 21632 21817 W System.err: 	at java.lang.Thread.run(Thread.java:1572)
           09-09 20:36:32.047 21632 21817 W System.err: android.content.pm.PackageManager$NameNotFoundException: android.display
           09-09 20:36:32.048 21632 21817 W System.err: 	at android.app.ApplicationPackageManager.getPackageInfoAsUser(ApplicationPackageManager.java:283)
           09-09 20:36:32.048 21632 21817 W System.err: 	at android.app.ApplicationPackageManager.getPackageInfo(ApplicationPackageManager.java:243)
           09-09 20:36:32.048 21632 21817 W System.err: 	at android.app.ApplicationPackageManager.getPackageInfo(ApplicationPackageManager.java:237)
           09-09 20:36:32.048 21632 21817 W System.err: 	at android.util.MiuiMultiWindowAdapter.calFreeformSuggestionList(MiuiMultiWindowAdapter.java:1207)
           09-09 20:36:32.048 21632 21817 W System.err: 	at android.util.MiuiMultiWindowUtils.getFreeformSuggestionList(MiuiMultiWindowUtils.java:3291)
           09-09 20:36:32.048 21632 21817 W System.err: 	at java.lang.reflect.Method.invoke(Native Method)
           09-09 20:36:32.048 21632 21817 W System.err: 	at com.miui.launcher.utils.ReflectUtils.invokeObject(ReflectUtils.java:116)
           09-09 20:36:32.048 21632 21817 W System.err: 	at com.miui.launcher.utils.ReflectUtils.callStaticMethod(ReflectUtils.java:76)
           09-09 20:36:32.048 21632 21817 W System.err: 	at com.miui.home.launcher.RecentsAndFSGestureUtils.getSuggestionList(RecentsAndFSGestureUtils.java:320)
           09-09 20:36:32.048 21632 21817 W System.err: 	at com.miui.home.launcher.RecentsAndFSGestureUtils.lambda$updateFreeformSuggestionList$1(RecentsAndFSGestureUtils.java:303)
           09-09 20:36:32.048 21632 21817 W System.err: 	at com.miui.home.launcher.RecentsAndFSGestureUtils.$r8$lambda$LZin6DRXj7G-7_LtcHxGXZMOL58(RecentsAndFSGestureUtils.java:0)
           09-09 20:36:32.048 21632 21817 W System.err: 	at com.miui.home.launcher.RecentsAndFSGestureUtils$$ExternalSyntheticLambda0.apply(R8$$SyntheticClass:0)
           09-09 20:36:32.048 21632 21817 W System.err: 	at com.miui.home.library.utils.AsyncTaskExecutorHelper$4.doInBackground(AsyncTaskExecutorHelper.java:138)
           09-09 20:36:32.048 21632 21817 W System.err: 	at com.miui.home.library.utils.AsyncTaskExecutorHelper$4.doInBackground(AsyncTaskExecutorHelper.java:133)
           09-09 20:36:32.048 21632 21817 W System.err: 	at android.os.AsyncTask$3.call(AsyncTask.java:397)
           09-09 20:36:32.048 21632 21817 W System.err: 	at java.util.concurrent.FutureTask.run(FutureTask.java:328)
           09-09 20:36:32.048 21632 21817 W System.err: 	at java.util.concurrent.ThreadPoolExecutor.runWorker(ThreadPoolExecutor.java:1100)
           09-09 20:36:32.048 21632 21817 W System.err: 	at java.util.concurrent.ThreadPoolExecutor$Worker.run(ThreadPoolExecutor.java:624)
           09-09 20:36:32.048 21632 21817 W System.err: 	at java.lang.Thread.run(Thread.java:1572)
           09-09 20:36:32.067 21632 21817 W System.err: android.content.pm.PackageManager$NameNotFoundException: com.termux.api
           09-09 20:36:32.067 21632 21817 W System.err: 	at android.app.ApplicationPackageManager.getPackageInfoAsUser(ApplicationPackageManager.java:283)
           09-09 20:36:32.067 21632 21817 W System.err: 	at android.app.ApplicationPackageManager.getPackageInfo(ApplicationPackageManager.java:243)
           09-09 20:36:32.067 21632 21817 W System.err: 	at android.app.ApplicationPackageManager.getPackageInfo(ApplicationPackageManager.java:237)
           09-09 20:36:32.067 21632 21817 W System.err: 	at android.util.MiuiMultiWindowAdapter.calFreeformSuggestionList(MiuiMultiWindowAdapter.java:1207)
           09-09 20:36:32.067 21632 21817 W System.err: 	at android.util.MiuiMultiWindowUtils.getFreeformSuggestionList(MiuiMultiWindowUtils.java:3291)
           09-09 20:36:32.068 21632 21817 W System.err: 	at java.lang.reflect.Method.invoke(Native Method)
           09-09 20:36:32.068 21632 21817 W System.err: 	at com.miui.launcher.utils.ReflectUtils.invokeObject(ReflectUtils.java:116)
           09-09 20:36:32.068 21632 21817 W System.err: 	at com.miui.launcher.utils.ReflectUtils.callStaticMethod(ReflectUtils.java:76)
           09-09 20:36:32.068 21632 21817 W System.err: 	at com.miui.home.launcher.RecentsAndFSGestureUtils.getSuggestionList(RecentsAndFSGestureUtils.java:320)
           09-09 20:36:32.068 21632 21817 W System.err: 	at com.miui.home.launcher.RecentsAndFSGestureUtils.lambda$updateFreeformSuggestionList$1(RecentsAndFSGestureUtils.java:303)
           09-09 20:36:32.068 21632 21817 W System.err: 	at com.miui.home.launcher.RecentsAndFSGestureUtils.$r8$lambda$LZin6DRXj7G-7_LtcHxGXZMOL58(RecentsAndFSGestureUtils.java:0)
           09-09 20:36:32.068 21632 21817 W System.err: 	at com.miui.home.launcher.RecentsAndFSGestureUtils$$ExternalSyntheticLambda0.apply(R8$$SyntheticClass:0)
           09-09 20:36:32.068 21632 21817 W System.err: 	at com.miui.home.library.utils.AsyncTaskExecutorHelper$4.doInBackground(AsyncTaskExecutorHelper.java:138)
           09-09 20:36:32.068 21632 21817 W System.err: 	at com.miui.home.library.utils.AsyncTaskExecutorHelper$4.doInBackground(AsyncTaskExecutorHelper.java:133)
           09-09 20:36:32.068 21632 21817 W System.err: 	at android.os.AsyncTask$3.call(AsyncTask.java:397)
           09-09 20:36:32.068 21632 21817 W System.err: 	at java.util.concurrent.FutureTask.run(FutureTask.java:328)
           09-09 20:36:32.068 21632 21817 W System.err: 	at java.util.concurrent.ThreadPoolExecutor.runWorker(ThreadPoolExecutor.java:1100)
           09-09 20:36:32.068 21632 21817 W System.err: 	at java.util.concurrent.ThreadPoolExecutor$Worker.run(ThreadPoolExecutor.java:624)
           09-09 20:36:32.068 21632 21817 W System.err: 	at java.lang.Thread.run(Thread.java:1572)

           $ dumpsys activity services com.auralis.v2 | grep -i MediaPlayback

           $ dumpsys notification | grep -i -A8 "auralis" | head -n 30
                 AppSettings: com.auralis.v2 (10417) importance=DEFAULT userSet=true
                         NotificationChannel{mId='auralis_playback_channel_v2', mName=Aud..., mDescription=hasDescription , mImportance=3, mBypassDnd=false, mLockscreenVisibility=-1000, mSound=null, mLights=false, mLightColor=0, mVibrationPattern=null, mVibrationEffect=null, mUserLockedFields=0, mUserVisibleTaskShown=false, mVibrationEnabled=false, mShowBadge=true, mDeleted=false, mDeletedTimeMs=-1, mGroup='null', mAudioAttributes=null, mBlockableSystem=false, mAllowBubbles=-1, mImportanceLockedDefaultApp=false, mOriginalImp=3, mParent=null, mConversationId=null, mDemoted=false, mImportantConvo=false, mLastNotificationUpdateTimeMs=0}
```