# Auralis v2 - Development Progress Report

## Project Overview

Auralis v2 is a lightweight, offline-first music player application built with Rust and Tauri, featuring a modern HTMX-based frontend with the **Soft Glass Audio** design system (glassmorphism + neumorphism).

## Technology Stack

- **Backend**: Rust with Clean Architecture
- **Frontend**: HTMX + Vanilla HTML/CSS (Soft Glass Audio design)
- **Desktop Runtime**: Tauri v2 (WebKitGTK on Linux)
- **Mobile Runtime**: Tauri Android v2
- **Database**: SQLite via rusqlite
- **Audio Playback**: rodio
- **P2P Networking**: libp2p (mDNS, gossipsub, request-response)
- **Design**: Glassmorphism + Neumorphism hybrid

## Component Status

### Backend (Rust)

| Component | Status | Notes |
|-----------|--------|-------|
| Database layer | ✅ Complete | SQLite via rusqlite, 871 lines of SQL |
| Library scanner | ✅ Complete | Glob + lofty metadata extraction |
| Audio player | ✅ Complete | rodio with queue/shuffle/repeat/seek |
| Download pipeline | ✅ Complete | yt-dlp subprocess with progress |
| Playlist CRUD | ✅ Complete | Full SQLite persistence |
| Settings | ✅ Complete | SQLite-backed load/save |
| P2P networking | ✅ Complete | libp2p: mDNS, gossipsub, request-response |
| Sync service | ⚠️ Partial | DB persistence works; transfers simulated |
| Library service scanner | ⚠️ Stub | Use infrastructure/scanner.rs instead |

### Frontend (Soft Glass Audio)

| Component | Status | Notes |
|-----------|--------|-------|
| App shell | ✅ Complete | Sidebar + content + player bar + mobile nav |
| Design system | ✅ Complete | tokens.css, base.css, components.css, responsive.css |
| HTMX partials | ✅ Complete | 9 partials (nav, home, library, albums, artists, playlists, player-full, sync, settings) |
| JS bridge | ✅ Complete | Tauri event listeners + player bar updates |
| JS player | ✅ Complete | Progress/seek + keyboard shortcuts |
| Responsive | ✅ Complete | Mobile/tablet/desktop breakpoints |

### Infrastructure

| Component | Status | Notes |
|-----------|--------|-------|
| CI/CD | ✅ Complete | Linux, macOS, Windows, Android builds |
| Auto-release | ✅ Complete | Tag push creates release with all artifacts |
| Android signing | ✅ Complete | Auto-generated keystore, V1+V2 signing |

## Features Implemented

- Audio playback with queue, shuffle, repeat, seek, volume
- Media downloading via yt-dlp with progress tracking
- Library scanning with metadata extraction (lofty)
- Playlist CRUD with SQLite persistence
- Settings with SQLite persistence
- P2P device discovery via mDNS
- QR code + PIN pairing
- Sync service with DB persistence (transfers simulated)

## Known Limitations

- **P2P data transfer** is simulated (uses `tokio::time::sleep`)
- **Library service scanner** is stubbed (infrastructure scanner works)
- **Smart playlists** lack built-in "Recently Added" / "Most Played"
- **Android icons** — no PNG mipmaps (only 32x32.png and icon.ico)

## UI/UX Design (Soft Glass Audio)

- **Color Scheme**: Dark mode default with light mode option
- **Primary Color**: Cyan accent (#6ee7ff) with violet secondary (#a78bfa)
- **Background**: Rich charcoal (#070b10) dark, Off-white (#FAFAFA) light
- **Glass Effects**: `.glass`, `.glass-weak`, `.glass-strong` with `backdrop-filter: blur()`
- **Neu Effects**: `.neu`, `.neu-inset`, `.neu-glass` with dual box-shadows
- **Layout**: Sidebar + Main Content + Now Playing Bar
- **Icons**: Lucide icons (SVG)
- **Responsive**: Mobile-first with collapsing sidebar

## Build Status

- **Compilation**: ✅ 0 errors, 0 warnings (in CI)
- **CI/CD**: ✅ All platforms build successfully
- **Android**: ✅ Signed APK produced

---

**Status**: Active Development — Core features complete; remaining work on real P2P transfer and polish.
