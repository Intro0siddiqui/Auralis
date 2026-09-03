/**
 * Library Module
 * Handles file importing, folder scanning, and library updates.
 */

export const libraryMethods = {
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
    },

    async triggerAudioImport() {
        this.appendScanLog('📥 Opening audio file picker...');
        // Ensure input exists (also called from init, but re-check after HTMX swaps)
        this.ensureFileInput();
        const input = document.getElementById('global-audio-import-input');
        if (input) {
            input.value = ''; // Crucial for Android: reset value so selecting same file fires change
            input.click();
            return;
        }
        // Desktop-only fallback: native Tauri dialog (returns POSIX paths, not content:// URIs)
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
            } else if (summary === null) {
                this.appendScanLog('ℹ️ Audio picker was cancelled');
            }
        } catch (err) {
            const msg = err && err.message ? err.message : String(err);
            this.appendScanLog(`⚠️ Audio picker error: ${msg}`);
            this.showToast(`Audio picker error: ${msg}`, 'error');
            console.warn('pick_audio_files_and_import error:', err);
        }
    },

    async triggerFolderScan() {
        this.appendScanLog('📂 Requesting music storage scan...');

        // Check if running on Android/mobile WebView
        const isMobile = /Android|iPhone|iPad|iPod/i.test(navigator.userAgent) || Boolean(window.__TAURI_METADATA__ && window.__TAURI_METADATA__.__currentWindow && !window.__TAURI_METADATA__.__currentWindow.label);

        if (isMobile) {
            // On mobile/Android:
            // 1. Run the system MediaStore & sandboxed storage scan directly to find all songs on device
            await this.scanLibrary();

            // 2. Also offer file picker if user wanted manual folder/file selection
            this.ensureFileInput();
            const folderInput = document.getElementById('global-folder-scan-input');
            if (folderInput) {
                try {
                    folderInput.value = '';
                    folderInput.click();
                } catch (_) {
                    this.triggerAudioImport();
                }
            }
            return;
        }

        // Desktop: native folder dialog
        try {
            const summary = await this.invoke('pick_folder_and_scan');
            if (summary !== undefined && summary !== null) {
                const added = summary.tracks_added ?? 0;
                const updated = summary.tracks_updated ?? 0;
                this.appendScanLog(`🎉 Scan complete: ${added} added, ${updated} updated`);
                this.showToast(`Scan complete: ${added} added, ${updated} updated`, 'success');
                await this.loadLibraryView();
                this.refreshCurrentView();
            } else if (summary === null) {
                this.appendScanLog('ℹ️ Folder picker was cancelled');
            }
        } catch (err) {
            console.warn('pick_folder_and_scan error:', err);
            // Fallback to scanLibrary if dialog not supported
            await this.scanLibrary();
        }
    },

    async handleFolderScan(input) {
        if (!input || !input.files || input.files.length === 0) {
            this.appendScanLog('ℹ️ Folder picker closed without selection');
            return;
        }
        if (input._scanning) return;
        input._scanning = true;

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
            input._scanning = false;
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
        const errors = [];

        try {
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
                    if (file.size > 64 * 1024 * 1024) {
                        throw new Error(`File too large (${Math.round(file.size / 1048576)} MB > 64 MB) — skipped to avoid memory issues on Android`);
                    }
                    const sizeKb = Math.round(file.size / 1024);
                    this.appendScanLog(`⏳ Reading: ${file.name} (${sizeKb} KB)`);
                    const base64Data = await new Promise((resolve, reject) => {
                        const reader = new FileReader();
                        reader.onload = () => {
                            try {
                                const result = String(reader.result || '');
                                const commaIdx = result.indexOf(',');
                                resolve(commaIdx !== -1 ? result.substring(commaIdx + 1) : result);
                            } catch (e) {
                                reject(e);
                            }
                        };
                        reader.onerror = (e) => reject(new Error(`FileReader error: ${reader.error ? reader.error.message : e}`));
                        reader.onabort = () => reject(new Error('FileReader aborted'));
                        reader.readAsDataURL(file);
                    });

                    const result = await this.invoke('import_audio_file', {
                        name: file.name,
                        data_base64: base64Data,
                        dataBase64: base64Data
                    });
                    if (result) {
                        importedCount++;
                        this.appendScanLog(`✅ Imported into SQLite: ${result.artist || 'Unknown'} - ${result.title || file.name}`);
                        this.handleTrackImported(result);
                    } else {
                        throw new Error('No track returned from backend');
                    }
                } catch (err) {
                    const errMsg = (err && (err.message || String(err))) || 'Unknown error';
                    errors.push(`${file.name}: ${errMsg}`);
                    this.appendScanLog(`❌ Failed to import ${file.name}: ${errMsg}`);
                    console.error(`Failed to scan file ${file.name}:`, err);
                }
            }
        } finally {
            input._scanning = false;
            input.value = '';
        }

        this.finishScanProgressUI({ tracks_added: importedCount, errors });

        if (importedCount > 0) {
            this.showToast(`Scan complete: ${importedCount} track(s) added to library!`, 'success');
            this.appendScanLog(`🎉 Folder scan complete: ${importedCount} added. Reloading UI...`);
        } else {
            this.showToast('No new audio tracks could be imported.', 'warning');
        }
        try {
            await this.loadLibraryView();
            this.refreshCurrentView();
        } catch (reloadErr) {
            console.error('Error reloading library view after folder scan:', reloadErr);
        }
    },

    async handleAudioImport(input) {
        if (!input || !input.files || input.files.length === 0) {
            this.appendScanLog('ℹ️ Audio import picker closed without file selection');
            return;
        }
        if (input._importing) return;
        input._importing = true;

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
        const errors = [];

        try {
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
                    if (file.size > 64 * 1024 * 1024) {
                        throw new Error(`File too large (${Math.round(file.size / 1048576)} MB > 64 MB) — skipped to avoid memory issues on Android`);
                    }
                    const sizeKb = Math.round(file.size / 1024);
                    this.appendScanLog(`⏳ Reading bytes: ${file.name} (${sizeKb} KB)`);
                    const base64Data = await new Promise((resolve, reject) => {
                        const reader = new FileReader();
                        reader.onload = () => {
                            try {
                                const result = String(reader.result || '');
                                const commaIdx = result.indexOf(',');
                                const b64 = commaIdx !== -1 ? result.substring(commaIdx + 1) : result;
                                resolve(b64);
                            } catch (e) {
                                reject(e);
                            }
                        };
                        reader.onerror = (e) => reject(new Error(`FileReader error: ${reader.error ? reader.error.message : e}`));
                        reader.onabort = () => reject(new Error('FileReader operation aborted'));
                        reader.readAsDataURL(file);
                    });

                    this.appendScanLog(`📡 Sending buffer to Tauri backend: ${file.name} (${base64Data.length} chars base64)`);
                    const result = await this.invoke('import_audio_file', {
                        name: file.name,
                        data_base64: base64Data,
                        dataBase64: base64Data
                    });

                    if (result) {
                        successCount++;
                        const trackTitle = result.title || file.name;
                        const trackArtist = result.artist || 'Unknown Artist';
                        this.appendScanLog(`✅ SQLite inserted: "${trackTitle}" by ${trackArtist} (id: ${result.id})`);
                        this.handleTrackImported(result);
                    } else {
                        throw new Error('No track returned from backend');
                    }
                } catch (err) {
                    const errMsg = (err && (err.message || String(err))) || 'Unknown error';
                    errors.push(`${file.name}: ${errMsg}`);
                    this.appendScanLog(`❌ Ingestion failed for ${file.name}: ${errMsg}`);
                    console.error(`Failed to import ${file.name}:`, err);
                }
            }
        } finally {
            input._importing = false;
            input.value = '';
        }

        this.finishScanProgressUI({ tracks_added: successCount, errors });

        if (successCount > 0) {
            this.showToast(`Successfully imported ${successCount} track(s)!`, 'success');
            this.appendScanLog(`🎉 Import session completed: ${successCount} track(s) added, ${errors.length} error(s). Reloading UI...`);
        } else if (errors.length > 0) {
            this.showToast(`Import failed: ${errors[0]}`, 'error', 7000);
            this.appendScanLog(`⚠️ Import session finished with 0 tracks imported (${errors.length} error(s))`);
        } else {
            this.showToast('No audio files were imported.', 'warning');
        }

        try {
            await this.loadLibraryView();
            this.refreshCurrentView();
        } catch (reloadErr) {
            console.error('Error reloading library view after import:', reloadErr);
        }
    },

    handleTrackImported(track) {
        if (!track || !track.id) return;
        const existingIdx = this.tracks.findIndex(t => String(t.id) === String(track.id));
        if (existingIdx >= 0) {
            this.tracks[existingIdx] = track;
        } else {
            this.tracks.unshift(track);
        }
        if (this.activeView === 'library') {
            this.renderLibraryTracks();
        } else {
            this.refreshCurrentView();
        }
    },

    async openTagEditor(trackOrId) {
        let track = trackOrId;
        if (typeof trackOrId === 'string' || (trackOrId && !trackOrId.title)) {
            const id = typeof trackOrId === 'string' ? trackOrId : trackOrId.id;
            track = (this.tracks || []).find(t => String(t.id) === String(id));
            if (!track) {
                try {
                    track = await this.invoke('get_track', { id });
                } catch (err) {
                    console.warn('Failed to fetch track for tag editing:', err);
                }
            }
        }

        if (!track) {
            this.showToast('Track not found for editing', 'warning');
            return;
        }

        const overlayRoot = document.getElementById('overlay-root');
        if (!overlayRoot) return;

        overlayRoot.innerHTML = `
            <div id="modal-tag-editor" class="overlay modal--dialog glass-strong" onclick="if(event.target === this) window.Auralis && window.Auralis.bridge && window.Auralis.bridge.closeTagEditor()">
                <div class="modal-dialog-card neu-glass" onclick="event.stopPropagation()">
                    <div class="modal-dialog-header">
                        <div style="display: flex; align-items: center; gap: var(--space-2);">
                            <i data-lucide="tag" style="color: var(--accent); width: 22px; height: 22px;"></i>
                            <h3 style="margin: 0; font-size: var(--text-lg); font-weight: var(--font-bold); color: var(--text-1);">Edit Track Tags</h3>
                        </div>
                        <button type="button" class="btn btn-ghost btn-icon" onclick="window.Auralis && window.Auralis.bridge && window.Auralis.bridge.closeTagEditor()" aria-label="Close">
                            <i data-lucide="x"></i>
                        </button>
                    </div>
                    <form id="tag-editor-form" onsubmit="event.preventDefault(); window.Auralis && window.Auralis.bridge && window.Auralis.bridge.saveTagEditor();">
                        <input type="hidden" id="tag-edit-id" value="${this.escapeHtml(String(track.id))}" />
                        
                        <div class="form-group" style="margin-bottom: var(--space-3);">
                            <label for="tag-edit-title" style="display: block; font-size: var(--text-xs); color: var(--text-3); margin-bottom: var(--space-1);">Title</label>
                            <input type="text" id="tag-edit-title" name="title" class="input" value="${this.escapeHtml(track.title || '')}" placeholder="Track Title" required />
                        </div>

                        <div class="form-group" style="margin-bottom: var(--space-3);">
                            <label for="tag-edit-artist" style="display: block; font-size: var(--text-xs); color: var(--text-3); margin-bottom: var(--space-1);">Artist</label>
                            <input type="text" id="tag-edit-artist" name="artist" class="input" value="${this.escapeHtml(track.artist || '')}" placeholder="Artist Name" />
                        </div>

                        <div class="form-group" style="margin-bottom: var(--space-3);">
                            <label for="tag-edit-album" style="display: block; font-size: var(--text-xs); color: var(--text-3); margin-bottom: var(--space-1);">Album</label>
                            <input type="text" id="tag-edit-album" name="album" class="input" value="${this.escapeHtml(track.album || '')}" placeholder="Album Name" />
                        </div>

                        <div style="display: grid; grid-template-columns: 1fr 1fr 1fr; gap: var(--space-2); margin-bottom: var(--space-4);">
                            <div class="form-group">
                                <label for="tag-edit-genre" style="display: block; font-size: var(--text-xs); color: var(--text-3); margin-bottom: var(--space-1);">Genre</label>
                                <input type="text" id="tag-edit-genre" name="genre" class="input" value="${this.escapeHtml(track.genre || '')}" placeholder="Genre" />
                            </div>
                            <div class="form-group">
                                <label for="tag-edit-year" style="display: block; font-size: var(--text-xs); color: var(--text-3); margin-bottom: var(--space-1);">Year</label>
                                <input type="number" id="tag-edit-year" name="year" class="input" value="${track.year || ''}" placeholder="YYYY" min="1900" max="2100" />
                            </div>
                            <div class="form-group">
                                <label for="tag-edit-track-number" style="display: block; font-size: var(--text-xs); color: var(--text-3); margin-bottom: var(--space-1);">Track #</label>
                                <input type="number" id="tag-edit-track-number" name="track_number" class="input" value="${track.track_number || ''}" placeholder="1" min="1" max="999" />
                            </div>
                        </div>

                        <div style="display: flex; justify-content: flex-end; gap: var(--space-2);">
                            <button type="button" class="btn btn-secondary neu" onclick="window.Auralis && window.Auralis.bridge && window.Auralis.bridge.closeTagEditor()">Cancel</button>
                            <button type="submit" class="btn btn-primary neu" id="tag-editor-save-btn">
                                <i data-lucide="check"></i>
                                Save
                            </button>
                        </div>
                    </form>
                </div>
            </div>
        `;

        if (window.lucide) window.lucide.createIcons();
    },

    closeTagEditor() {
        const overlayRoot = document.getElementById('overlay-root');
        if (overlayRoot) {
            const modal = overlayRoot.querySelector('#modal-tag-editor');
            if (modal) {
                overlayRoot.innerHTML = '';
            }
        }
    },

    async saveTagEditor() {
        const idInput = document.getElementById('tag-edit-id');
        const titleInput = document.getElementById('tag-edit-title');
        const artistInput = document.getElementById('tag-edit-artist');
        const albumInput = document.getElementById('tag-edit-album');
        const genreInput = document.getElementById('tag-edit-genre');
        const yearInput = document.getElementById('tag-edit-year');
        const trackNumInput = document.getElementById('tag-edit-track-number');

        if (!idInput || !titleInput) return;

        const id = idInput.value;
        const title = titleInput.value.trim();
        if (!title) {
            this.showToast('Title cannot be empty', 'warning');
            return;
        }

        const artist = artistInput && artistInput.value.trim() ? artistInput.value.trim() : null;
        const album = albumInput && albumInput.value.trim() ? albumInput.value.trim() : null;
        const genre = genreInput && genreInput.value.trim() ? genreInput.value.trim() : null;
        const yearVal = yearInput && yearInput.value.trim() ? parseInt(yearInput.value.trim(), 10) : null;
        const year = yearVal && !isNaN(yearVal) && yearVal > 0 ? yearVal : null;
        const trackNumVal = trackNumInput && trackNumInput.value.trim() ? parseInt(trackNumInput.value.trim(), 10) : null;
        const trackNumber = trackNumVal && !isNaN(trackNumVal) && trackNumVal > 0 ? trackNumVal : null;

        const saveBtn = document.getElementById('tag-editor-save-btn');
        if (saveBtn) {
            saveBtn.disabled = true;
            saveBtn.innerHTML = `<i data-lucide="loader-2" class="spin"></i> Saving...`;
            if (window.lucide) window.lucide.createIcons();
        }

        try {
            const updatedTrack = await this.invoke('update_track_metadata', {
                id,
                update: {
                    title,
                    artist,
                    album,
                    genre,
                    year,
                    track_number: trackNumber
                }
            });

            this.showToast('Track tags saved successfully', 'success');
            this.closeTagEditor();

            if (updatedTrack) {
                this.handleTrackImported(updatedTrack);
            }
        } catch (err) {
            console.error('Failed to save track metadata:', err);
            const errMsg = err && err.message ? err.message : String(err);
            this.showToast(`Failed to save metadata: ${errMsg}`, 'error');
            if (saveBtn) {
                saveBtn.disabled = false;
                saveBtn.innerHTML = `<i data-lucide="check"></i> Save`;
                if (window.lucide) window.lucide.createIcons();
            }
        }
    }
};
