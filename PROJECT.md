# Auralis — Project Knowledge & Vision

> [!IMPORTANT]
> **Developer & Agent Notice**: This document serves as the high-level product vision and architectural reference. For active development rules, strict coding conventions, CI/CD specifications, and live investigation logs, refer directly to [**`AGENTS.md`**](file:///home/intro/spectre-enviroment/Auralis/AGENTS.md).

---

## 1. What Auralis Is

Auralis is a modern, lightweight, self-hosted music player and streaming client built for **Desktop (Linux, macOS, Windows)** and **Mobile (Android-first)**.

### Core Philosophy
1. **Thick Rust Core, Thin UI**: All heavy lifting—audio decoding, database persistence, streaming downloads, background services, and P2P synchronization—runs in compiled native Rust. The frontend is a lightweight, zero-framework presentation layer using **HTML5, CSS, and HTMX 1.9** (no React, Vue, or Electron bloat).
2. **Zero Binary Sidecars**: No `yt-dlp`, `ffmpeg`, or `rusty_ytdl` executables are bundled. YouTube URL resolution runs directly in the frontend via InnerTube (`youtube.js`), and audio bytes are streamed natively via `reqwest`.
3. **Local-First & E2EE by Default**: Media libraries, playlists, favorites, and settings reside in a local SQLite database with optional End-to-End Encrypted (E2EE) device-to-device synchronization.
4. **Soft Glass Audio Design**: Built on modern Glassmorphism (`backdrop-filter`) and Neumorphic tactile UI tokens with dynamic dark/light mode system adaptation.

---

## 2. Core Architecture & Subsystems

```
src/
├── domain/           # Pure business logic — models, repository traits, services
├── infrastructure/   # Concrete implementations
│   ├── database/     # SQLite persistence via rusqlite (repositories)
│   ├── filesystem/   # Audio scanner and tag extraction via lofty
│   └── media/        # rodio audio player, reqwest downloader, loopback StreamProxy
├── commands/         # Tauri IPC command bridge (UI <-> Rust)
├── templates/        # Static HTMX partial server
└── lib.rs            # App setup, Tauri state management, JNI lifecycle
```

### Key Subsystems
* **Audio Playback Engine**: Powered by `rodio 0.22`, featuring sample-accurate seeking, queue auto-advance with 250ms progress events, Opus SIMD decoders, and Android foreground `MediaPlaybackService` with lockscreen controls.
* **Streaming Proxy (`StreamProxy`)**: Runs on local loopback (`http://127.0.0.1:<port>/stream/<id>`), bridging HTML5 `<audio>` elements to YouTube CDN streams with client-matched headers (`User-Agent`, `Referer`, `Origin`) for direct live playback with zero disk writes.
* **Dual-Save Downloader**: Downloads audio streams via `reqwest` with HTTP Range pause/resume and atomic rename. Saves internally to `app_data_dir/downloads` and publishes visible copies to `Download/Auralis/` via Android `MediaStore`.
* **P2P Sync Engine**: Migrating from `libp2p` to `Iroh` + `Iroh-Blobs` for seamless QUIC-based device pairing via QR code `EndpointTicket` and BLAKE3 verified chunked audio file transfer.

---

## 3. Reference Pointers to `AGENTS.md`

To keep context windows lean, consult specific sections in [`AGENTS.md`](file:///home/intro/spectre-enviroment/Auralis/AGENTS.md):

| Topic | Relevant Section in `AGENTS.md` |
|---|---|
| **Layer Architecture & Conventions** | [`AGENTS.md §2 (Architecture & Key Conventions)`](file:///home/intro/spectre-enviroment/Auralis/AGENTS.md#2-architecture) |
| **Evidence Rule & Debugging Lessons** | [`AGENTS.md §2 (Evidence Rule)`](file:///home/intro/spectre-enviroment/Auralis/AGENTS.md#evidence-rule-a-working-example-outranks-a-theory) |
| **Implementation Roadmap & Milestones**| [`AGENTS.md §3 (Roadmap)`](file:///home/intro/spectre-enviroment/Auralis/AGENTS.md#3-implementation-roadmap) |
| **Download Pipeline & YouTube Resolver** | [`AGENTS.md §4.5 (Downloads) & §4.6 (YouTube Resolver)`](file:///home/intro/spectre-enviroment/Auralis/AGENTS.md#45-downloads--where-files-live-v2518-dual-save-since-v2511) |
| **CI/CD Pipeline & Android Builds** | [`AGENTS.md §7 (CI/CD Pipeline)`](file:///home/intro/spectre-enviroment/Auralis/AGENTS.md#7-cicd-pipeline) |
| **Code Style & Formatting Guidelines** | [`AGENTS.md §6 (Code Style)`](file:///home/intro/spectre-enviroment/Auralis/AGENTS.md#6-code-style) |
