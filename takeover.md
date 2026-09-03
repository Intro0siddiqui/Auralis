# Takeover Guide: Critical Runtime Issues & Solutions

This document outlines four critical runtime bugs in Auralis v2, their real-world symptoms, root causes, and the verified fixes applied to resolve them.

---

## 1. Opus Audio File Playing Silently (No Sound Output)

### Runtime Symptoms
- The track loaded into the player bar and full-screen view.
- The elapsed time and progress bar advanced normally.
- **No audible sound was emitted from speakers or headphones.**
- Sample file tested: `scratch/sample.m4a` (downloaded from YouTube / third-party sources).

### Root Cause
1. **Container / Extension Discrepancy**: Hex analysis of the file header revealed EBML magic bytes (`0x1A 0x45 0xDF 0xA3`), which identify a Matroska/WebM container carrying Opus audio. The file had been mislabeled or saved with an `.m4a` extension.
2. **Blind Extension Dispatch**: In `src/infrastructure/media/player.rs`, `create_decoder` relied on the `.m4a` extension to route the stream directly to Rodio's MP4/AAC decoder. The decoder accepted the stream but yielded empty PCM blocks without returning an error.
3. **Misleading Demuxer Hint**: When fallback was attempted, `src/infrastructure/media/opus.rs` passed the file's extension (`"m4a"`) to Symphonia's `Hint`. Symphonia attempted an MP4 probe and failed to demux the WebM container.

### The Fix
- **EBML Header Sniffing**: In `create_decoder` (`src/infrastructure/media/player.rs`), inspect the first 4 bytes of the file stream before inspecting extensions. If the header matches `b"\x1a\x45\xdf\xa3"`, immediately bypass MP4 and instantiate an `OpusSource`.
- **Enforced Container Hint**: In both `OpusSource::new` and `extract_opus_metadata` (`src/infrastructure/media/opus.rs`), detect EBML magic bytes and force `hint.with_extension("webm")`. This ensures Symphonia's WebM demuxer parses the container and delegates to the native Opus sample decoder.

---

## 2. Android Playback Notification Not Displaying

### Runtime Symptoms
- Audio played in the background on Android devices.
- `POST_NOTIFICATIONS` permission was granted by the user.
- **No playback notification or media controls appeared in the Android notification shade or lockscreen.**

### Root Cause
1. **Suppressed Notification Channel**: In `scripts/android/MediaPlaybackService.kt`, `createNotificationChannel` configured the playback channel with `NotificationManager.IMPORTANCE_LOW` and `setShowBadge(false)`. On modern Android (Android 13–16 / API 33–36), `IMPORTANCE_LOW` notifications are treated as ambient/silent notifications and suppressed from the status bar, lockscreen, and heads-up banner.
2. **Adaptive Icon Rejection**: In `buildNotification`, the small icon fallback used `applicationInfo.icon`. On Android 13+, the status bar small icon (`setSmallIcon`) requires a monochrome alpha-only vector drawable. Passing an adaptive color bitmap icon causes SystemUI to suppress the notification or fail rendering.

### The Fix
- **Channel Importance Upgrade**: In `scripts/android/MediaPlaybackService.kt`, set `NotificationManager.IMPORTANCE_DEFAULT` with `lockscreenVisibility = Notification.VISIBILITY_PUBLIC`.
- **System Vector Fallback**: Use `android.R.drawable.ic_media_play` for `iconRes` in `buildNotification` to guarantee valid monochrome rendering across all Android versions.

---

## 3. "Scan Storage" Button Inactive on Android

### Runtime Symptoms
- Tapping the "Scan Storage" button in the Library view did nothing on Android.
- No folder picker was displayed, and local device tracks were not indexed.

### Root Cause
- **Unsupported Attribute in Mobile WebViews**: In `ui/js/modules/library.js`, `triggerFolderScan()` triggered a click on `<input type="file" webkitdirectory directory multiple>`. While desktop Chromium supports `webkitdirectory`, Android WebViews completely ignore directory-selection attributes on DOM file inputs.

### The Fix
- **Mobile-Aware Storage Discovery**: In `ui/js/modules/library.js`, detect mobile environments and call `scanLibrary()` directly. On Android, this triggers native `MediaStore` querying to discover all audio files on public storage (`Music/`, `Download/`, SD cards) without requiring manual folder picking.
- **Picker Fallback**: Provide an automated fallback to the multi-file audio input (`global-audio-import-input`) if explicit user selection is required.

---

## 4. Theme Button Overridden by System Dark/Light Mode

### Runtime Symptoms
- Switching themes (Dark / Light / System) in Settings failed to take effect, or the UI snapped back to the device's system appearance.
- Manual Light or Dark selections were ignored when the operating system was set to the opposite mode.

### Root Cause
1. **Malformed CSS Syntax**: In `ui/styles/tokens.css`, orphaned closing braces `}` and duplicate un-scoped variable definitions at the end of the file corrupted stylesheet parsing in the browser engine.
2. **Media Query Specificity**: `@media (prefers-color-scheme: light)` rules were overriding `:root[data-theme="dark"]` when the operating system was set to light theme.

### The Fix
- **Excise Corrupted Syntax**: Removed all orphaned braces and duplicate rules at the end of `ui/styles/tokens.css`.
- **Explicit Theme Precedence**: Structured theme rules so that `:root[data-theme="dark"]` and `:root[data-theme="light"]` take absolute precedence over `@media (prefers-color-scheme)` when manually configured by the user.
