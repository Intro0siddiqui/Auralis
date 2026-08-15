class Bridge {
    constructor() {
        this.listeners = {};
        this.tauriAvailable = false;
        this.tracks = [];
        this.activeView = 'home';
        this.init();
    }

    async init() {
        this.initTheme();
        try {
            if (window.__TAURI_INTERNALS__) {
                const { listen } = window.__TAURI_INTERNALS__.tauri;
                this.tauriAvailable = true;

                await listen('playback:state_changed', (event) => {
                    this.emit('playback:state', event.payload);
                });

                await listen('playback:track_changed', (event) => {
                    this.emit('playback:track', event.payload);
                    this.updatePlayerBar(event.payload);
                });

                await listen('playback:queue_updated', (event) => {
                    this.emit('playback:queue', event.payload);
                });

                await listen('download:progress', (event) => {
                    this.emit('download:progress', event.payload);
                    this.updateDownloadProgressUI(event.payload);
                });

                await listen('download:completed', (event) => {
                    this.emit('download:completed', event.payload);
                    this.showToast('Download complete! Refreshing library...', 'success');
                    this.scanLibrary();
                });

                await listen('library:scan_complete', (event) => {
                    this.emit('library:scan', event.payload);
                    this.showToast(`Library scan complete: ${event.payload.tracks_added} new tracks`, 'info');
                    this.refreshCurrentView();
                });

                await listen('download:progress', (event) => {
                    this.updateDownloadProgressUI(event.payload);
                });

                await listen('download:completed', (event) => {
                    const p = event.payload;
                    if (p.status === 'completed') {
                        this.showToast(`Download complete: ${p.title}`, 'success');
                        this.scanLibrary();
                    } else if (p.status === 'failed') {
                        this.showToast(`Download failed: ${p.error_message || 'Stream error'}`, 'error');
                    }
                    this.updateDownloadProgressUI(p);
                });
            }
        } catch (e) {
            console.warn('Tauri bridge not available:', e);
        }

        this.initKeyboardHandler();
        this.bindHTMXEvents();
        this.refreshCurrentView();
    }

    async invoke(command, args = {}) {
        try {
            if (window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.invoke) {
                return await window.__TAURI_INTERNALS__.invoke(command, args);
            } else if (window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.tauri && window.__TAURI_INTERNALS__.tauri.invoke) {
                return await window.__TAURI_INTERNALS__.tauri.invoke(command, args);
            } else if (window.__TAURI__ && window.__TAURI__.core && window.__TAURI__.core.invoke) {
                return await window.__TAURI__.core.invoke(command, args);
            }
            return null;
        } catch (err) {
            console.error(`Error executing command '${command}':`, err);
            const msg = typeof err === 'string' ? err : (err.message || JSON.stringify(err));
            throw msg;
        }
    }

    initKeyboardHandler() {
        if (window.visualViewport) {
            window.visualViewport.addEventListener('resize', () => {
                const isKeyboardOpen = window.visualViewport.height < window.innerHeight - 150;
                document.body.classList.toggle('keyboard-open', isKeyboardOpen);
            });
        }
    }

    bindHTMXEvents() {
        document.body.addEventListener('htmx:afterSwap', () => {
            if (window.lucide) window.lucide.createIcons();
            this.refreshCurrentView();
        });
    }

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
    }

    async scanLibrary(paths = null) {
        this.showToast('Scanning storage for audio files...', 'info');

        try {
            const summary = await this.invoke('scan_library_paths', { paths: paths || null });
            if (summary) {
                this.showToast(`Scan complete: ${summary.tracks_added} new, ${summary.tracks_total} total`, 'success');
                this.refreshCurrentView();
            }
        } catch (err) {
            this.showToast(`Scan failed: ${err}`, 'error');
        }
    }

    async handleAudioImport(input) {
        if (!input || !input.files || input.files.length === 0) return;
        const files = Array.from(input.files);
        this.showToast(`Importing ${files.length} audio file(s)...`, 'info');

        let successCount = 0;
        for (const file of files) {
            try {
                const base64Data = await new Promise((resolve, reject) => {
                    const reader = new FileReader();
                    reader.onload = () => {
                        const result = String(reader.result || '');
                        const commaIdx = result.indexOf(',');
                        resolve(commaIdx !== -1 ? result.substring(commaIdx + 1) : result);
                    };
                    reader.onerror = (e) => reject(e);
                    reader.readAsDataURL(file);
                });

                const result = await this.invoke('import_audio_file', {
                    name: file.name,
                    data_base64: base64Data
                });
                if (result) successCount++;
            } catch (err) {
                console.error(`Failed to import ${file.name}:`, err);
            }
        }

        input.value = '';
        if (successCount > 0) {
            this.showToast(`Successfully imported ${successCount} track(s)!`, 'success');
            this.loadLibraryView();
        } else {
            this.showToast('Import failed. Please verify the audio files.', 'error');
        }
    }

    async loadLibraryView() {
        const trackList = document.querySelector('.page-library .track-list');
        if (!trackList) return;

        try {
            const page = await this.invoke('get_tracks');
            if (page && page.tracks && page.tracks.length > 0) {
                this.tracks = page.tracks;
                this.renderTrackRows(trackList, page.tracks);
            } else {
                trackList.innerHTML = `
                    <div class="empty-state glass neu" style="padding: var(--space-8); border-radius: var(--radius-lg); text-align: center;">
                        <div class="empty-state-icon" style="color: var(--accent); margin-bottom: var(--space-4);">
                            <i data-lucide="music" style="width: 48px; height: 48px;"></i>
                        </div>
                        <h2 class="empty-state-title" style="color: var(--text-1); font-size: var(--text-xl); margin-bottom: var(--space-2);">No tracks found in library</h2>
                        <p class="empty-state-description" style="color: var(--text-2); margin-bottom: var(--space-6); max-width: 420px; margin-left: auto; margin-right: auto;">Scan your device storage or download music to start listening.</p>
                        <div style="display: flex; gap: var(--space-3); flex-wrap: wrap; justify-content: center;">
                            <button class="btn btn-primary neu" onclick="window.Auralis.bridge.scanLibrary()">
                                <i data-lucide="folder-search"></i>
                                Scan Music Directory
                            </button>
                        </div>
                    </div>
                `;
                if (window.lucide) window.lucide.createIcons();
            }
        } catch (err) {
            console.error('Error loading library:', err);
        }
    }

    async loadHomeView() {
        try {
            const page = await this.invoke('get_tracks', { filter: { limit: 12 } });
            const shelf = document.getElementById('recently-added-shelf') || document.querySelector('.page-home .shelf') || document.querySelector('#content .shelf');
            const trackList = document.getElementById('continue-listening-tracks') || document.querySelector('.page-home .track-list');
            const container = document.getElementById('home-dynamic-content') || document.querySelector('.page-home');

            if (page && page.tracks && page.tracks.length > 0) {
                this.tracks = page.tracks;
                if (shelf) {
                    shelf.innerHTML = page.tracks.slice(0, 6).map(track => `
                        <div class="card album-card neu-glass" onclick="window.Auralis.bridge.playTrack('${track.id}')" style="cursor: pointer;">
                            <div class="card-artwork">
                                ${track.album_art_path ? `<img src="${track.album_art_path}" alt="${this.escapeHtml(track.title)}">` : `<i data-lucide="disc-3"></i>`}
                            </div>
                            <div class="card-body">
                                <div class="card-title">${this.escapeHtml(track.title)}</div>
                                <div class="card-subtitle">${this.escapeHtml(track.artist || 'Unknown Artist')}</div>
                            </div>
                        </div>
                    `).join('');
                }
                if (trackList) {
                    this.renderTrackRows(trackList, page.tracks.slice(0, 6));
                }
                if (window.lucide) window.lucide.createIcons();
            } else {
                if (container) {
                    container.innerHTML = `
                        <div class="empty-state glass neu" style="padding: var(--space-8); border-radius: var(--radius-lg); text-align: center; margin-top: var(--space-4);">
                            <div class="empty-state-icon" style="color: var(--accent); margin-bottom: var(--space-4);">
                                <i data-lucide="music" style="width: 48px; height: 48px;"></i>
                            </div>
                            <h2 class="empty-state-title" style="color: var(--text-1); font-size: var(--text-xl); margin-bottom: var(--space-2);">Your library is empty</h2>
                            <p class="empty-state-description" style="color: var(--text-2); margin-bottom: var(--space-6); max-width: 420px; margin-left: auto; margin-right: auto;">Scan your device storage for local audio files or download audio streams to start playing.</p>
                            <div style="display: flex; gap: var(--space-3); flex-wrap: wrap; justify-content: center;">
                                <button class="btn btn-primary neu" onclick="window.Auralis.bridge.scanLibrary()">
                                    <i data-lucide="folder-search"></i>
                                    Scan Device Storage
                                </button>
                                <button class="btn btn-secondary neu" hx-get="/partials/download.html" hx-target="#content" hx-swap="innerHTML transition:true">
                                    <i data-lucide="download"></i>
                                    Download Audio
                                </button>
                            </div>
                        </div>
                    `;
                    if (window.lucide) window.lucide.createIcons();
                }
            }
        } catch (err) {
            console.error('Error loading home view:', err);
        }
    }

    async loadAlbumsView() {
        const grid = document.getElementById('albums-grid') || document.querySelector('.page-albums .grid');
        if (!grid) return;

        try {
            const page = await this.invoke('get_tracks');
            if (page && page.tracks && page.tracks.length > 0) {
                this.tracks = page.tracks;
                const albumMap = new Map();
                for (const t of page.tracks) {
                    const albumName = t.album || 'Unknown Album';
                    if (!albumMap.has(albumName)) {
                        albumMap.set(albumName, { name: albumName, artist: t.artist || 'Unknown Artist', art: t.album_art_path, tracks: [] });
                    }
                    albumMap.get(albumName).tracks.push(t);
                }

                grid.innerHTML = Array.from(albumMap.values()).map(album => `
                    <div class="card album-card neu-glass" onclick="window.Auralis.bridge.playTrack('${album.tracks[0].id}')" style="cursor: pointer;">
                        <div class="card-artwork">
                            ${album.art ? `<img src="${album.art}" alt="${this.escapeHtml(album.name)}">` : `<i data-lucide="disc-3"></i>`}
                        </div>
                        <div class="card-body">
                            <div class="card-title">${this.escapeHtml(album.name)}</div>
                            <div class="card-subtitle">${this.escapeHtml(album.artist)} · ${album.tracks.length} tracks</div>
                        </div>
                    </div>
                `).join('');
                if (window.lucide) window.lucide.createIcons();
            } else {
                grid.innerHTML = `
                    <div class="empty-state glass neu" style="grid-column: 1 / -1; padding: var(--space-8); text-align: center; border-radius: var(--radius-lg);">
                        <i data-lucide="disc-3" style="width: 48px; height: 48px; color: var(--accent); margin-bottom: var(--space-3);"></i>
                        <h3 style="color: var(--text-1); font-size: var(--text-lg); margin-bottom: var(--space-2);">No albums found</h3>
                        <p style="color: var(--text-3); font-size: var(--text-sm);">Scan your music library to populate your album catalog.</p>
                    </div>
                `;
                if (window.lucide) window.lucide.createIcons();
            }
        } catch (err) {
            console.error('Error loading albums:', err);
        }
    }

    async loadArtistsView() {
        const grid = document.getElementById('artists-grid') || document.querySelector('.page-artists .grid');
        if (!grid) return;

        try {
            const page = await this.invoke('get_tracks');
            if (page && page.tracks && page.tracks.length > 0) {
                this.tracks = page.tracks;
                const artistMap = new Map();
                for (const t of page.tracks) {
                    const artistName = t.artist || 'Unknown Artist';
                    if (!artistMap.has(artistName)) {
                        artistMap.set(artistName, { name: artistName, tracks: [] });
                    }
                    artistMap.get(artistName).tracks.push(t);
                }

                grid.innerHTML = Array.from(artistMap.values()).map(artist => `
                    <div class="card artist-card neu-glass" onclick="window.Auralis.bridge.playTrack('${artist.tracks[0].id}')" style="cursor: pointer;">
                        <div class="card-artwork">
                            <i data-lucide="user"></i>
                        </div>
                        <div class="card-body">
                            <div class="card-title">${this.escapeHtml(artist.name)}</div>
                            <div class="card-subtitle">${artist.tracks.length} tracks</div>
                        </div>
                    </div>
                `).join('');
                if (window.lucide) window.lucide.createIcons();
            } else {
                grid.innerHTML = `
                    <div class="empty-state glass neu" style="grid-column: 1 / -1; padding: var(--space-8); text-align: center; border-radius: var(--radius-lg);">
                        <i data-lucide="users" style="width: 48px; height: 48px; color: var(--accent); margin-bottom: var(--space-3);"></i>
                        <h3 style="color: var(--text-1); font-size: var(--text-lg); margin-bottom: var(--space-2);">No artists found</h3>
                        <p style="color: var(--text-3); font-size: var(--text-sm);">Scan your music library to discover artists.</p>
                    </div>
                `;
                if (window.lucide) window.lucide.createIcons();
            }
        } catch (err) {
            console.error('Error loading artists:', err);
        }
    }

    async loadPlaylistsView() {
        const grid = document.getElementById('playlists-grid') || document.querySelector('.page-playlists .grid');
        if (!grid) return;

        try {
            const playlists = await this.invoke('get_playlists');
            if (playlists && playlists.length > 0) {
                grid.innerHTML = playlists.map(pl => `
                    <div class="card playlist-card neu-glass" style="cursor: pointer;">
                        <div class="card-artwork"><i data-lucide="list-music"></i></div>
                        <div class="card-body">
                            <div class="card-title">${this.escapeHtml(pl.name)}</div>
                            <div class="card-subtitle">${pl.track_ids ? pl.track_ids.length : 0} tracks</div>
                        </div>
                    </div>
                `).join('');
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
    }

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
    }

    async loadSyncView() {
        try {
            const pairingInfo = await this.invoke('start_pairing');
            if (pairingInfo) {
                const deviceIdEl = document.getElementById('sync-device-id');
                if (deviceIdEl) {
                    deviceIdEl.textContent = `Pairing PIN: ${pairingInfo.pin}`;
                }
                const qrContainer = document.getElementById('sync-qr-container');
                if (qrContainer && pairingInfo.qr_image) {
                    qrContainer.innerHTML = `<img src="data:image/png;base64,${pairingInfo.qr_image}" alt="Pairing QR" style="width: 100%; height: 100%; object-fit: contain; border-radius: var(--radius-sm);">`;
                }
            }
        } catch (e) {
            console.warn('Pairing info query failed:', e);
        }

        try {
            const devices = await this.invoke('get_paired_devices');
            const list = document.getElementById('synced-devices-list');
            if (list) {
                if (devices && devices.length > 0) {
                    list.innerHTML = devices.map(d => `
                        <div class="track-row neu-glass" style="margin-bottom: var(--space-2); border-radius: var(--radius-md); display: flex; align-items: center; justify-content: space-between; padding: var(--space-3);">
                            <div style="display: flex; align-items: center; gap: var(--space-3);">
                                <i data-lucide="${d.device_type === 'mobile' ? 'smartphone' : 'laptop'}" style="width: 24px; height: 24px; color: var(--accent);"></i>
                                <div>
                                    <div style="font-weight: var(--font-semibold); color: var(--text-1);">${this.escapeHtml(d.name)}</div>
                                    <div style="font-size: var(--text-xs); color: var(--text-3);">${d.ip_address || 'LAN Peer'} · Status: ${d.status || 'paired'}</div>
                                </div>
                            </div>
                            <div style="display: flex; gap: var(--space-2);">
                                <button class="btn btn-primary btn-sm neu" onclick="window.Auralis.bridge.syncWithDevice('${d.id}')">
                                    <i data-lucide="refresh-cw"></i>
                                    Sync
                                </button>
                            </div>
                        </div>
                    `).join('');
                    if (window.lucide) window.lucide.createIcons();
                } else {
                    list.innerHTML = `
                        <div class="empty-state glass neu" style="padding: var(--space-6); text-align: center; border-radius: var(--radius-md);">
                            <i data-lucide="wifi" style="width: 32px; height: 32px; color: var(--accent); margin-bottom: var(--space-2);"></i>
                            <h4 style="color: var(--text-1); font-size: var(--text-base); margin-bottom: var(--space-1);">No paired devices</h4>
                            <p style="color: var(--text-3); font-size: var(--text-xs);">Use the pairing PIN above on another device to start sharing your library.</p>
                        </div>
                    `;
                    if (window.lucide) window.lucide.createIcons();
                }
            }
        } catch (e) {
            console.error('Failed to load paired devices:', e);
        }
    }

    async syncWithDevice(deviceId) {
        if (!deviceId) return;
        this.showToast('Syncing with device...', 'info');
        try {
            await this.invoke('sync_with_device', { id: deviceId });
            this.showToast('Device synchronization complete!', 'success');
            this.loadSyncView();
        } catch (err) {
            this.showToast(`Sync failed: ${err}`, 'error');
        }
    }

    async connectDirectPeer() {
        const input = document.getElementById('direct-peer-address');
        if (!input || !input.value.trim()) {
            this.showToast('Please enter an IP:Port or Multiaddr', 'warning');
            return;
        }
        const address = input.value.trim();
        this.showToast(`Connecting to ${address}...`, 'info');
        try {
            const res = await this.invoke('connect_peer_address', { address });
            this.showToast(res || 'Direct connection established!', 'success');
            input.value = '';
            this.loadSyncView();
        } catch (err) {
            this.showToast(`Direct connection failed: ${err}`, 'error');
        }
    }

    async syncNow() {
        this.showToast('Initiating peer synchronization...', 'info');
        try {
            const devices = await this.invoke('get_paired_devices');
            if (devices && devices.length > 0) {
                for (const device of devices) {
                    await this.invoke('sync_with_device', { id: device.id });
                }
                this.showToast(`Synced with ${devices.length} device(s)!`, 'success');
            } else {
                await this.scanLibrary();
                this.showToast('Library scan & sync complete!', 'success');
            }
            this.loadSyncView();
        } catch (err) {
            this.showToast(`Sync failed: ${err}`, 'error');
        }
    }

    async loadDownloadView() {
        const form = document.getElementById('download-form');
        if (!form || form.dataset.bound) return;
        form.dataset.bound = 'true';

        form.addEventListener('submit', async (e) => {
            e.preventDefault();
            const urlInput = form.querySelector('input[name="url"]');
            const formatSelect = form.querySelector('select[name="format"]');
            if (!urlInput || !urlInput.value) return;

            const url = urlInput.value.trim();
            const format = formatSelect ? formatSelect.value : 'mp3';

            if (!url.startsWith('https://')) {
                this.showToast('Only secure HTTPS URLs are supported', 'error');
                return;
            }

            this.showToast('Starting audio download...', 'info');
            try {
                const result = await this.invoke('download_audio', { request: { url, format } });
                if (result) {
                    this.showToast('Download started!', 'success');
                    urlInput.value = '';
                    this.updateDownloadProgressUI(result);
                }
            } catch (err) {
                this.showToast(`Download failed: ${err}`, 'error');
            }
        });
    }

    async loadSearchView() {
        const form = document.getElementById('search-form');
        const resultsContainer = document.getElementById('search-results');
        if (!form || !resultsContainer || form.dataset.bound) return;
        form.dataset.bound = 'true';

        const performSearch = async () => {
            const input = form.querySelector('input[name="q"]');
            if (!input || !input.value.trim()) return;

            try {
                const page = await this.invoke('get_tracks', { filter: { search: input.value.trim() } });
                if (page && page.tracks && page.tracks.length > 0) {
                    this.renderTrackRows(resultsContainer, page.tracks);
                } else {
                    resultsContainer.innerHTML = `
                        <div class="empty-state glass neu" style="padding: var(--space-8); text-align: center; border-radius: var(--radius-lg);">
                            <div class="empty-state-icon"><i data-lucide="search"></i></div>
                            <h2 class="empty-state-title">No matching tracks</h2>
                            <p class="empty-state-description">Try searching for a different title, artist, or album.</p>
                        </div>
                    `;
                    if (window.lucide) window.lucide.createIcons();
                }
            } catch (err) {
                console.error('Search error:', err);
            }
        };

        form.addEventListener('submit', (e) => { e.preventDefault(); performSearch(); });
        const searchInput = form.querySelector('input[name="q"]');
        if (searchInput) {
            let debounceTimer;
            searchInput.addEventListener('input', () => {
                clearTimeout(debounceTimer);
                debounceTimer = setTimeout(performSearch, 300);
            });
        }
    }

    async initTheme() {
        const mediaQuery = window.matchMedia('(prefers-color-scheme: dark)');
        mediaQuery.addEventListener('change', () => {
            const currentSetting = (this.currentSettings && this.currentSettings.appearance && this.currentSettings.appearance.theme) || 'system';
            if (String(currentSetting).toLowerCase() === 'system') {
                this.applyTheme('system');
            }
        });

        try {
            const settings = await this.invoke('get_settings');
            if (settings) {
                this.currentSettings = settings;
                const theme = (settings.appearance && settings.appearance.theme) || 'system';
                this.applyTheme(theme);
            } else {
                this.applyTheme('system');
            }
        } catch (e) {
            this.applyTheme('system');
        }
    }

    applyTheme(theme) {
        const themeStr = String(theme || 'system').toLowerCase();
        let activeTheme = themeStr;
        if (themeStr === 'system') {
            activeTheme = window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
        }

        document.documentElement.setAttribute('data-theme', activeTheme);
        const metaThemeColor = document.querySelector('meta[name="theme-color"]');
        if (metaThemeColor) {
            metaThemeColor.setAttribute('content', activeTheme === 'light' ? '#f0f4f8' : '#070b10');
        }
    }

    async loadSettingsView() {
        const settingsView = document.querySelector('.page-settings, #settings-view');
        if (!settingsView || settingsView.dataset.bound) return;
        settingsView.dataset.bound = 'true';

        try {
            const settings = await this.invoke('get_settings');
            if (!settings) return;
            this.currentSettings = settings;

            const volInput = settingsView.querySelector('input[name="volume"]');
            if (volInput && settings.audio) {
                volInput.value = Math.round(settings.audio.volume * 100);
            }

            const downloadPathInput = settingsView.querySelector('input[name="download_path"]');
            if (downloadPathInput && settings.downloads) {
                downloadPathInput.value = settings.downloads.download_path || '';
            }

            const formatSelect = settingsView.querySelector('select[name="default_format"]');
            if (formatSelect && settings.downloads && settings.downloads.default_format) {
                formatSelect.value = String(settings.downloads.default_format).toLowerCase();
            }

            const themeSelect = settingsView.querySelector('select[name="theme"]');
            if (themeSelect && settings.appearance && settings.appearance.theme) {
                themeSelect.value = String(settings.appearance.theme).toLowerCase();
            }

            const syncToggle = settingsView.querySelector('[name="sync_enabled"], [data-name="sync_enabled"]');
            if (syncToggle && settings.sync) {
                if (syncToggle.type === 'checkbox') {
                    syncToggle.checked = Boolean(settings.sync.enabled);
                } else {
                    syncToggle.classList.toggle('active', Boolean(settings.sync.enabled));
                    syncToggle.setAttribute('aria-checked', Boolean(settings.sync.enabled).toString());
                }
            }

            const wifiToggle = settingsView.querySelector('[name="sync_wifi_only"], [data-name="sync_wifi_only"]');
            if (wifiToggle && settings.sync) {
                if (wifiToggle.type === 'checkbox') {
                    wifiToggle.checked = Boolean(settings.sync.wifi_only);
                } else {
                    wifiToggle.classList.toggle('active', Boolean(settings.sync.wifi_only));
                    wifiToggle.setAttribute('aria-checked', Boolean(settings.sync.wifi_only).toString());
                }
            }

            const saveSettings = async () => {
                if (!this.currentSettings) return;
                try {
                    await this.invoke('update_settings', { settings: this.currentSettings });
                    this.showToast('Settings saved', 'success');
                } catch (err) {
                    this.showToast(`Failed to save settings: ${err}`, 'error');
                }
            };

            if (volInput) {
                volInput.addEventListener('change', async (e) => {
                    if (this.currentSettings && this.currentSettings.audio) {
                        this.currentSettings.audio.volume = parseFloat(e.target.value) / 100;
                        await saveSettings();
                    }
                });
            }

            if (downloadPathInput) {
                downloadPathInput.addEventListener('change', async (e) => {
                    if (this.currentSettings && this.currentSettings.downloads) {
                        this.currentSettings.downloads.download_path = e.target.value;
                        await saveSettings();
                    }
                });
            }

            if (formatSelect) {
                formatSelect.addEventListener('change', async (e) => {
                    if (this.currentSettings && this.currentSettings.downloads) {
                        this.currentSettings.downloads.default_format = e.target.value;
                        await saveSettings();
                    }
                });
            }

            if (themeSelect) {
                themeSelect.addEventListener('change', async (e) => {
                    if (this.currentSettings && this.currentSettings.appearance) {
                        this.currentSettings.appearance.theme = e.target.value;
                        this.applyTheme(e.target.value);
                        await saveSettings();
                    }
                });
            }

            if (syncToggle) {
                syncToggle.addEventListener('click', async () => {
                    if (this.currentSettings && this.currentSettings.sync) {
                        const newState = !syncToggle.classList.contains('active');
                        syncToggle.classList.toggle('active', newState);
                        syncToggle.setAttribute('aria-checked', newState.toString());
                        this.currentSettings.sync.enabled = newState;
                        await saveSettings();
                    }
                });
            }

            if (wifiToggle) {
                wifiToggle.addEventListener('click', async () => {
                    if (this.currentSettings && this.currentSettings.sync) {
                        const newState = !wifiToggle.classList.contains('active');
                        wifiToggle.classList.toggle('active', newState);
                        wifiToggle.setAttribute('aria-checked', newState.toString());
                        this.currentSettings.sync.wifi_only = newState;
                        await saveSettings();
                    }
                });
            }
        } catch (err) {
            console.error('Settings load error:', err);
        }
    }

    async playTrack(trackId) {
        if (!trackId) return;
        try {
            const nowPlaying = await this.invoke('play', { trackId });
            if (nowPlaying && nowPlaying.track) {
                this.updatePlayerBar(nowPlaying.track);
            } else {
                const track = this.tracks.find(t => t.id === trackId);
                if (track) this.updatePlayerBar(track);
            }
        } catch (err) {
            this.showToast(`Playback error: ${err}`, 'error');
        }
    }

    renderTrackRows(container, tracks) {
        container.innerHTML = tracks.map(track => {
            const isFav = Boolean(track.is_favorite);
            return `
            <div class="track-row neu-glass" data-track-id="${track.id}" onclick="window.Auralis.bridge.playTrack('${track.id}')" style="cursor: pointer; margin-bottom: var(--space-2); border-radius: var(--radius-md);">
                <div class="track-row-artwork">
                    ${track.album_art_path ? `<img src="${track.album_art_path}" alt="${this.escapeHtml(track.title)}">` : `<i data-lucide="music"></i>`}
                </div>
                <div class="track-row-info">
                    <div class="track-row-title">${this.escapeHtml(track.title)}</div>
                    <div class="track-row-subtitle">${this.escapeHtml(track.artist || 'Unknown Artist')} — ${this.escapeHtml(track.album || 'Single')}</div>
                </div>
                <span class="track-row-duration">${this.formatTime(track.duration_secs || 0)}</span>
                <div class="track-row-actions" onclick="event.stopPropagation()">
                    <button class="btn btn-ghost btn-icon" title="Play" onclick="window.Auralis.bridge.playTrack('${track.id}')">
                        <i data-lucide="play"></i>
                    </button>
                    <button class="btn btn-ghost btn-icon ${isFav ? 'liked' : ''}" style="${isFav ? 'color: var(--like);' : ''}" title="Like" onclick="window.Auralis.bridge.toggleTrackFavorite('${track.id}', this)">
                        <i data-lucide="heart"></i>
                    </button>
                </div>
            </div>
        `;
        }).join('');

        if (window.lucide) window.lucide.createIcons();
    }

    async toggleTrackFavorite(trackId, buttonEl) {
        const track = this.tracks.find(t => t.id === trackId);
        const currentFav = track ? Boolean(track.is_favorite) : (buttonEl && buttonEl.classList.contains('liked'));
        const nextFav = !currentFav;

        if (track) {
            track.is_favorite = nextFav;
        }

        if (buttonEl) {
            buttonEl.classList.toggle('liked', nextFav);
            buttonEl.style.color = nextFav ? 'var(--like)' : '';
        }

        // If this track is currently playing in PlayerController, update player controller state too
        if (window.Auralis && window.Auralis.player && window.Auralis.player.currentTrack && window.Auralis.player.currentTrack.id === trackId) {
            window.Auralis.player.isLiked = nextFav;
            window.Auralis.player.currentTrack.is_favorite = nextFav;
            window.Auralis.player.updateLikeUI();
        }

        try {
            await this.invoke('set_track_favorite', { id: trackId, favorite: nextFav });
            this.showToast(nextFav ? 'Added to Liked Songs' : 'Removed from Liked Songs', 'info');
        } catch (err) {
            console.error('Failed to update track favorite:', err);
            if (track) track.is_favorite = currentFav;
            if (buttonEl) {
                buttonEl.classList.toggle('liked', currentFav);
                buttonEl.style.color = currentFav ? 'var(--like)' : '';
            }
            this.showToast(`Failed to update favorite: ${err}`, 'error');
        }
    }

    updateDownloadProgressUI(progress) {
        const list = document.getElementById('downloads-list');
        if (!list) return;

        let row = list.querySelector(`[data-download-id="${progress.id}"]`);
        if (!row) {
            row = document.createElement('div');
            row.className = 'track-row neu-glass';
            row.dataset.downloadId = progress.id;
            list.prepend(row);
        }

        const pct = Math.round((progress.progress || 0) * 100);
        row.innerHTML = `
            <div class="track-row-info">
                <div class="track-row-title">${this.escapeHtml(progress.filename || progress.url || 'Downloading...')}</div>
                <div class="track-row-subtitle">${progress.status} • ${pct}%</div>
            </div>
            <div class="progress-track neu-inset" style="width: 120px; height: 6px;">
                <div class="progress-fill" style="width: ${pct}%; background: var(--accent); height: 100%;"></div>
            </div>
        `;
    }

    on(event, callback) {
        if (!this.listeners[event]) this.listeners[event] = [];
        this.listeners[event].push(callback);
    }

    emit(event, data) {
        if (this.listeners[event]) {
            this.listeners[event].forEach(cb => cb(data));
        }
    }

    updatePlayerBar(track) {
        const title = document.getElementById('track-title');
        const artist = document.getElementById('track-artist');
        const artwork = document.getElementById('current-artwork');

        if (title) title.textContent = track.title || 'No track playing';
        if (artist) artist.textContent = track.artist || 'Select a song';
        if (artwork) {
            if (track.album_art_path) {
                artwork.innerHTML = `<img src="${track.album_art_path}" alt="${this.escapeHtml(track.title)}">`;
            } else {
                artwork.innerHTML = `<i data-lucide="music"></i>`;
                if (window.lucide) window.lucide.createIcons();
            }
        }
    }

    formatTime(secs) {
        const m = Math.floor(secs / 60);
        const s = Math.floor(secs % 60);
        return `${m}:${s.toString().padStart(2, '0')}`;
    }

    escapeHtml(str) {
        if (!str) return '';
        return str.replace(/[&<>"']/g, match => ({
            '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;'
        }[match]));
    }

    showToast(message, type = 'info') {
        const container = document.getElementById('toast-container') || document.body;
        const toast = document.createElement('div');
        toast.className = `toast toast-${type} glass`;
        toast.style.cssText = `
            position: fixed; top: calc(20px + env(safe-area-inset-top, 0px)); right: 20px; z-index: 1000;
            padding: 12px 20px; border-radius: 12px; background: rgba(11, 17, 24, 0.94);
            color: var(--text-1); border: 1px solid var(--glass-border); box-shadow: var(--shadow-lg);
            font-size: var(--text-sm); font-weight: 500;
        `;
        toast.textContent = message;
        container.appendChild(toast);
        setTimeout(() => {
            toast.style.opacity = '0';
            toast.style.transition = 'opacity 300ms ease';
            setTimeout(() => toast.remove(), 300);
        }, 3500);
    }
}

window.Auralis = window.Auralis || {};
window.Auralis.bridge = new Bridge();
document.addEventListener('DOMContentLoaded', () => {
    window.Auralis.bridge.init();
});
