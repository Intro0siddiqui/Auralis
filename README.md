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

| Platform | Format | Link |
|----------|--------|------|
| Linux (any distro) | .tar.gz | [Download](https://github.com/Intro0siddiqui/Auralis/releases/download/v1.0.0/Auralis-linux-x86_64.tar.gz) |
| Debian/Ubuntu | .deb | [Download](https://github.com/Intro0siddiqui/Auralis/releases/download/v1.0.0/auralis_1.0.0-1_amd64.deb) |
| Fedora | .rpm | [Download](https://github.com/Intro0siddiqui/Auralis/releases/download/v1.0.0/auralis-1.0.0-1.x86_64.rpm) |
| macOS | .dmg | [Download](https://github.com/Intro0siddiqui/Auralis/releases/download/v1.0.0/Auralis-1.0.0.dmg) |
| Android | .apk | [Download](https://github.com/Intro0siddiqui/Auralis/releases/download/v1.0.0/android-release-unsigned.apk) |

## Install

### Linux (portable)

```bash
tar xzf Auralis-linux-x86_64.tar.gz
cd Auralis && ./bin/Auralis
```

Or install to `~/.local/bin` and app launcher:

```bash
./install.sh
```

### Linux (Debian/Ubuntu)

```bash
sudo dpkg -i auralis_1.0.0-1_amd64.deb
```

### Linux (Fedora)

```bash
sudo rpm -i auralis-1.0.0-1.x86_64.rpm
```

### macOS

Open the `.dmg` and drag Auralis to Applications.

### Android

Enable "Install from unknown sources" and install the APK.

## Build from Source

Requirements: JDK 17+, Android SDK (for Android builds)

```bash
git clone https://github.com/Intro0siddiqui/Auralis.git
cd Auralis/auralis

# Run desktop app
./gradlew :desktop:run

# Build packages
./gradlew :desktop:packageDeb       # Linux .deb
./gradlew :desktop:packageRpm       # Linux .rpm
./gradlew :desktop:packageDmg       # macOS .dmg
./gradlew :desktop:packageAppImage  # Portable Linux binary
./gradlew :android:assembleDebug    # Android APK
```

## Requirements

- **Desktop**: Java 17+ (not needed for the portable .tar.gz build)
- **Download feature**: [yt-dlp](https://github.com/yt-dlp/yt-dlp) and [ffmpeg](https://ffmpeg.org/) must be installed and in your PATH
- **Android**: Android 8.0+

## Storage

All data is stored in `~/.auralis/` (desktop) or app-private storage (Android).

## Tech Stack

- Kotlin Multiplatform + Compose Multiplatform
- SQLDelight (SQLite)
- ExoPlayer (Android), javax.sound (Desktop)
- yt-dlp for media downloading

## License

MIT
