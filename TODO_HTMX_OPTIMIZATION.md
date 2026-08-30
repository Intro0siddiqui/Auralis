# ⚡ Auralis Frontend: JavaScript-to-HTMX & Rust Refactoring Roadmap

This document outlines the strategy for reducing excessive client-side JavaScript by offloading catalog rendering, sorting, filtering, and state binding to native Rust SQLite queries and server-rendered HTMX partials.

---

## 📊 Summary of JavaScript Redundancy Audit

- **Current Footprint:** ~5,200 LOC (~235 KB) of JavaScript across `ui/js/`.
- **Target Reduction:** Eliminate **~1,200 to 1,500 LOC (~60% of UI JS logic)**.
- **Goal:** Replace heavy DOM string concatenation (`.innerHTML = ...`), browser memory caching (`this.tracks`, `this._albumMap`), and client-side array sorting with instant, native server-rendered HTMX partials.

---

## 🎯 Phase 1: High-Impact View Offloading (Catalog & Lists)
> **Goal:** Stop transferring massive JSON arrays to browser memory; deliver pre-rendered HTML cards and rows directly from SQLite.

- [ ] **1. Albums & Artists Grid Generation (`views.js:251-377`)**
  - **Current Issue:** Pulls all tracks into JS memory, runs nested `Map` loops, computes counts, and generates HTML strings.
  - **Refactoring:**
    - [ ] Add SQL aggregation queries in Rust:
      ```sql
      -- Albums
      SELECT album, artist, album_art_path, COUNT(*) as track_count, MIN(id) as first_track_id 
      FROM tracks WHERE album IS NOT NULL GROUP BY album ORDER BY album COLLATE NOCASE ASC;
      -- Artists
      SELECT artist, COUNT(*) as track_count, MIN(id) as first_track_id 
      FROM tracks WHERE artist IS NOT NULL GROUP BY artist ORDER BY artist COLLATE NOCASE ASC;
      ```
    - [ ] Expose Rust endpoints `/partials/albums-grid` and `/partials/artists-grid`.
    - [ ] Update `albums.html` and `artists.html` with HTMX triggers:
      ```html
      <div id="albums-grid" class="grid grid-auto" hx-get="/partials/albums-grid" hx-trigger="load"></div>
      <div id="artists-grid" class="grid grid-auto" hx-get="/partials/artists-grid" hx-trigger="load"></div>
      ```
    - [ ] Remove `loadAlbumsView`, `loadArtistsView`, and `this._albumMap` / `this._artistMap` from `views.js` (~130 LOC removed).

- [ ] **2. Library Track Sorting, Filtering & Row Generation (`views.js:69-162`, `799-852`)**
  - **Current Issue:** Sorts in-memory JS arrays with `Array.prototype.sort()` and filters `is_downloaded` client-side.
  - **Refactoring:**
    - [ ] Use HTMX standard input attributes in `library.html`:
      ```html
      <div class="filter-bar">
          <select name="sort_by" class="input" 
                  hx-get="/partials/library/tracks" 
                  hx-target="#library-track-list" 
                  hx-include="[name='downloaded_only']">
              <option value="date_added">Date added</option>
              <option value="title">Title</option>
              <option value="artist">Artist</option>
              <option value="album">Album</option>
          </select>
          <label class="checkbox">
              <input type="checkbox" name="downloaded_only" 
                     hx-get="/partials/library/tracks" 
                     hx-target="#library-track-list" 
                     hx-include="[name='sort_by']" />
              Downloaded only
          </label>
      </div>
      <div id="library-track-list" class="track-list" 
           hx-get="/partials/library/tracks" 
           hx-trigger="load, library:refresh from:body">
      </div>
      ```
    - [ ] Remove `renderLibraryTracks`, `renderTrackRows`, and client sorting from `views.js` (~150 LOC removed).

- [ ] **3. Home Shelves & Recommendations (`views.js:173-250`)**
  - **Current Issue:** Slices tracks client-side (`tracks.slice(0, 6)`) and injects shelf cards via JS template literals.
  - **Refactoring:**
    - [ ] Render `home.html` directly from Rust with pre-populated shelves:
      - "Recently Added" (`SELECT * FROM tracks ORDER BY created_at DESC LIMIT 6`)
      - "Continue Listening" (`SELECT * FROM tracks WHERE last_played IS NOT NULL ORDER BY last_played DESC LIMIT 6`)
    - [ ] Remove `loadHomeView` from `views.js` (~80 LOC removed).

- [ ] **4. In-Library Search Debouncing (`views.js:539-584`)**
  - **Current Issue:** Manual JS `setTimeout` debounce timers and client-side empty state injection.
  - **Refactoring:**
    - [ ] Replace with declarative HTMX debounced search in `search.html`:
      ```html
      <input type="search" name="q" class="input" placeholder="Search title, artist, album..."
             hx-get="/partials/search/results"
             hx-trigger="input changed delay:300ms, search"
             hx-target="#search-results">
      <div id="search-results" class="track-list"></div>
      ```
    - [ ] Remove `loadSearchView` from `views.js` (~45 LOC removed).

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
