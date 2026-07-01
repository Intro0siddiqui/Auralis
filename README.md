# Auralis

Offline music player with YouTube/Instagram audio download and P2P sync between devices.

## Features

- **Media Download** — Download audio from YouTube and Instagram in MP3, FLAC, AAC, OGG, WAV, or M4A
- **Music Player** — Full playback with queue, repeat, shuffle, and background play
- **Local Import** — Scan any folder on your device for music files
- **P2P Sync** — Sync your library between devices over LAN via QR code or 6-digit temp code
- **Playlists** — Create and manage playlists
- **Metadata Editing** — Edit track info and organize your library

## Download

| Platform | Link |
|----------|------|
| Linux x86_64 | [Download](https://github.com/Intro0siddiqui/Auralis/releases/download/v1.0.0/Auralis-linux-x86_64.tar.gz) |
| macOS | [Download](https://github.com/Intro0siddiqui/Auralis/releases/download/v1.0.0/Auralis-mac-arm64.tar.gz) |
| Android | [Download](https://github.com/Intro0siddiqui/Auralis/releases/download/v1.0.0/android-debug.apk) |

All desktop builds use the same format: portable binary with bundled JRE. Extract and run.

## Install

### Linux / macOS

```bash
tar xzf Auralis-*.tar.gz
cd Auralis && ./bin/Auralis
```

### Android

Enable "Install from unknown sources" and install the APK.

## Build from Source

Requirements: JDK 17+, Android SDK (for Android builds)

```bash
git clone https://github.com/Intro0siddiqui/Auralis.git
cd Auralis/auralis

./gradlew :desktop:run              # Run desktop app
./gradlew :desktop:packageAppImage  # Build portable binary
./gradlew :android:assembleDebug    # Build Android APK
```

## Requirements

- **Desktop**: No Java needed (bundled in the download)
- **Download feature**: [yt-dlp](https://github.com/yt-dlp/yt-dlp) and [ffmpeg](https://ffmpeg.org/) must be installed and in your PATH
- **Android**: Android 8.0+

## Tech Stack

- Kotlin Multiplatform + Compose Multiplatform
- SQLDelight (SQLite)
- ExoPlayer (Android), javax.sound (Desktop)
- yt-dlp for media downloading

## License

MIT
