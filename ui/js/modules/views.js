/**
 * Views Module
 * Handles loading, rendering, filtering and UI state transitions for all app views.
 */

let _lastPlayTriggerTime = 0;
let _lastPlayTrackId = null;
export function _safePlayTrack(trackId) {
    if (!trackId) return;
    const now = Date.now();
    if (_lastPlayTrackId === trackId && (now - _lastPlayTriggerTime < 350)) return;
    _lastPlayTriggerTime = now;
    _lastPlayTrackId = trackId;
    if (window.Auralis && window.Auralis.bridge && typeof window.Auralis.bridge.playTrack === 'function') {
        window.Auralis.bridge.playTrack(trackId);
    } else if (window.AuralisPlayer && typeof window.AuralisPlayer.playTrack === 'function') {
        window.AuralisPlayer.playTrack(trackId);
    }
}
try {
    window._safePlayTrack = _safePlayTrack;
} catch (_) {}

document.addEventListener('click', (evt) => {
    const playBtn = evt.target.closest && evt.target.closest('.play-shelf-btn, .play-track-btn');
    if (!playBtn) return;

    const trackId = playBtn.dataset.firstTrackId || playBtn.dataset.trackId;
    if (trackId) {
        _safePlayTrack(trackId);
    }
});

export const viewMethods = {
    refreshCurrentView() {
        const content = document.getElementById('content');
        if (!content) return;

        if (content.querySelector('.page-library')) {
            this.activeView = 'library';
            this.loadLibraryView();
        } else if (content.querySelector('.page-albums')) {
            this.activeView = 'albums';
            this.loadAlbumsView();
        } else if (content.querySelector('.page-artists')) {
            this.activeView = 'artists';
            this.loadArtistsView();
        } else if (content.querySelector('.page-downloads')) {
            this.activeView = 'downloads';
            this.loadDownloadView();
        } else if (content.querySelector('.page-search')) {
            this.activeView = 'search';
            this.loadSearchView();
        } else if (content.querySelector('.page-settings, #settings-view')) {
            this.activeView = 'settings';
            this.loadSettingsView();
        } else if (content.querySelector('.page-playlists')) {
            this.activeView = 'playlists';
            this.loadPlaylistsView();
        } else if (content.querySelector('.page-sync')) {
            this.activeView = 'sync';
            this.loadSyncView();
        } else {
            this.activeView = 'home';
            this.loadHomeView();
        }
    },

    async loadLibraryView() {
        const expected = 'library';
        if (this.activeView !== expected) return;

        const content = document.getElementById('content');
        if (!content || !content.querySelector('.page-library')) return;

        this.bindLibraryFilterControls();
        await this.renderLibraryTracks();
    },

    bindLibraryFilterControls() {
        const sortSelect = document.querySelector('.page-library select[name="sort_by"]');
        const downloadedCheckbox = document.querySelector('.page-library input[name="downloaded_only"]');

        if (sortSelect && !sortSelect.dataset.bound) {
            sortSelect.dataset.bound = 'true';
            sortSelect.addEventListener('change', () => {
                this.renderLibraryTracks();
            });
        }

        if (downloadedCheckbox && !downloadedCheckbox.dataset.bound) {
            downloadedCheckbox.dataset.bound = 'true';
            downloadedCheckbox.addEventListener('change', () => {
                this.renderLibraryTracks();
            });
        }
    },

    async renderLibraryTracks() {
        const trackList = document.querySelector('.page-library .track-list');
        if (!trackList) return;

        const sortSelect = document.querySelector('.page-library select[name="sort_by"]');
        const downloadedCheckbox = document.querySelector('.page-library input[name="downloaded_only"]');

        const sortBy = sortSelect ? sortSelect.value : 'date_added';
        const downloadedOnly = downloadedCheckbox ? downloadedCheckbox.checked : false;

        try {
            const html = await this.invoke('get_library_tracks_html', {
                sortBy,
                downloadedOnly,
                sort_by: sortBy,
                downloaded_only: downloadedOnly,
            });
            const list = document.querySelector('.page-library .track-list');
            if (list && html) {
                list.innerHTML = html;
                if (window.lucide) window.lucide.createIcons();
            }
        } catch (err) {
            console.error('Error rendering library tracks:', err);
        }
    },

    _syncPlayerBar() {
        // Re-sync mini-player bar with Rust after view navigation (fixes 01:39-01:44 desync)
        try {
            if (window.Auralis && window.Auralis.player && typeof window.Auralis.player.hydrateState === 'function') {
                window.Auralis.player.hydrateState().catch(()=>{});
            }
        } catch (_) {}
    },

    async loadHomeView() {
        const expected = 'home';
        if (this.activeView !== expected) return;
        try {
            const html = await this.invoke('get_home_shelves_html');
            if (this.activeView !== expected) return;
            const _content = document.getElementById('content');
            if (_content && _content.firstElementChild && !_content.querySelector('.page-home')) return;
            const container = document.getElementById('home-dynamic-content') || document.querySelector('.page-home');

            if (container && html) {
                container.innerHTML = html;
                if (window.lucide) window.lucide.createIcons();
                this._syncPlayerBar();
            }
        } catch (err) {
            console.error('Error loading home view:', err);
        }
    },

    async loadAlbumsView() {
        const expected = 'albums';
        if (this.activeView !== expected) return;

        try {
            const html = await this.invoke('get_albums_grid_html');
            if (this.activeView !== expected) return;
            const _content = document.getElementById('content');
            if (!_content || !_content.querySelector('.page-albums')) return;
            const grid = document.getElementById('albums-grid') || document.querySelector('.page-albums .grid');
            if (grid && html) {
                grid.innerHTML = html;
                if (window.lucide) window.lucide.createIcons();
            }
        } catch (err) {
            console.error('Error loading albums:', err);
        }
    },

    async loadArtistsView() {
        const expected = 'artists';
        if (this.activeView !== expected) return;

        try {
            const html = await this.invoke('get_artists_grid_html');
            if (this.activeView !== expected) return;
            const _content = document.getElementById('content');
            if (!_content || !_content.querySelector('.page-artists')) return;
            const grid = document.getElementById('artists-grid') || document.querySelector('.page-artists .grid');
            if (grid && html) {
                grid.innerHTML = html;
                if (window.lucide) window.lucide.createIcons();
            }
        } catch (err) {
            console.error('Error loading artists:', err);
        }
    },

    async loadPlaylistsView() {
        const expected = 'playlists';
        if (this.activeView !== expected) return;
        const grid = document.getElementById('playlists-grid') || document.querySelector('.page-playlists .grid');
        const detail = document.getElementById('playlist-detail');
        if (detail && detail.style.display !== 'block') {
            detail.style.display = 'none';
            if (grid) grid.style.display = 'grid';
        }
        if (!grid && !detail) return;

        try {
            const playlists = await this.invoke('get_playlists');
            if (this.activeView !== expected) return;
            const _content = document.getElementById('content');
            if (!_content || !_content.querySelector('.page-playlists')) return;
            if (playlists && playlists.length > 0) {
                grid.innerHTML = playlists.map(pl => {
                    const isSmart = Boolean(pl.is_smart);
                    let iconName = 'list-music';
                    let badgeHtml = '';
                    if (isSmart) {
                        const idStr = String(pl.id);
                        if (idStr === 'smart_favorites' || pl.name === 'Favorites') {
                            iconName = 'heart';
                            badgeHtml = `<span class="badge" style="background: rgba(239, 68, 68, 0.2); color: #f87171; border: 1px solid rgba(239, 68, 68, 0.3); font-size: var(--text-xs); padding: 2px 8px; border-radius: 999px;">Smart</span>`;
                        } else if (idStr === 'smart_recent' || pl.name === 'Recently Added') {
                            iconName = 'clock';
                            badgeHtml = `<span class="badge" style="background: rgba(59, 130, 246, 0.2); color: #60a5fa; border: 1px solid rgba(59, 130, 246, 0.3); font-size: var(--text-xs); padding: 2px 8px; border-radius: 999px;">Smart</span>`;
                        } else if (idStr === 'smart_most_played' || pl.name === 'Most Played') {
                            iconName = 'flame';
                            badgeHtml = `<span class="badge" style="background: rgba(245, 158, 11, 0.2); color: #fbbf24; border: 1px solid rgba(245, 158, 11, 0.3); font-size: var(--text-xs); padding: 2px 8px; border-radius: 999px;">Smart</span>`;
                        } else {
                            iconName = 'sparkles';
                            badgeHtml = `<span class="badge" style="background: rgba(168, 85, 247, 0.2); color: #c084fc; border: 1px solid rgba(168, 85, 247, 0.3); font-size: var(--text-xs); padding: 2px 8px; border-radius: 999px;">Smart</span>`;
                        }
                    }

                    return `
                    <div class="card playlist-card neu-glass" onclick="window.Auralis.bridge.openPlaylist('${pl.id}')" style="cursor: pointer;">
                        <div class="card-artwork"><i data-lucide="${iconName}"></i></div>
                        <div class="card-body">
                            <div style="display: flex; align-items: center; justify-content: space-between; gap: var(--space-2); margin-bottom: var(--space-1);">
                                <div class="card-title" style="margin-bottom: 0;">${this.escapeHtml(pl.name)}</div>
                                ${badgeHtml}
                            </div>
                            <div class="card-subtitle">${pl.track_ids ? pl.track_ids.length : 0} tracks</div>
                        </div>
                    </div>
                `;
                }).join('');
                if (window.lucide) window.lucide.createIcons();
            } else {
                grid.innerHTML = `
                    <div class="empty-state glass neu" style="grid-column: 1 / -1; padding: var(--space-8); text-align: center; border-radius: var(--radius-lg);">
                        <i data-lucide="list-music" style="width: 48px; height: 48px; color: var(--accent); margin-bottom: var(--space-3);"></i>
                        <h3 style="color: var(--text-1); font-size: var(--text-lg); margin-bottom: var(--space-2);">No playlists created</h3>
                        <p style="color: var(--text-3); font-size: var(--text-sm); margin-bottom: var(--space-4);">Create playlists to curate your favorite music collections.</p>
                        <button class="btn btn-primary neu" onclick="window.Auralis.bridge.promptCreatePlaylist()">
                            <i data-lucide="plus"></i>
                            Create First Playlist
                        </button>
                    </div>
                `;
                if (window.lucide) window.lucide.createIcons();
            }
        } catch (err) {
            console.error('Error loading playlists:', err);
        }
    },

    async openPlaylist(playlistId) {
        if (!playlistId) return;
        try {
            const result = await this.invoke('get_playlist', { id: playlistId });
            if (!result) {
                this.showToast('Playlist not found', 'warning');
                return;
            }
            const [playlist, tracks] = result;
            this.tracks = tracks || [];
            const detailEl = document.getElementById('playlist-detail');
            const gridEl = document.getElementById('playlists-grid') || document.querySelector('.page-playlists .grid');

            if (detailEl && gridEl) {
                gridEl.style.display = 'none';
                detailEl.style.display = 'block';

                const isSmart = Boolean(playlist.is_smart);
                let badge = isSmart ? `<span class="badge" style="background: rgba(168, 85, 247, 0.2); color: #c084fc; border: 1px solid rgba(168, 85, 247, 0.3); font-size: var(--text-xs); padding: 2px 8px; border-radius: 999px;">Smart Playlist</span>` : '';

                detailEl.innerHTML = `
                    <div style="display: flex; align-items: center; justify-content: space-between; margin-bottom: var(--space-4);">
                        <button class="btn btn-secondary btn-sm neu" onclick="window.Auralis.bridge.closePlaylistDetail()">
                            <i data-lucide="arrow-left"></i>
                            Back to Playlists
                        </button>
                        ${tracks && tracks.length > 0 ? `
                            <button class="btn btn-primary btn-sm neu" onclick="window.Auralis.bridge.playTrack('${tracks[0].id}')">
                                <i data-lucide="play"></i>
                                Play All (${tracks.length})
                            </button>
                        ` : ''}
                    </div>
                    <div class="card neu-glass" style="margin-bottom: var(--space-4); padding: var(--space-4);">
                        <div style="display: flex; align-items: center; gap: var(--space-2); margin-bottom: var(--space-1);">
                            <h3 style="font-size: var(--text-xl); font-weight: var(--font-bold); color: var(--text-1); margin: 0;">${this.escapeHtml(playlist.name)}</h3>
                            ${badge}
                        </div>
                        ${playlist.description ? `<p style="color: var(--text-3); font-size: var(--text-sm); margin: 0 0 var(--space-2) 0;">${this.escapeHtml(playlist.description)}</p>` : ''}
                        <div style="color: var(--text-3); font-size: var(--text-xs);">${tracks ? tracks.length : 0} tracks</div>
                    </div>
                    <div class="track-list" id="playlist-track-list">
                        <!-- Tracks -->
                    </div>
                `;

                const trackListEl = document.getElementById('playlist-track-list');
                if (tracks && tracks.length > 0) {
                    this.renderTrackRows(trackListEl, tracks);
                } else {
                    trackListEl.innerHTML = `
                        <div class="empty-state glass neu" style="padding: var(--space-6); text-align: center; border-radius: var(--radius-md);">
                            <i data-lucide="music" style="width: 32px; height: 32px; color: var(--accent); margin-bottom: var(--space-2);"></i>
                            <h4 style="color: var(--text-1); font-size: var(--text-base); margin-bottom: var(--space-1);">No tracks in playlist</h4>
                            <p style="color: var(--text-3); font-size: var(--text-xs);">Add tracks to this playlist from your library.</p>
                        </div>
                    `;
                }
                if (window.lucide) window.lucide.createIcons();
            }
        } catch (err) {
            console.error('Failed to open playlist:', err);
            this.showToast(`Failed to open playlist: ${err}`, 'error');
        }
    },

    closePlaylistDetail() {
        const detailEl = document.getElementById('playlist-detail');
        const gridEl = document.getElementById('playlists-grid') || document.querySelector('.page-playlists .grid');
        if (detailEl) detailEl.style.display = 'none';
        if (gridEl) gridEl.style.display = 'grid';
        if (window.lucide) window.lucide.createIcons();
    },

    async promptCreatePlaylist() {
        const name = prompt('Enter playlist name:');
        if (!name || !name.trim()) return;

        try {
            const created = await this.invoke('create_playlist', { request: { name: name.trim() } });
            if (created) {
                this.showToast(`Playlist "${created.name}" created!`, 'success');
                this.loadPlaylistsView();
            }
        } catch (err) {
            this.showToast(`Failed to create playlist: ${err}`, 'error');
        }
    },

    async loadSearchView() {
        const form = document.getElementById('search-form');
        const resultsContainer = document.getElementById('search-results');
        if (!form || !resultsContainer) return;

        const performSearch = async () => {
            const input = form.querySelector('input[name="q"]');
            const query = (input ? input.value : '').trim();

            try {
                const html = await this.invoke('get_search_results_html', { query, q: query });
                const target = document.getElementById('search-results');
                if (target && html) {
                    target.innerHTML = html;
                    if (window.lucide) window.lucide.createIcons();
                }
            } catch (err) {
                console.error('Search error:', err);
            }
        };

        if (form._searchHandler) form.removeEventListener('submit', form._searchHandler);
        form._searchHandler = (e) => { e.preventDefault(); performSearch(); };
        form.addEventListener('submit', form._searchHandler);
        form.dataset.bound = 'true';

        const searchInput = form.querySelector('input[name="q"]');
        if (searchInput) {
            if (searchInput._inputHandler) searchInput.removeEventListener('input', searchInput._inputHandler);
            let debounceTimer;
            searchInput._inputHandler = () => {
                clearTimeout(debounceTimer);
                debounceTimer = setTimeout(performSearch, 300);
            };
            searchInput.addEventListener('input', searchInput._inputHandler);
        }
    },

    async loadSettingsView() {
        const settingsView = document.querySelector('.page-settings, #settings-view');
        if (!settingsView) return;

        try {
            const settings = await this.invoke('get_settings');
            if (!settings || !document.querySelector('.page-settings, #settings-view')) return;

            this.currentSettings = {
                audio: {},
                downloads: {},
                sync: {},
                appearance: {},
                ...settings,
            };

            // 1. Populate values into controls
            const volInput = settingsView.querySelector('input[name="volume"]');
            const volBadge = settingsView.querySelector('#settings-volume-val');
            if (volInput && settings.audio) {
                const volPct = Math.round((settings.audio.volume ?? 0.8) * 100);
                volInput.value = volPct;
                if (volBadge) volBadge.textContent = `${volPct}%`;
            }

            const inputBindings = {
                download_path: settings.downloads?.download_path || '',
                default_format: String(settings.downloads?.default_format || 'mp3').toLowerCase(),
                youtube_cookie: settings.downloads?.youtube_cookie || '',
                youtube_po_token: settings.downloads?.youtube_po_token || '',
            };

            for (const [name, val] of Object.entries(inputBindings)) {
                const el = settingsView.querySelector(`[name="${name}"]`);
                if (el) el.value = val;
            }

            const toggleBindings = {
                use_system_downloads: settings.downloads?.use_system_downloads ?? true,
                sync_enabled: settings.sync?.enabled ?? true,
                sync_wifi_only: settings.sync?.wifi_only ?? false,
            };

            for (const [name, val] of Object.entries(toggleBindings)) {
                const el = settingsView.querySelector(`[name="${name}"], [data-name="${name}"]`);
                if (el) {
                    if (el.type === 'checkbox') {
                        el.checked = Boolean(val);
                    } else {
                        el.classList.toggle('active', Boolean(val));
                        el.setAttribute('aria-checked', Boolean(val).toString());
                    }
                }
            }

            if (window.AuralisYouTube && settings.downloads) {
                window.AuralisYouTube.setCredentials({
                    cookie: settings.downloads.youtube_cookie,
                    po_token: settings.downloads.youtube_po_token,
                });
            }

            const currentTheme = String(settings.appearance?.theme || 'dark').toLowerCase();
            const themeOptions = settingsView.querySelectorAll('.theme-option[data-theme]');
            themeOptions.forEach((opt) => {
                opt.classList.toggle('active', opt.dataset.theme === currentTheme);
            });

            if (settingsView.dataset.bound) {
                if (window.lucide) window.lucide.createIcons();
                return;
            }
            settingsView.dataset.bound = 'true';

            // 2. Save settings helper
            const saveSettings = async () => {
                if (!this.currentSettings) return;
                try {
                    await this.invoke('update_settings', { settings: this.currentSettings });
                    this.showToast('Settings saved', 'success');
                } catch (err) {
                    this.showToast(`Failed to save settings: ${err}`, 'error');
                }
            };

            // 3. Unified input / change listeners
            if (volInput) {
                volInput.addEventListener('input', (e) => {
                    if (volBadge) volBadge.textContent = `${e.target.value}%`;
                });
            }

            settingsView.addEventListener('change', async (e) => {
                const target = e.target;
                if (!target || !target.name) return;
                const name = target.name;
                const val = target.value;

                if (name === 'volume') {
                    this.currentSettings.audio.volume = parseFloat(val) / 100;
                } else if (name === 'download_path') {
                    this.currentSettings.downloads.download_path = val;
                } else if (name === 'default_format') {
                    this.currentSettings.downloads.default_format = val;
                } else if (name === 'youtube_cookie') {
                    this.currentSettings.downloads.youtube_cookie = val || null;
                } else if (name === 'youtube_po_token') {
                    this.currentSettings.downloads.youtube_po_token = val || null;
                }
                await saveSettings();
            });

            // 4. Unified click listener for switches and theme options
            settingsView.addEventListener('click', async (e) => {
                const themeBtn = e.target.closest && e.target.closest('.theme-option[data-theme]');
                if (themeBtn) {
                    e.preventDefault();
                    const selectedTheme = themeBtn.dataset.theme;
                    if (typeof this.setTheme === 'function') {
                        await this.setTheme(selectedTheme);
                    } else {
                        themeOptions.forEach((opt) => opt.classList.toggle('active', opt === themeBtn));
                        this.applyTheme(selectedTheme);
                        this.currentSettings.appearance.theme = selectedTheme;
                        await saveSettings();
                    }
                    return;
                }

                const toggle = e.target.closest && e.target.closest('.toggle[name], .toggle[data-name], [role="switch"]');
                if (toggle) {
                    const name = toggle.getAttribute('name') || toggle.getAttribute('data-name');
                    const newState = !toggle.classList.contains('active');
                    toggle.classList.toggle('active', newState);
                    toggle.setAttribute('aria-checked', newState.toString());

                    if (name === 'use_system_downloads') {
                        this.currentSettings.downloads.use_system_downloads = newState;
                    } else if (name === 'sync_enabled') {
                        this.currentSettings.sync.enabled = newState;
                    } else if (name === 'sync_wifi_only') {
                        this.currentSettings.sync.wifi_only = newState;
                    }
                    await saveSettings();
                }
            });

            if (window.lucide) window.lucide.createIcons();
        } catch (err) {
            console.error('Settings load error:', err);
        }
    },

    renderTrackRows(container, tracks) {
        container.innerHTML = tracks.map(track => {
            const isFav = Boolean(track.is_favorite);
            return `
            <div class="track-row neu-glass" data-track-id="${track.id}" data-role="play-row" style="cursor: pointer; margin-bottom: var(--space-2); border-radius: var(--radius-md); touch-action: manipulation;">
                <div class="track-row-artwork">
                    ${track.album_art_path ? this.artImgTag(track.album_art_path, track.title) : `<i data-lucide="music"></i>`}
                </div>
                <div class="track-row-info">
                    <div class="track-row-title">${this.escapeHtml(track.title)}</div>
                    <div class="track-row-subtitle">${this.escapeHtml(track.artist || 'Unknown Artist')} — ${this.escapeHtml(track.album || 'Single')}</div>
                </div>
                <span class="track-row-duration">${this.formatTime(track.duration_secs || 0)}</span>
                <div class="track-row-actions">
                    <button type="button" class="btn btn-ghost btn-icon play-track-btn" 
                            title="Play" data-role="play-btn" data-track-id="${track.id}" 
                            onclick="event.stopPropagation(); window._safePlayTrack ? window._safePlayTrack('${track.id}') : (window.Auralis && window.Auralis.bridge && window.Auralis.bridge.playTrack('${track.id}'))" 
                            ontouchend="event.stopPropagation(); window._safePlayTrack ? window._safePlayTrack('${track.id}') : (window.Auralis && window.Auralis.bridge && window.Auralis.bridge.playTrack('${track.id}'))">
                        <i data-lucide="play"></i>
                    </button>
                    <button type="button" class="btn btn-ghost btn-icon ${isFav ? 'liked' : ''}" style="${isFav ? 'color: var(--like);' : ''}" title="Like" onclick="event.stopPropagation(); window.Auralis.bridge.toggleTrackFavorite('${track.id}', this)" ontouchend="event.stopPropagation(); window.Auralis.bridge.toggleTrackFavorite('${track.id}', this)">
                        <i data-lucide="heart"></i>
                    </button>
                    <button type="button" class="btn btn-ghost btn-icon track-menu-btn" 
                            title="More options" data-role="menu-btn" data-track-id="${track.id}" 
                            onclick="event.stopPropagation(); window.Auralis && window.Auralis.bridge && window.Auralis.bridge.openTrackContextMenu(event, '${track.id}')" 
                            ontouchend="event.stopPropagation(); window.Auralis && window.Auralis.bridge && window.Auralis.bridge.openTrackContextMenu(event, '${track.id}')">
                        <i data-lucide="more-vertical"></i>
                    </button>
                </div>
            </div>
        `;
        }).join('');

        // Delegate row taps for mobile WebView reliability (fixes 01:40-01:44 swallowed taps)
        if (container && !container.dataset.bound) {
            container.dataset.bound = 'true';
            const rowHandler = (e) => {
                // Ignore clicks on action buttons handled separately
                if (e.target.closest && e.target.closest('.track-row-actions')) {
                    const playBtn = e.target.closest('[data-role="play-btn"]');
                    if (playBtn) {
                        e.preventDefault(); e.stopPropagation();
                        const tid = playBtn.dataset.trackId;
                        if (tid) _safePlayTrack(tid);
                    }
                    return;
                }
                const row = e.target.closest && e.target.closest('[data-role="play-row"]');
                if (!row || !container.contains(row)) return;
                e.preventDefault();
                const tid = row.dataset.trackId;
                if (tid) _safePlayTrack(tid);
            };
            container.addEventListener('click', rowHandler);
            container.addEventListener('touchend', rowHandler, { passive: false });
        }

        if (window.lucide) window.lucide.createIcons();
    },

    closeTrackContextMenu() {
        const existing = document.getElementById('floating-track-context-menu');
        if (existing) existing.remove();
        if (this._contextMenuOutsideListener) {
            document.removeEventListener('click', this._contextMenuOutsideListener);
            document.removeEventListener('touchend', this._contextMenuOutsideListener);
            document.removeEventListener('keydown', this._contextMenuKeyHandler);
            this._contextMenuOutsideListener = null;
            this._contextMenuKeyHandler = null;
        }
    },

    async openTrackContextMenu(event, trackId) {
        if (event) {
            event.preventDefault();
            event.stopPropagation();
        }
        this.closeTrackContextMenu();
        if (!trackId) return;

        let track = (this.tracks && this.tracks.find(t => String(t.id) === String(trackId)));
        if (!track) {
            try {
                track = await this.invoke('get_track', { id: trackId });
            } catch (_) {}
        }
        track = track || { id: trackId };

        const menu = document.createElement('div');
        menu.id = 'floating-track-context-menu';
        menu.className = 'context-menu';

        menu.innerHTML = `
            <button type="button" class="context-menu-item" data-action="play-next">
                <i data-lucide="play-circle"></i>
                <span>Play Next</span>
            </button>
            <button type="button" class="context-menu-item" data-action="add-queue">
                <i data-lucide="list-plus"></i>
                <span>Add to Queue</span>
            </button>
            <button type="button" class="context-menu-item" data-action="add-playlist">
                <i data-lucide="folder-plus"></i>
                <span>Add to Playlist</span>
            </button>
            <div class="context-menu-divider"></div>
            ${track.artist ? `
            <button type="button" class="context-menu-item" data-action="go-artist">
                <i data-lucide="user"></i>
                <span>Go to Artist</span>
            </button>` : ''}
            ${track.album ? `
            <button type="button" class="context-menu-item" data-action="go-album">
                <i data-lucide="disc"></i>
                <span>Go to Album</span>
            </button>` : ''}
            <button type="button" class="context-menu-item" data-action="edit-metadata">
                <i data-lucide="pencil"></i>
                <span>Edit Metadata</span>
            </button>
            <div class="context-menu-divider"></div>
            <button type="button" class="context-menu-item danger" data-action="delete-track">
                <i data-lucide="trash-2"></i>
                <span>Delete Track</span>
            </button>
        `;

        document.body.appendChild(menu);
        if (window.lucide) window.lucide.createIcons();

        // Calculate positioning
        const triggerEl = event.currentTarget || event.target;
        const rect = triggerEl && triggerEl.getBoundingClientRect ? triggerEl.getBoundingClientRect() : null;
        const clientX = (event.touches && event.touches[0] ? event.touches[0].clientX : event.clientX) || (rect ? rect.left : 100);
        const clientY = (event.touches && event.touches[0] ? event.touches[0].clientY : event.clientY) || (rect ? rect.bottom : 100);

        const menuRect = menu.getBoundingClientRect();
        const menuWidth = menuRect.width || 190;
        const menuHeight = menuRect.height || 260;

        let left = clientX;
        let top = clientY + 6;

        if (rect) {
            left = rect.right - menuWidth;
            top = rect.bottom + 4;
        }

        if (left + menuWidth > window.innerWidth - 8) {
            left = window.innerWidth - menuWidth - 8;
        }
        if (left < 8) left = 8;

        if (top + menuHeight > window.innerHeight - 8) {
            if (rect && rect.top - menuHeight > 8) {
                top = rect.top - menuHeight - 4;
            } else {
                top = window.innerHeight - menuHeight - 8;
            }
        }
        if (top < 8) top = 8;

        menu.style.left = `${Math.round(left)}px`;
        menu.style.top = `${Math.round(top)}px`;

        // Wire item actions
        menu.querySelector('[data-action="play-next"]')?.addEventListener('click', (e) => {
            e.stopPropagation();
            this.closeTrackContextMenu();
            this.playNextTrack(trackId);
        });

        menu.querySelector('[data-action="add-queue"]')?.addEventListener('click', (e) => {
            e.stopPropagation();
            this.closeTrackContextMenu();
            this.addToQueue(trackId);
        });

        menu.querySelector('[data-action="add-playlist"]')?.addEventListener('click', (e) => {
            e.stopPropagation();
            this.closeTrackContextMenu();
            this.openAddToPlaylistModal(trackId);
        });

        menu.querySelector('[data-action="go-artist"]')?.addEventListener('click', (e) => {
            e.stopPropagation();
            this.closeTrackContextMenu();
            this.goToArtist(track.artist);
        });

        menu.querySelector('[data-action="go-album"]')?.addEventListener('click', (e) => {
            e.stopPropagation();
            this.closeTrackContextMenu();
            this.goToAlbum(track.album);
        });

        menu.querySelector('[data-action="edit-metadata"]')?.addEventListener('click', (e) => {
            e.stopPropagation();
            this.closeTrackContextMenu();
            this.openTagEditor(track);
        });

        menu.querySelector('[data-action="delete-track"]')?.addEventListener('click', (e) => {
            e.stopPropagation();
            this.closeTrackContextMenu();
            this.deleteTracks([trackId]);
        });

        // Click outside & ESC listener
        this._contextMenuOutsideListener = (e) => {
            if (!menu.contains(e.target)) {
                this.closeTrackContextMenu();
            }
        };
        this._contextMenuKeyHandler = (e) => {
            if (e.key === 'Escape') {
                this.closeTrackContextMenu();
            }
        };

        setTimeout(() => {
            document.addEventListener('click', this._contextMenuOutsideListener);
            document.addEventListener('touchend', this._contextMenuOutsideListener);
            document.addEventListener('keydown', this._contextMenuKeyHandler);
        }, 20);
    },

    async openTagEditor(trackOrId) {
        let track = typeof trackOrId === 'object' ? trackOrId : null;
        const trackId = track ? track.id : trackOrId;
        if (!track && trackId) {
            track = (this.tracks && this.tracks.find(t => String(t.id) === String(trackId)));
            if (!track) {
                try {
                    track = await this.invoke('get_track', { id: trackId });
                } catch (_) {}
            }
        }
        if (!track) {
            this.showToast('Track not found', 'error');
            return;
        }

        const existingModal = document.getElementById('tag-editor-modal');
        if (existingModal) existingModal.remove();

        const backdrop = document.createElement('div');
        backdrop.id = 'tag-editor-modal';
        backdrop.className = 'modal-backdrop';

        backdrop.innerHTML = `
            <div class="modal-dialog glass neu" onclick="event.stopPropagation()">
                <div class="modal-header">
                    <h3 class="modal-title">Edit Metadata</h3>
                    <button type="button" class="modal-close" id="tag-editor-close" aria-label="Close">
                        <i data-lucide="x"></i>
                    </button>
                </div>
                <form id="tag-editor-form" class="modal-body" onsubmit="return false">
                    <div class="form-group">
                        <label class="form-label">Title</label>
                        <input type="text" name="title" class="input" value="${this.escapeHtml(track.title || '')}" required>
                    </div>
                    <div class="form-row">
                        <div class="form-group">
                            <label class="form-label">Artist</label>
                            <input type="text" name="artist" class="input" value="${this.escapeHtml(track.artist || '')}">
                        </div>
                        <div class="form-group">
                            <label class="form-label">Album Artist</label>
                            <input type="text" name="album_artist" class="input" value="${this.escapeHtml(track.album_artist || '')}">
                        </div>
                    </div>
                    <div class="form-group">
                        <label class="form-label">Album</label>
                        <input type="text" name="album" class="input" value="${this.escapeHtml(track.album || '')}">
                    </div>
                    <div class="form-row">
                        <div class="form-group">
                            <label class="form-label">Genre</label>
                            <input type="text" name="genre" class="input" value="${this.escapeHtml(track.genre || '')}">
                        </div>
                        <div class="form-group">
                            <label class="form-label">Year</label>
                            <input type="number" name="year" class="input" value="${track.year || ''}" placeholder="YYYY">
                        </div>
                    </div>
                    <div class="form-row">
                        <div class="form-group">
                            <label class="form-label">Track #</label>
                            <input type="number" name="track_number" class="input" value="${track.track_number || ''}">
                        </div>
                        <div class="form-group">
                            <label class="form-label">Disc #</label>
                            <input type="number" name="disc_number" class="input" value="${track.disc_number || ''}">
                        </div>
                    </div>
                    <div class="modal-footer">
                        <button type="button" class="btn btn-secondary neu" id="tag-editor-cancel">Cancel</button>
                        <button type="submit" class="btn btn-primary neu" id="tag-editor-save">
                            <i data-lucide="check"></i>
                            Save Changes
                        </button>
                    </div>
                </form>
            </div>
        `;

        document.body.appendChild(backdrop);
        if (window.lucide) window.lucide.createIcons();

        const closeModal = () => {
            backdrop.remove();
            document.removeEventListener('keydown', escListener);
        };

        const escListener = (e) => {
            if (e.key === 'Escape') closeModal();
        };
        document.addEventListener('keydown', escListener);

        backdrop.addEventListener('click', closeModal);
        backdrop.querySelector('#tag-editor-close')?.addEventListener('click', closeModal);
        backdrop.querySelector('#tag-editor-cancel')?.addEventListener('click', closeModal);

        const form = backdrop.querySelector('#tag-editor-form');
        form?.addEventListener('submit', async (e) => {
            e.preventDefault();
            const title = form.querySelector('input[name="title"]').value.trim();
            const artist = form.querySelector('input[name="artist"]').value.trim() || null;
            const albumArtist = form.querySelector('input[name="album_artist"]').value.trim() || null;
            const album = form.querySelector('input[name="album"]').value.trim() || null;
            const genre = form.querySelector('input[name="genre"]').value.trim() || null;
            const yearVal = form.querySelector('input[name="year"]').value.trim();
            const trackNumVal = form.querySelector('input[name="track_number"]').value.trim();
            const discNumVal = form.querySelector('input[name="disc_number"]').value.trim();

            const update = {
                title: title || track.title,
                artist,
                album,
                album_artist: albumArtist,
                genre,
                year: yearVal ? parseInt(yearVal, 10) : null,
                track_number: trackNumVal ? parseInt(trackNumVal, 10) : null,
                disc_number: discNumVal ? parseInt(discNumVal, 10) : null,
            };

            try {
                const updatedTrack = await this.invoke('update_track_metadata', { id: track.id, update });
                if (updatedTrack) {
                    const idx = this.tracks.findIndex(t => String(t.id) === String(track.id));
                    if (idx >= 0) this.tracks[idx] = updatedTrack;

                    if (window.Auralis && window.Auralis.player && window.Auralis.player.currentTrack && String(window.Auralis.player.currentTrack.id) === String(track.id)) {
                        Object.assign(window.Auralis.player.currentTrack, updatedTrack);
                        window.Auralis.player.updateFullScreenMetadata();
                        this.updatePlayerBar(updatedTrack);
                    }

                    this.showToast('Metadata updated successfully!', 'success');
                    closeModal();
                    this.refreshCurrentView();
                }
            } catch (err) {
                console.error('Failed to update track metadata:', err);
                this.showToast(`Failed to update metadata: ${err}`, 'error');
            }
        });
    },

    async openAddToPlaylistModal(trackId) {
        if (!trackId) return;

        let playlists = [];
        try {
            playlists = await this.invoke('get_playlists') || [];
        } catch (e) {
            console.warn('Failed to load playlists:', e);
        }

        const existingModal = document.getElementById('add-playlist-modal');
        if (existingModal) existingModal.remove();

        const backdrop = document.createElement('div');
        backdrop.id = 'add-playlist-modal';
        backdrop.className = 'modal-backdrop';

        backdrop.innerHTML = `
            <div class="modal-dialog glass neu" onclick="event.stopPropagation()">
                <div class="modal-header">
                    <h3 class="modal-title">Add to Playlist</h3>
                    <button type="button" class="modal-close" id="add-playlist-close" aria-label="Close">
                        <i data-lucide="x"></i>
                    </button>
                </div>
                <div class="modal-body">
                    <div style="display: flex; gap: var(--space-2); margin-bottom: var(--space-2);">
                        <input type="text" id="new-playlist-input" class="input" placeholder="Create new playlist..." style="flex: 1;">
                        <button type="button" class="btn btn-primary neu" id="new-playlist-create-btn">
                            <i data-lucide="plus"></i>
                            Create
                        </button>
                    </div>
                    <div class="form-label" style="margin-top: var(--space-2);">Your Playlists</div>
                    <div class="playlist-picker-list" id="playlist-picker-list">
                        ${playlists.length > 0 ? playlists.map(pl => `
                            <div class="playlist-picker-item" data-playlist-id="${pl.id}">
                                <div style="display: flex; align-items: center; gap: var(--space-2);">
                                    <i data-lucide="${pl.is_smart ? 'sparkles' : 'list-music'}" style="width: 18px; height: 18px; color: var(--accent);"></i>
                                    <span style="font-weight: var(--font-medium); color: var(--text-1);">${this.escapeHtml(pl.name)}</span>
                                </div>
                                <span style="font-size: var(--text-xs); color: var(--text-3);">${(pl.track_ids && pl.track_ids.length) || 0} tracks</span>
                            </div>
                        `).join('') : `
                            <div class="empty-state glass" style="padding: var(--space-4); text-align: center; border-radius: var(--radius-md);">
                                <p style="color: var(--text-3); font-size: var(--text-xs);">No custom playlists yet. Create one above!</p>
                            </div>
                        `}
                    </div>
                </div>
                <div class="modal-footer">
                    <button type="button" class="btn btn-secondary neu" id="add-playlist-cancel">Cancel</button>
                </div>
            </div>
        `;

        document.body.appendChild(backdrop);
        if (window.lucide) window.lucide.createIcons();

        const closeModal = () => {
            backdrop.remove();
            document.removeEventListener('keydown', escListener);
        };

        const escListener = (e) => {
            if (e.key === 'Escape') closeModal();
        };
        document.addEventListener('keydown', escListener);

        backdrop.addEventListener('click', closeModal);
        backdrop.querySelector('#add-playlist-close')?.addEventListener('click', closeModal);
        backdrop.querySelector('#add-playlist-cancel')?.addEventListener('click', closeModal);

        const addTrackToPl = async (playlistId, playlistName) => {
            try {
                await this.invoke('add_tracks_to_playlist', {
                    playlist_id: playlistId,
                    playlistId: playlistId,
                    track_ids: [trackId],
                    trackIds: [trackId],
                });
                this.showToast(`Added to "${playlistName}"`, 'success');
                closeModal();
            } catch (err) {
                console.error('Failed to add to playlist:', err);
                this.showToast(`Failed to add: ${err}`, 'error');
            }
        };

        backdrop.querySelectorAll('.playlist-picker-item').forEach(item => {
            item.addEventListener('click', () => {
                const plId = item.dataset.playlistId;
                const plName = item.querySelector('span')?.textContent || 'Playlist';
                addTrackToPl(plId, plName);
            });
        });

        const newPlInput = backdrop.querySelector('#new-playlist-input');
        const createBtn = backdrop.querySelector('#new-playlist-create-btn');
        const handleCreate = async () => {
            const name = newPlInput?.value.trim();
            if (!name) return;
            try {
                const created = await this.invoke('create_playlist', { request: { name } });
                if (created && created.id) {
                    await addTrackToPl(created.id, created.name);
                }
            } catch (err) {
                this.showToast(`Failed to create playlist: ${err}`, 'error');
            }
        };

        createBtn?.addEventListener('click', handleCreate);
        newPlInput?.addEventListener('keydown', (e) => {
            if (e.key === 'Enter') handleCreate();
        });
    },

    async goToArtist(artistName) {
        if (!artistName || artistName === 'Unknown Artist') {
            this.showToast('No artist information available', 'info');
            return;
        }
        try {
            const html = await this.invoke('get_library_tracks_html', { artist: artistName });
            const content = document.getElementById('content');
            if (content) {
                content.innerHTML = `
                    <section class="page-library">
                        <header class="section-header">
                            <div>
                                <button class="btn btn-secondary btn-sm neu" onclick="window.Auralis.bridge.loadLibraryView()" style="margin-bottom: var(--space-2);">
                                    <i data-lucide="arrow-left"></i> All Tracks
                                </button>
                                <h2 class="section-title">Artist: ${this.escapeHtml(artistName)}</h2>
                            </div>
                        </header>
                        <div class="track-list">${html || ''}</div>
                    </section>
                `;
                if (window.lucide) window.lucide.createIcons();
            }
        } catch (err) {
            console.error('Failed to filter by artist:', err);
        }
    },

    async goToAlbum(albumName) {
        if (!albumName || albumName === 'Unknown Album' || albumName === 'Single') {
            this.showToast('No album information available', 'info');
            return;
        }
        try {
            const html = await this.invoke('get_library_tracks_html', { album: albumName });
            const content = document.getElementById('content');
            if (content) {
                content.innerHTML = `
                    <section class="page-library">
                        <header class="section-header">
                            <div>
                                <button class="btn btn-secondary btn-sm neu" onclick="window.Auralis.bridge.loadLibraryView()" style="margin-bottom: var(--space-2);">
                                    <i data-lucide="arrow-left"></i> All Tracks
                                </button>
                                <h2 class="section-title">Album: ${this.escapeHtml(albumName)}</h2>
                            </div>
                        </header>
                        <div class="track-list">${html || ''}</div>
                    </section>
                `;
                if (window.lucide) window.lucide.createIcons();
            }
        } catch (err) {
            console.error('Failed to filter by album:', err);
        }
    },

    async deleteTracks(trackIds) {
        if (!trackIds || !trackIds.length) return;
        const confirmMsg = trackIds.length === 1
            ? 'Are you sure you want to delete this track from your library?'
            : `Are you sure you want to delete these ${trackIds.length} tracks from your library?`;
        if (!confirm(confirmMsg)) return;

        try {
            await this.invoke('delete_tracks', { ids: trackIds });
            this.showToast('Track deleted from library', 'info');
            const idSet = new Set(trackIds.map(String));
            this.tracks = this.tracks.filter(t => !idSet.has(String(t.id)));
            this.refreshCurrentView();
        } catch (err) {
            console.error('Failed to delete tracks:', err);
            this.showToast(`Failed to delete track: ${err}`, 'error');
        }
    }
};
