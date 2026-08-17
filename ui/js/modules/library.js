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
            this.appendScanLog(`⚠️ Audio picker error: ${err}`);
            console.warn('pick_audio_files_and_import error:', err);
        }
    },

    async triggerFolderScan() {
        this.appendScanLog('📂 Requesting music storage scan...');
        // On mobile: DOM file input is the only reliable path
        this.ensureFileInput();
        const input = document.getElementById('global-folder-scan-input') || document.getElementById('global-audio-import-input');
        if (input) {
            input.value = '';
            input.click();
            return;
        }
        // Desktop-only: native folder dialog
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
        }
    },

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
    },

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
    },

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
};
