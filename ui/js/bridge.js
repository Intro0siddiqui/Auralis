class Bridge {
    constructor() {
        this.listeners = {};
        this.tauriAvailable = false;
        this.tracks = [];
        this.activeView = 'home';
        this.initialized = false;
        this.currentSettings = null;
        this.init();
    }

    async init() {
        if (this.initialized) return;
        this.initialized = true;

        this.initTheme();

        try {
            const tauriListen = (window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.tauri && window.__TAURI_INTERNALS__.tauri.listen)
                || (window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.event && window.__TAURI_INTERNALS__.event.listen)
                || (window.__TAURI__ && window.__TAURI__.event && window.__TAURI__.event.listen)
                || (window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.listen);

            if (tauriListen) {
                this.tauriAvailable = true;

                await tauriListen('playback:state_changed', (event) => {
                    this.emit('playback:state', event.payload);
                });

                await tauriListen('playback:track_changed', (event) => {
                    this.emit('playback:track', event.payload);
                    this.updatePlayerBar(event.payload);
                });

                await tauriListen('playback:queue_updated', (event) => {
                    this.emit('playback:queue', event.payload);
                });

                await tauriListen('download:progress', (event) => {
                    this.emit('download:progress', event.payload);
                    this.updateDownloadProgressUI(event.payload);
                });

                await tauriListen('download:completed', (event) => {
                    this.emit('download:completed', event.payload);
                    const p = event.payload;
                    if (!p || p.status === 'completed') {
                        this.showToast(`Download complete: ${(p && p.title) || 'Audio track'}`, 'success');
                        this.scanLibrary();
                    } else if (p && p.status === 'failed') {
                        this.showToast(`Download failed: ${p.error_message || 'Stream error'}`, 'error');
                    }
                    if (p) {
                        this.updateDownloadProgressUI(p);
                    }
                });

                // Listen for scan progress events during background or folder scan
                await tauriListen('library:scan_progress', (event) => {
                    this.emit('library:scan_progress', event.payload);
                    this.updateScanProgressUI(event.payload);
                });

                // Listen for live diagnostic scan logs
                await tauriListen('library:scan_log', (event) => {
                    this.emit('library:scan_log', event.payload);
                    this.appendScanLog(event.payload);
                });

                // Listen for individual tracks imported in real-time
                await tauriListen('library:track_imported', (event) => {
                    this.emit('library:track_imported', event.payload);
                    this.handleTrackImported(event.payload);
                });

                // Listen for scan completion
                await tauriListen('library:scan_complete', (event) => {
                    this.emit('library:scan', event.payload);
                    this.finishScanProgressUI(event.payload);
                    const added = (event.payload && event.payload.tracks_added) || 0;
                    const updated = (event.payload && event.payload.tracks_updated) || 0;
                    this.showToast(`Library scan complete: ${added} added, ${updated} updated`, 'success');
                    this.refreshCurrentView();
                });
            }
        } catch (e) {
            console.warn('Tauri bridge event listener setup:', e);
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
        this.appendScanLog('🚀 Initiating storage scan...');
        this.showToast('Scanning storage for audio tracks...', 'info');
        this.updateScanProgressUI({
            title: 'Scanning Storage',
            subtitle: 'Searching audio paths...',
            current: 0,
            percentage: 0
        });

        try {
            const summary = await this.invoke('scan_library_paths', { paths: paths || null });
            if (summary) {
                const added = summary.tracks_added ?? 0;
                const updated = summary.tracks_updated ?? 0;
                const errCount = (summary.errors && summary.errors.length) || 0;
                this.appendScanLog(`🎉 Complete: ${added} added, ${updated} updated, ${errCount} errors`);
                
                if (added === 0 && updated === 0 && (!paths || paths.length === 0)) {
                    this.showToast('No audio files in app sandbox. Tap "Import Audio" to add songs from device storage.', 'info');
                    this.appendScanLog('ℹ️ Android Scoped Storage: Tap "Import Audio" to select audio files from your device storage');
                } else {
                    this.showToast(`Scan complete: ${added} added, ${updated} updated`, 'success');
                }
                await this.loadLibraryView();
                this.refreshCurrentView();
            }
        } catch (err) {
            this.appendScanLog(`❌ Scan error: ${err}`);
            this.showToast(`Scan error: ${err}`, 'error');
        }
    }

    async triggerAudioImport() {
        this.appendScanLog('📥 Opening system audio file picker...');
        const input = document.getElementById('global-audio-import-input') || document.getElementById('audio-import-input');
        if (input) {
            input.click();
            return;
        }

        try {
            this.showToast('Select audio files to import...', 'info');
            const summary = await this.invoke('pick_audio_files_and_import');
            if (summary !== undefined && summary !== null) {
                const added = summary.tracks_added ?? 0;
                const updated = summary.tracks_updated ?? 0;
                this.appendScanLog(`🎉 Import complete: ${added} added, ${updated} updated`);
                this.showToast(`Import complete: ${added} added, ${updated} updated`, 'success');
                await this.loadLibraryView();
                this.refreshCurrentView();
            } else if (summary === null && this.tauriAvailable) {
                this.appendScanLog('ℹ️ Audio picker was cancelled');
            }
        } catch (err) {
            this.appendScanLog(`⚠️ Native audio picker notice: ${err}`);
            console.log('Native audio picker fallback:', err);
        }
    }

    async triggerFolderScan() {
        this.appendScanLog('📂 Requesting music storage selection...');
        try {
            const summary = await this.invoke('pick_folder_and_scan');
            if (summary !== undefined && summary !== null) {
                const added = summary.tracks_added ?? 0;
                const updated = summary.tracks_updated ?? 0;
                this.appendScanLog(`🎉 Scan complete: ${added} added, ${updated} updated`);
                this.showToast(`Scan complete: ${added} added, ${updated} updated`, 'success');
                await this.loadLibraryView();
                this.refreshCurrentView();
                return;
            } else if (summary === null && this.tauriAvailable) {
                this.appendScanLog('ℹ️ Folder picker was cancelled');
                return;
            }
        } catch (err) {
            console.log('Native folder dialog notice:', err);
        }

        // Fallback to direct audio file picker
        await this.triggerAudioImport();
    }

    async handleFolderScan(input) {
        if (!input || !input.files || input.files.length === 0) return;
        const allFiles = Array.from(input.files);
        this.appendScanLog(`📱 SAF picker returned ${allFiles.length} raw files`);
        const audioExtensions = ['.mp3', '.flac', '.wav', '.m4a', '.aac', '.ogg', '.opus', '.wma'];
        const audioFiles = allFiles.filter(file => {
            const name = file.name.toLowerCase();
            return file.type.startsWith('audio/') || audioExtensions.some(ext => name.endsWith(ext));
        });

        if (audioFiles.length === 0) {
            this.appendScanLog('⚠️ No supported audio format files found in selected directory');
            this.showToast('No audio files found in selected folder.', 'info');
            input.value = '';
            return;
        }

        this.appendScanLog(`🎵 Found ${audioFiles.length} audio file(s) to import`);
        this.showToast(`Found ${audioFiles.length} audio file(s). Scanning & importing...`, 'info');
        this.updateScanProgressUI({
            title: 'Importing Folder Audio',
            subtitle: `0 / ${audioFiles.length} files processed`,
            current: 0,
            total: audioFiles.length,
            percentage: 0
        });

        let importedCount = 0;
        for (let i = 0; i < audioFiles.length; i++) {
            const file = audioFiles[i];
            const pct = Math.round(((i + 1) / audioFiles.length) * 100);
            this.updateScanProgressUI({
                title: `Importing (${i + 1}/${audioFiles.length})`,
                subtitle: file.name,
                current: i + 1,
                total: audioFiles.length,
                percentage: pct
            });

            try {
                this.appendScanLog(`⏳ Reading: ${file.name} (${Math.round(file.size / 1024)} KB)`);
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
                if (result) {
                    importedCount++;
                    this.appendScanLog(`✅ Imported: ${result.artist || 'Unknown'} - ${result.title || file.name}`);
                    this.handleTrackImported(result);
                }
            } catch (err) {
                this.appendScanLog(`❌ Failed to import ${file.name}: ${err}`);
                console.error(`Failed to scan file ${file.name}:`, err);
            }
        }

        input.value = '';
        this.finishScanProgressUI({ tracks_added: importedCount, errors: [] });

        if (importedCount > 0) {
            this.showToast(`Scan complete: ${importedCount} track(s) added to library!`, 'success');
        } else {
            this.showToast('No new audio tracks could be imported.', 'warning');
        }
        await this.loadLibraryView();
        this.refreshCurrentView();
    }

    async handleAudioImport(input) {
        if (!input || !input.files || input.files.length === 0) return;
        const files = Array.from(input.files);
        this.appendScanLog(`📥 Selected ${files.length} audio file(s) for direct import`);
        this.showToast(`Importing ${files.length} audio file(s)...`, 'info');
        this.updateScanProgressUI({
            title: 'Importing Audio Files',
            subtitle: `0 / ${files.length} files imported`,
            current: 0,
            total: files.length,
            percentage: 0
        });

        let successCount = 0;
        for (let i = 0; i < files.length; i++) {
            const file = files[i];
            const pct = Math.round(((i + 1) / files.length) * 100);
            this.updateScanProgressUI({
                title: `Importing (${i + 1}/${files.length})`,
                subtitle: file.name,
                current: i + 1,
                total: files.length,
                percentage: pct
            });

            try {
                this.appendScanLog(`⏳ Processing buffer: ${file.name}`);
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
                    data_base64: base64Data,
                    dataBase64: base64Data
                });
                if (result) {
                    successCount++;
                    this.appendScanLog(`✅ Successfully ingested: ${result.artist} - ${result.title}`);
                    this.handleTrackImported(result);
                }
            } catch (err) {
                this.appendScanLog(`❌ Ingestion failed for ${file.name}: ${err}`);
                console.error(`Failed to import ${file.name}:`, err);
            }
        }

        input.value = '';
        this.finishScanProgressUI({ tracks_added: successCount, errors: [] });

        if (successCount > 0) {
            this.showToast(`Successfully imported ${successCount} track(s)!`, 'success');
        } else {
            this.showToast('Import failed. Please verify the audio files.', 'error');
        }
        await this.loadLibraryView();
        this.refreshCurrentView();
    }

    handleTrackImported(track) {
        if (!track || !track.id) return;
        const existingIdx = this.tracks.findIndex(t => t.id === track.id);
        if (existingIdx >= 0) {
            this.tracks[existingIdx] = track;
        } else {
            this.tracks.unshift(track);
        }
        if (this.activeView === 'library') {
            this.renderLibraryTracks();
        }
    }

    updateScanProgressUI(payload) {
        if (!payload) return;
        const current = payload.current ?? payload.scanned ?? payload.processed ?? payload.count ?? 0;
        const total = payload.total ?? payload.total_files ?? null;
        const file = payload.file ?? payload.path ?? payload.filename ?? payload.current_file ?? payload.title ?? '';
        const percentage = payload.percentage !== undefined && payload.percentage !== null
            ? payload.percentage
            : (total && total > 0 ? Math.round((current / total) * 100) : (payload.progress ? Math.round(payload.progress * 100) : null));

        const banner = document.getElementById('library-scan-progress');
        const titleEl = document.getElementById('library-scan-title');
        const subEl = document.getElementById('library-scan-subtitle');
        const counterEl = document.getElementById('library-scan-counter');
        const barEl = document.getElementById('library-scan-bar');

        if (banner) {
            banner.style.display = 'flex';
            if (titleEl) {
                titleEl.textContent = payload.title || (total ? `Scanning storage (${current}/${total})` : `Scanning storage (${current} files)`);
            }
            if (subEl) {
                const fileName = file ? file.split(/[\\/]/).pop() : (payload.subtitle || 'Processing audio files...');
                subEl.textContent = fileName;
            }
            if (counterEl) {
                counterEl.textContent = total ? `${current} / ${total}` : `${current} files`;
            }
            if (barEl) {
                if (percentage !== null) {
                    barEl.style.width = `${Math.min(100, Math.max(0, percentage))}%`;
                } else {
                    barEl.style.width = '100%';
                }
            }
        }

        this.updateScanToast(payload, current, total, percentage, file);
    }

    updateScanToast(payload, current, total, percentage, file) {
        const container = document.getElementById('toast-container') || document.body;
        let toast = document.getElementById('scan-progress-toast');
        if (!toast) {
            toast = document.createElement('div');
            toast.id = 'scan-progress-toast';
            toast.className = 'toast toast-info glass';
            toast.style.cssText = `
                position: fixed; top: calc(20px + env(safe-area-inset-top, 0px)); right: 20px; z-index: 1000;
                padding: 12px 20px; border-radius: 12px; background: rgba(11, 17, 24, 0.94);
                color: var(--text-1); border: 1px solid var(--glass-border); box-shadow: var(--shadow-lg);
                font-size: var(--text-sm); font-weight: 500; min-width: 240px; display: flex; flex-direction: column; gap: 6px;
                transition: opacity 300ms ease;
            `;
            container.appendChild(toast);
        }

        const fileName = file ? file.split(/[\\/]/).pop() : (payload.subtitle || 'Scanning...');
        const countText = total ? `${current} / ${total}` : `${current} files`;
        const pctText = percentage !== null ? ` (${percentage}%)` : '';

        toast.innerHTML = `
            <div style="display: flex; justify-content: space-between; align-items: center;">
                <span style="font-weight: 600;">Scanning audio...</span>
                <span style="color: var(--accent); font-size: var(--text-xs); font-weight: 600;">${countText}${pctText}</span>
            </div>
            <div style="font-size: var(--text-xs); color: var(--text-3); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; max-width: 260px;">
                ${this.escapeHtml(fileName)}
            </div>
            <div class="progress-track neu-inset" style="width: 100%; height: 4px; margin-top: 2px;">
                <div class="progress-fill" style="width: ${percentage !== null ? percentage : 100}%; background: var(--accent); height: 100%; transition: width 0.2s ease;"></div>
            </div>
        `;
        toast.style.opacity = '1';
    }

    appendScanLog(msg) {
        if (!msg) return;
        const banner = document.getElementById('library-scan-progress');
        if (banner) banner.style.display = 'flex';

        const box = document.getElementById('library-scan-logbox');
        if (box) box.style.display = 'block';

        const content = document.getElementById('library-scan-log-content');
        if (content) {
            const line = document.createElement('div');
            const time = new Date().toLocaleTimeString();
            line.style.cssText = 'white-space: pre-wrap; word-break: break-all; margin-bottom: 2px;';
            if (msg.includes('❌') || msg.includes('Error') || msg.includes('failed')) {
                line.style.color = '#ef4444';
            } else if (msg.includes('⚠️') || msg.includes('Warning')) {
                line.style.color = '#f59e0b';
            } else if (msg.includes('✅') || msg.includes('🎉')) {
                line.style.color = '#10b981';
            } else if (msg.includes('🎵') || msg.includes('📂')) {
                line.style.color = '#38bdf8';
            }
            line.textContent = `[${time}] ${msg}`;
            content.appendChild(line);
            if (box) box.scrollTop = box.scrollHeight;
        }
    }

    toggleScanLogs() {
        const box = document.getElementById('library-scan-logbox');
        if (box) {
            box.style.display = box.style.display === 'none' ? 'block' : 'none';
        }
    }

    copyScanLogs() {
        const content = document.getElementById('library-scan-log-content');
        if (content) {
            const text = content.innerText || content.textContent;
            navigator.clipboard.writeText(text).then(() => {
                this.showToast('Diagnostic logs copied to clipboard!', 'success');
            }).catch(() => {
                this.showToast('Failed to copy logs', 'error');
            });
        }
    }

    finishScanProgressUI(payload) {
        const titleEl = document.getElementById('library-scan-title');
        const subEl = document.getElementById('library-scan-subtitle');
        const icon = document.querySelector('#library-scan-progress i');

        if (titleEl) titleEl.textContent = 'Scan Complete';
        if (subEl) {
            const added = (payload && payload.tracks_added) || 0;
            const errors = (payload && payload.errors && payload.errors.length) || 0;
            subEl.textContent = `${added} tracks added • ${errors} errors`;
        }
        if (icon) {
            icon.classList.remove('spin');
            icon.setAttribute('data-lucide', 'check-circle-2');
            if (window.lucide) window.lucide.createIcons();
        }

        const toast = document.getElementById('scan-progress-toast');
        if (toast) {
            toast.style.opacity = '0';
            setTimeout(() => {
                if (toast && toast.parentElement) toast.remove();
            }, 300);
        }
    }

    hideScanProgressUI() {
        const banner = document.getElementById('library-scan-progress');
        if (banner) {
            banner.style.display = 'none';
        }
        const toast = document.getElementById('scan-progress-toast');
        if (toast) {
            toast.style.opacity = '0';
            setTimeout(() => {
                if (toast && toast.parentElement) toast.remove();
            }, 300);
        }
    }

    async loadLibraryView() {
        const trackList = document.querySelector('.page-library .track-list');
        if (!trackList) return;

        try {
            const page = await this.invoke('get_tracks');
            if (page && page.tracks && page.tracks.length > 0) {
                this.tracks = page.tracks;
            } else {
                this.tracks = [];
            }
            this.bindLibraryFilterControls();
            this.renderLibraryTracks();
        } catch (err) {
            console.error('Error loading library:', err);
            this.bindLibraryFilterControls();
            this.renderLibraryTracks();
        }
    }

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
    }

    renderLibraryTracks() {
        const trackList = document.querySelector('.page-library .track-list');
        if (!trackList) return;

        const sortSelect = document.querySelector('.page-library select[name="sort_by"]');
        const downloadedCheckbox = document.querySelector('.page-library input[name="downloaded_only"]');

        let filtered = [...this.tracks];

        if (downloadedCheckbox && downloadedCheckbox.checked) {
            filtered = filtered.filter(t => t.is_downloaded || (t.file_path && !t.file_path.startsWith('http')));
        }

        const sortBy = sortSelect ? sortSelect.value : 'date_added';
        if (sortBy === 'title') {
            filtered.sort((a, b) => (a.title || '').localeCompare(b.title || ''));
        } else if (sortBy === 'artist') {
            filtered.sort((a, b) => (a.artist || '').localeCompare(b.artist || ''));
        } else if (sortBy === 'album') {
            filtered.sort((a, b) => (a.album || '').localeCompare(b.album || ''));
        } else if (sortBy === 'date_added') {
            if (filtered.some(t => t.created_at || t.date_added)) {
                filtered.sort((a, b) => new Date(b.created_at || b.date_added || 0) - new Date(a.created_at || a.date_added || 0));
            }
        }

        if (filtered.length > 0) {
            this.renderTrackRows(trackList, filtered);
        } else {
            trackList.innerHTML = `
                <div class="empty-state glass neu" style="padding: var(--space-8); border-radius: var(--radius-lg); text-align: center;">
                    <div class="empty-state-icon" style="color: var(--accent); margin-bottom: var(--space-4);">
                        <i data-lucide="music" style="width: 48px; height: 48px;"></i>
                    </div>
                    <h2 class="empty-state-title" style="color: var(--text-1); font-size: var(--text-xl); margin-bottom: var(--space-2);">No tracks found in library</h2>
                    <p class="empty-state-description" style="color: var(--text-2); margin-bottom: var(--space-6); max-width: 420px; margin-left: auto; margin-right: auto;">Scan your device storage or download music to start listening.</p>
                    <div style="display: flex; gap: var(--space-3); flex-wrap: wrap; justify-content: center;">
                        <label class="btn btn-primary neu" for="global-audio-import-input" style="cursor: pointer; display: inline-flex; align-items: center; gap: var(--space-2); margin: 0;">
                            <i data-lucide="file-plus-2"></i>
                            Import Audio
                        </label>
                        <label class="btn btn-secondary neu" for="global-audio-import-input" style="cursor: pointer; display: inline-flex; align-items: center; gap: var(--space-2); margin: 0;">
                            <i data-lucide="folder-search"></i>
                            Scan Storage
                        </label>
                    </div>
                </div>
            `;
            if (window.lucide) window.lucide.createIcons();
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
                                ${track.album_art_path ? this.artImgTag(track.album_art_path, track.title) : `<i data-lucide="disc-3"></i>`}
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
                            <p class="empty-state-description" style="color: var(--text-2); margin-bottom: var(--space-6); max-width: 420px; margin-left: auto; margin-right: auto;">Import audio files from your device storage or download music to start playing.</p>
                            <div style="display: flex; gap: var(--space-3); flex-wrap: wrap; justify-content: center;">
                                <label class="btn btn-primary neu" for="global-audio-import-input" style="cursor: pointer; display: inline-flex; align-items: center; gap: var(--space-2); margin: 0;">
                                    <i data-lucide="file-plus-2"></i>
                                    Import Audio
                                </label>
                                <label class="btn btn-secondary neu" for="global-audio-import-input" style="cursor: pointer; display: inline-flex; align-items: center; gap: var(--space-2); margin: 0;">
                                    <i data-lucide="folder-search"></i>
                                    Scan Storage
                                </label>
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
                            ${album.art ? this.artImgTag(album.art, album.name) : `<i data-lucide="disc-3"></i>`}
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
        const detail = document.getElementById('playlist-detail');
        if (detail) detail.style.display = 'none';
        if (grid) grid.style.display = 'grid';
        if (!grid) return;

        try {
            const playlists = await this.invoke('get_playlists');
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
    }

    async openPlaylist(playlistId) {
        if (!playlistId) return;
        try {
            const result = await this.invoke('get_playlist', { id: playlistId });
            if (!result) {
                this.showToast('Playlist not found', 'warning');
                return;
            }
            const [playlist, tracks] = result;
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
    }

    closePlaylistDetail() {
        const detailEl = document.getElementById('playlist-detail');
        const gridEl = document.getElementById('playlists-grid') || document.querySelector('.page-playlists .grid');
        if (detailEl) detailEl.style.display = 'none';
        if (gridEl) gridEl.style.display = 'grid';
        if (window.lucide) window.lucide.createIcons();
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

    async ensureSettings() {
        if (this.currentSettings) return this.currentSettings;
        try {
            this.currentSettings = await this.invoke('get_settings');
        } catch (_) {}
        return this.currentSettings;
    }

    getDownloadOptions(form) {
        const containerSelect = form ? form.querySelector('select[name="container"]') : null;
        const qualitySelect = form ? form.querySelector('select[name="quality"]') : null;
        const opts = {
            container: containerSelect ? containerSelect.value : 'auto',
            quality: qualitySelect ? qualitySelect.value : 'best',
        };
        const dl = (this.currentSettings && this.currentSettings.downloads) || {};
        opts.cookie = dl.youtube_cookie || '';
        opts.poToken = dl.youtube_po_token || '';
        return opts;
    }

    buildDownloadPayload(resolved, format) {
        return {
            request: {
                url: resolved.stream_url,
                title: resolved.title,
                platform: resolved.platform,
                format,
                ext: resolved.ext,
                total_bytes: resolved.total_bytes,
                thumbnail: resolved.thumbnail,
            },
        };
    }

    async downloadResolvedTrack(resolved, format) {
        if (!resolved || resolved.kind !== 'track') throw new Error('Not a downloadable track');
        const result = await this.invoke('download_audio', this.buildDownloadPayload(resolved, format));
        if (result) this.updateDownloadProgressUI(result);
        return result;
    }

    async loadDownloadView() {
        const form = document.getElementById('download-form');
        const searchForm = document.getElementById('youtube-search-form');
        if (!form || form.dataset.bound) return;
        form.dataset.bound = 'true';

        await this.ensureSettings();

        form.addEventListener('submit', async (e) => {
            e.preventDefault();
            const urlInput = form.querySelector('input[name="url"]');
            if (!urlInput || !urlInput.value) return;

            const url = urlInput.value.trim();
            if (!url.startsWith('https://')) {
                this.showToast('Only secure HTTPS URLs are supported', 'error');
                return;
            }
            if (!window.AuralisYouTube) {
                this.showToast('YouTube resolver unavailable', 'error');
                return;
            }

            const opts = this.getDownloadOptions(form);
            this.showToast('Resolving source…', 'info');
            try {
                const resolved = await window.AuralisYouTube.resolve(url, opts);
                if (resolved.kind === 'playlist') {
                    await this.startPlaylistDownloads(resolved.items, 'm4a', opts, urlInput);
                    return;
                }
                const result = await this.downloadResolvedTrack(resolved, 'm4a');
                if (result) {
                    this.showToast('Download started!', 'success');
                    urlInput.value = '';
                }
            } catch (err) {
                console.error(err);
                this.showToast(`Resolve failed: ${err && err.message ? err.message : err}`, 'error');
            }
        });

        if (searchForm && !searchForm.dataset.bound) {
            searchForm.dataset.bound = 'true';
            searchForm.addEventListener('submit', async (ev) => {
                ev.preventDefault();
                const q = searchForm.querySelector('input[name="q"]');
                if (!q || !q.value.trim()) return;
                await this.performYouTubeSearch(q.value.trim(), this.getDownloadOptions(form || searchForm));
            });
        }
    }

    async performYouTubeSearch(query, opts) {
        if (!window.AuralisYouTube) {
            this.showToast('YouTube resolver unavailable', 'error');
            return;
        }
        const resultsEl = document.getElementById('youtube-search-results');
        if (!resultsEl) return;
        this.showToast('Searching YouTube…', 'info');
        try {
            const results = await window.AuralisYouTube.search(query, opts);
            resultsEl.style.display = 'block';
            if (!results || results.length === 0) {
                resultsEl.innerHTML = `
                    <div class="empty-state" style="padding: var(--space-4);">
                        <p style="color: var(--text-3); font-size: var(--text-sm);">No results found for “${this.escapeHtml(query)}”.</p>
                    </div>`;
                return;
            }
            this._lastSearchResults = results;
            resultsEl.innerHTML = results.map((r, i) => `
                <div class="track-row neu-glass" style="cursor: pointer;">
                    <div class="track-row-info" style="flex: 1; min-width: 0;" onclick="window.Auralis.bridge.downloadSearchResult(${i})">
                        <div class="track-row-title">${this.escapeHtml(r.title)}</div>
                        <div class="track-row-subtitle">YouTube${r.channel ? ` • ${this.escapeHtml(r.channel)}` : ''}</div>
                    </div>
                    <button type="button" class="btn btn-secondary btn-sm" onclick="event.stopPropagation(); window.Auralis.bridge.downloadSearchResult(${i})">
                        <i data-lucide="download"></i>
                    </button>
                </div>
            `).join('');
            if (window.lucide) window.lucide.createIcons();
        } catch (err) {
            console.error(err);
            this.showToast(`Search failed: ${err && err.message ? err.message : err}`, 'error');
        }
    }

    async downloadSearchResult(index) {
        const results = this._lastSearchResults || [];
        const item = results[index];
        if (!item) return;
        const form = document.getElementById('download-form');
        const opts = this.getDownloadOptions(form);
        this.showToast('Resolving track…', 'info');
        try {
            const resolved = await window.AuralisYouTube.resolve(item.url, opts);
            if (resolved.kind !== 'track') throw new Error('Not a track');
            const result = await this.downloadResolvedTrack(resolved, 'm4a');
            if (result) this.showToast('Download started!', 'success');
        } catch (err) {
            console.error(err);
            this.showToast(`Resolve failed: ${err && err.message ? err.message : err}`, 'error');
        }
    }

    async startPlaylistDownloads(items, format, opts, urlInput) {
        if (!items || items.length === 0) {
            this.showToast('Playlist contained no tracks', 'error');
            return;
        }
        const capped = items.slice(0, 20);
        this.showToast(`Resolving playlist (${capped.length} tracks)…`, 'info');

        let started = 0;
        for (const item of capped) {
            try {
                const t = await window.AuralisYouTube.resolve(item.url, opts);
                if (t.kind !== 'track') continue;
                const result = await this.downloadResolvedTrack(t, format);
                if (result) started++;
            } catch (err) {
                console.error('Playlist item failed:', err);
            }
        }

        if (started > 0) {
            this.showToast(`Started ${started} downloads from playlist`, 'success');
            if (urlInput) urlInput.value = '';
        } else {
            this.showToast('Could not start any playlist downloads', 'error');
        }
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
            const volBadge = settingsView.querySelector('#settings-volume-val');
            if (volInput && settings.audio) {
                const volPct = Math.round(settings.audio.volume * 100);
                volInput.value = volPct;
                if (volBadge) volBadge.textContent = `${volPct}%`;
            }

            const downloadPathInput = settingsView.querySelector('input[name="download_path"]');
            if (downloadPathInput && settings.downloads) {
                downloadPathInput.value = settings.downloads.download_path || '';
            }

            const formatSelect = settingsView.querySelector('select[name="default_format"]');
            if (formatSelect && settings.downloads && settings.downloads.default_format) {
                formatSelect.value = String(settings.downloads.default_format).toLowerCase();
            }

            const ytCookieInput = settingsView.querySelector('input[name="youtube_cookie"]');
            if (ytCookieInput && settings.downloads) {
                ytCookieInput.value = settings.downloads.youtube_cookie || '';
            }

            const ytPoTokenInput = settingsView.querySelector('input[name="youtube_po_token"]');
            if (ytPoTokenInput && settings.downloads) {
                ytPoTokenInput.value = settings.downloads.youtube_po_token || '';
            }

            if (window.AuralisYouTube && settings.downloads) {
                window.AuralisYouTube.setCredentials({
                    cookie: settings.downloads.youtube_cookie,
                    po_token: settings.downloads.youtube_po_token,
                });
            }

            const currentTheme = (settings.appearance && settings.appearance.theme) ? String(settings.appearance.theme).toLowerCase() : 'dark';
            const themeOptions = settingsView.querySelectorAll('.theme-option[data-theme]');
            themeOptions.forEach(opt => {
                opt.classList.toggle('active', opt.dataset.theme === currentTheme);
            });

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
                volInput.addEventListener('input', (e) => {
                    if (volBadge) volBadge.textContent = `${e.target.value}%`;
                });
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

            if (ytCookieInput) {
                ytCookieInput.addEventListener('change', async (e) => {
                    if (this.currentSettings && this.currentSettings.downloads) {
                        this.currentSettings.downloads.youtube_cookie = e.target.value || null;
                        await saveSettings();
                    }
                });
            }

            if (ytPoTokenInput) {
                ytPoTokenInput.addEventListener('change', async (e) => {
                    if (this.currentSettings && this.currentSettings.downloads) {
                        this.currentSettings.downloads.youtube_po_token = e.target.value || null;
                        await saveSettings();
                    }
                });
            }

            themeOptions.forEach(btn => {
                btn.addEventListener('click', async () => {
                    const selectedTheme = btn.dataset.theme;
                    themeOptions.forEach(opt => opt.classList.toggle('active', opt === btn));
                    if (this.currentSettings && this.currentSettings.appearance) {
                        this.currentSettings.appearance.theme = selectedTheme;
                        this.applyTheme(selectedTheme);
                        await saveSettings();
                    }
                });
            });

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

            if (window.lucide) window.lucide.createIcons();
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
                    ${track.album_art_path ? this.artImgTag(track.album_art_path, track.title) : `<i data-lucide="music"></i>`}
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
                <div class="track-row-title">${this.escapeHtml(progress.title || progress.url || 'Downloading...')}</div>
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
                artwork.innerHTML = this.artImgTag(track.album_art_path, track.title);
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

    assetUrl(path) {
        if (!path) return '';
        if (/^(https?:|data:|blob:|asset:)/.test(path)) return path;
        const internals = window.__TAURI_INTERNALS__;
        if (internals && typeof internals.convertFileSrc === 'function') {
            try {
                return internals.convertFileSrc(path);
            } catch (_) {}
        }
        if (window.__TAURI__ && window.__TAURI__.core && typeof window.__TAURI__.core.convertFileSrc === 'function') {
            try {
                return window.__TAURI__.core.convertFileSrc(path);
            } catch (_) {}
        }
        return path;
    }

    async embedArt(imgEl, path) {
        if (!imgEl || !path) return;
        try {
            const dataUri = await this.invoke('media_data_url', { path });
            if (dataUri) imgEl.src = dataUri;
        } catch (err) {
            console.error('Cover art fallback failed:', err);
        }
    }

    artImgTag(path, altText) {
        if (!path) return '';
        const safeAlt = this.escapeHtml(altText || '');
        const src = this.assetUrl(path);
        const jsonPath = JSON.stringify(path).replace(/</g, '\\u003c');
        return `<img src="${src}" alt="${safeAlt}" onerror="if(!this.dataset.fb){this.dataset.fb='1';window.Auralis.bridge.embedArt(this, ${jsonPath})}">`;
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
window.Auralis.assetUrl = (path) => window.Auralis.bridge.assetUrl(path);
document.addEventListener('DOMContentLoaded', () => {
    window.Auralis.bridge.init();
});
