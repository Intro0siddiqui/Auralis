# ⚡ Auralis Frontend: JavaScript-to-HTMX & Rust Refactoring Roadmap

This document outlines the strategy for reducing excessive client-side JavaScript by offloading catalog rendering, sorting, filtering, and state binding to native Rust SQLite queries and server-rendered HTMX partials.

---

## 📊 Summary of JavaScript Redundancy Audit

- **Current Footprint:** ~5,200 LOC (~235 KB) of JavaScript across `ui/js/`.
- **Target Reduction:** Eliminate **~1,200 to 1,500 LOC (~60% of UI JS logic)**.
- **Goal:** Replace heavy DOM string concatenation (`.innerHTML = ...`), browser memory caching (`this.tracks`, `this._albumMap`), and client-side array sorting with instant, native server-rendered HTMX partials.

---

## 🎯 Phase 1: High-Impact View Offloading (Catalog & Lists) — ✅ COMPLETED
> **Goal:** Stop transferring massive JSON arrays to browser memory; deliver pre-rendered HTML cards and rows directly from SQLite.

- [x] **1. Albums & Artists Grid Generation (`views.js`, `commands/library.rs`)**
  - **Accomplished:**
    - [x] Added SQLite aggregation queries in Rust (`get_albums_grid_html` and `get_artists_grid_html`).
    - [x] Removed `this._albumMap` and `this._artistMap` memory caches from `views.js`.
    - [x] Converted `loadAlbumsView` and `loadArtistsView` to declarative single-call HTML rendering.

- [x] **2. Library Track Sorting, Filtering & Row Generation (`views.js`, `commands/library.rs`)**
  - **Accomplished:**
    - [x] Added `get_library_tracks_html` backend command supporting `sort_by`, `downloaded_only`, `artist`, `album`, and `search`.
    - [x] Removed client-side `Array.prototype.sort()` and manual DOM row generation from `views.js`.
    - [x] Connected filter bar controls directly to backend queries.

- [x] **3. Home Shelves & Recommendations (`views.js`, `commands/library.rs`, `home.html`)**
  - **Accomplished:**
    - [x] Added `get_home_shelves_html` command in Rust: pre-renders "Recently Added" shelf cards and "Continue Listening" rows directly from SQLite.
    - [x] Removed client-side slicing and manual card/row construction from `views.js`.

- [x] **4. In-Library Search Debouncing (`views.js`, `commands/library.rs`, `search.html`)**
  - **Accomplished:**
    - [x] Added `get_search_results_html` backend command.
    - [x] Refactored `loadSearchView` in `views.js` to render search results via backend query.


---

## 🛠️ Phase 2: Form & State Binding Simplification
> **Goal:** Replace manual input listeners and JSON getters with standard HTMX form posts.

- [ ] **1. Settings Form Hydration & Live Binding (`views.js:586-797`)**
  - **Current Issue:** 210+ lines of JS querying individual inputs, reading `get_settings`, setting values, and attaching individual `change` listeners.
  - **Refactoring:**
    - [ ] Serve `/partials/settings` with current settings pre-populated in inputs.
    - [ ] Bind toggles and inputs via HTMX triggers:
      ```html
      <input type="range" name="volume" value="70" 
             hx-post="/commands/settings/volume" 
             hx-trigger="change" />
      ```
    - [ ] Remove manual listeners from `views.js` (~210 LOC removed).

- [ ] **2. Queue Panel Rendering (`player.js:949-1019`)**
  - **Current Issue:** Every queue update triggers client-side DOM building for now-playing and upcoming tracks.
  - **Refactoring:**
    - [ ] Make `#queue-panel` an HTMX partial endpoint `/partials/queue` triggered by the `playback:queue` event:
      ```html
      <aside id="queue-panel" class="glass queue-panel" 
             hx-get="/partials/queue" 
             hx-trigger="playback:queue from:body">
      </aside>
      ```
    - [ ] Remove `renderQueuePanel` and `renderQueueTrackRow` from `player.js` (~70 LOC removed).

- [ ] **3. P2P Sync Devices View (`downloads.js:413-467`)**
  - **Current Issue:** Client JS creates Base64 QR image tags and loops over paired devices.
  - **Refactoring:**
    - [ ] Pre-render QR code SVG/PNG and paired devices list directly in Rust server partial `sync.html`.
    - [ ] Remove `loadSyncView` from `downloads.js` (~55 LOC removed).

---

## 🧹 Phase 3: DOM Listeners & Memory Cleanup
> **Goal:** Eliminate redundant listeners, MutationObservers, and duplicate memory caches.

- [ ] **1. Eliminate `MutationObserver` in Full-Screen Modal (`player.js:207-225`)**
  - **Current Issue:** Watches `#overlay-root` for modal insertion to wire buttons.
  - **Refactoring:** Keep modal permanently in DOM and toggle visibility via CSS class (`.open`), or use `htmx:afterSwap` listener.
- [ ] **2. Consolidate Event Delegation (`views.js:24-32`, `200-211`, `284-298`, `828-849`)**
  - **Current Issue:** Attaches duplicate `click` and `touchend` handlers across multiple views.
  - **Refactoring:** Rely exclusively on global event delegation in `core.js` for `[data-role="play-row"]` and `[data-role="play-card"]`.
- [ ] **3. Clean Up Bridge Memory Cache**
  - **Refactoring:** Remove `this.tracks`, `this._albumMap`, `this._artistMap`, and `this.currentSettings` from `Bridge` state, relying purely on SQLite persistence.

---

## 🛡️ Areas Where JavaScript MUST Be Preserved (Do Not Refactor)

1. **YouTube InnerTube Resolver & BotGuard (`youtube.js`, `po_token.js`):** Deciphers audio streams and mints PO-tokens in-browser without external binary sidecars (`yt-dlp`/`ffmpeg`).
2. **High-Frequency 250ms Audio Scrubbing (`player.js`):** Smooth drag-scrubbing on `<div class="progress-track">` requires instant local DOM updates to avoid IPC latency.
3. **Web Audio & MediaSession API (`player.js`):** Browser platform API for OS hardware keys and lockscreen controls.
4. **Android SAF Scoped Storage Ingestion (`library.js`):** Reading local files as base64 byte chunks via `FileReader` across Tauri IPC.
5. **Keyboard Shortcuts (`player.js`):** Local keydown interception for `Space` (Play/Pause), `ArrowLeft`/`ArrowRight` (Seek), `KeyS` (Shuffle), `KeyR` (Repeat).
