/**
 * Android Storage Adapter
 * Handles Android MediaStore system-wide scanning, sandboxed app directories,
 * and SAF (Storage Access Framework) file/folder input picker logic.
 */

/**
 * Read a File object as base64 string (stripping data URL prefix).
 * @param {File} file
 * @returns {Promise<string>}
 */
export function readFileAsBase64(file) {
    return new Promise((resolve, reject) => {
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
        reader.onabort = () => reject(new Error('FileReader operation aborted'));
        reader.readAsDataURL(file);
    });
}

export const androidStorageAdapter = {
    name: 'android',

    /**
     * Ensure clean <input type="file"> elements exist in the DOM with direct event listeners
     * for Android 14/15/16 WebView / Scoped Storage compatibility without clipped/z-index issues.
     */
    ensureFileInput(ctx) {
        const bridge = ctx || (typeof window !== 'undefined' && window.Auralis && window.Auralis.bridge);
        if (typeof document === 'undefined' || typeof document.getElementById !== 'function' || typeof document.createElement !== 'function') {
            return;
        }

        let audioInput = document.getElementById('global-audio-import-input');
        if (!audioInput) {
            audioInput = document.createElement('input');
            audioInput.type = 'file';
            audioInput.id = 'global-audio-import-input';
            audioInput.accept = 'audio/*';
            audioInput.multiple = true;
            audioInput.style.display = 'none';
            if (document.body && typeof document.body.appendChild === 'function') {
                document.body.appendChild(audioInput);
            }
        }
        const isAudioBound = audioInput?.dataset ? audioInput.dataset.bound : audioInput?.getAttribute?.('data-bound');
        if (audioInput && !isAudioBound) {
            if (audioInput.dataset) audioInput.dataset.bound = 'true';
            else audioInput.setAttribute?.('data-bound', 'true');
            if (typeof audioInput.addEventListener === 'function') {
                audioInput.addEventListener('change', (e) => {
                    const targetBridge = (bridge && typeof bridge.handleAudioImport === 'function')
                        ? bridge
                        : (typeof window !== 'undefined' && window.Auralis && window.Auralis.bridge);
                    if (targetBridge && typeof targetBridge.handleAudioImport === 'function') {
                        targetBridge.handleAudioImport(e.target);
                    }
                });
            }
        }

        let folderInput = document.getElementById('global-folder-scan-input');
        if (!folderInput) {
            folderInput = document.createElement('input');
            folderInput.type = 'file';
            folderInput.id = 'global-folder-scan-input';
            folderInput.webkitdirectory = true;
            folderInput.setAttribute('directory', '');
            folderInput.multiple = true;
            folderInput.style.display = 'none';
            if (document.body && typeof document.body.appendChild === 'function') {
                document.body.appendChild(folderInput);
            }
        }
        const isFolderBound = folderInput?.dataset ? folderInput.dataset.bound : folderInput?.getAttribute?.('data-bound');
        if (folderInput && !isFolderBound) {
            if (folderInput.dataset) folderInput.dataset.bound = 'true';
            else folderInput.setAttribute?.('data-bound', 'true');
            if (typeof folderInput.addEventListener === 'function') {
                folderInput.addEventListener('change', (e) => {
                    const targetBridge = (bridge && typeof bridge.handleFolderScan === 'function')
                        ? bridge
                        : (typeof window !== 'undefined' && window.Auralis && window.Auralis.bridge);
                    if (targetBridge && typeof targetBridge.handleFolderScan === 'function') {
                        targetBridge.handleFolderScan(e.target);
                    }
                });
            }
        }

        // Global fallback listener on document for change events on file inputs
        const isBodyBound = document.body?.dataset ? document.body.dataset.audioImportFallbackBound : document.body?.getAttribute?.('data-audio-fallback-bound');
        if (document.body && !isBodyBound) {
            if (document.body.dataset) document.body.dataset.audioImportFallbackBound = 'true';
            else document.body.setAttribute?.('data-audio-fallback-bound', 'true');
            if (typeof document.addEventListener === 'function') {
                document.addEventListener('change', (e) => {
                    const target = e.target;
                    if (!target) return;
                    const targetBridge = (bridge && typeof bridge.handleAudioImport === 'function')
                        ? bridge
                        : (typeof window !== 'undefined' && window.Auralis && window.Auralis.bridge);
                    if (!targetBridge) return;
                    if (target.id === 'global-audio-import-input') {
                        targetBridge.handleAudioImport(target);
                    } else if (target.id === 'global-folder-scan-input') {
                        targetBridge.handleFolderScan(target);
                    }
                });
            }
        }
    },

    /**
     * Scan Android storage (system MediaStore + sandboxed app directories: app_data_dir/music, downloads).
     */
    async scanLibrary(ctx, paths = null) {
        const bridge = ctx || (typeof window !== 'undefined' && window.Auralis && window.Auralis.bridge);
        bridge?.appendScanLog?.('🚀 Initiating Android MediaStore & storage scan...');
        bridge?.showToast?.('Scanning storage for audio tracks...', 'info');
        bridge?.updateScanProgressUI?.({
            title: 'Scanning Storage',
            subtitle: 'Searching MediaStore & app sandbox...',
            current: 0,
            percentage: 0
        });

        try {
            const summary = await bridge?.invoke?.('scan_library_paths', { paths: paths || null });
            if (summary) {
                const added = summary.tracks_added ?? 0;
                const updated = summary.tracks_updated ?? 0;
                const errCount = (summary.errors && summary.errors.length) || 0;
                bridge?.appendScanLog?.(`🎉 Complete: ${added} added, ${updated} updated, ${errCount} errors`);

                if (added === 0 && updated === 0 && (!paths || paths.length === 0)) {
                    bridge?.showToast?.('No audio files in app sandbox. Tap "Import Audio" to add songs from device storage.', 'info');
                    bridge?.appendScanLog?.('ℹ️ Android Scoped Storage: Tap "Import Audio" to select audio files from your device storage');
                } else {
                    bridge?.showToast?.(`Scan complete: ${added} added, ${updated} updated`, 'success');
                }
                await bridge?.loadLibraryView?.();
                bridge?.refreshCurrentView?.();
                return summary;
            }
            return null;
        } catch (err) {
            bridge?.appendScanLog?.(`❌ Scan error: ${err}`);
            bridge?.showToast?.(`Scan error: ${err}`, 'error');
            throw err;
        }
    },

    /**
     * Trigger Android SAF audio file picker.
     */
    async triggerAudioImport(ctx) {
        const bridge = ctx || (typeof window !== 'undefined' && window.Auralis && window.Auralis.bridge);
        bridge?.appendScanLog?.('📥 Opening Android SAF audio file picker...');
        this.ensureFileInput(bridge);
        const input = typeof document !== 'undefined' ? document.getElementById('global-audio-import-input') : null;
        if (input) {
            input.value = ''; // Crucial for Android: reset value so selecting same file fires change
            input.click();
            return;
        }
        bridge?.showToast?.('Audio import input not found in DOM', 'warning');
    },

    /**
     * Trigger Android folder scan (MediaStore/sandboxed scan + SAF folder picker).
     */
    async triggerFolderScan(ctx) {
        const bridge = ctx || (typeof window !== 'undefined' && window.Auralis && window.Auralis.bridge);
        bridge?.appendScanLog?.('📂 Requesting music storage scan (Android MediaStore + SAF)...');

        // 1. Run the system MediaStore & sandboxed storage scan directly
        await this.scanLibrary(bridge);

        // 2. Also offer file picker if user wanted manual folder/file selection via SAF
        this.ensureFileInput(bridge);
        const folderInput = typeof document !== 'undefined' ? document.getElementById('global-folder-scan-input') : null;
        if (folderInput) {
            try {
                folderInput.value = '';
                folderInput.click();
            } catch (_) {
                await this.triggerAudioImport(bridge);
            }
        } else {
            await this.triggerAudioImport(bridge);
        }
    },

    /**
     * Handle folder scan change event from SAF directory input.
     */
    async handleFolderScan(ctx, input) {
        const bridge = ctx || (typeof window !== 'undefined' && window.Auralis && window.Auralis.bridge);
        if (!input || !input.files || input.files.length === 0) {
            bridge?.appendScanLog?.('ℹ️ Folder picker closed without selection');
            return;
        }
        if (input._scanning) return;
        input._scanning = true;

        const allFiles = Array.from(input.files);
        bridge?.appendScanLog?.(`📱 SAF picker returned ${allFiles.length} raw files`);
        const audioExtensions = ['.mp3', '.flac', '.wav', '.m4a', '.aac', '.ogg', '.opus', '.wma'];
        const audioFiles = allFiles.filter(file => {
            const name = file.name.toLowerCase();
            return (file.type && file.type.startsWith('audio/')) || audioExtensions.some(ext => name.endsWith(ext));
        });

        if (audioFiles.length === 0) {
            bridge?.appendScanLog?.('⚠️ No supported audio format files found in selected directory');
            bridge?.showToast?.('No audio files found in selected folder.', 'info');
            input._scanning = false;
            input.value = '';
            return;
        }

        bridge?.appendScanLog?.(`🎵 Found ${audioFiles.length} audio file(s) to import`);
        bridge?.showToast?.(`Found ${audioFiles.length} audio file(s). Scanning & importing...`, 'info');
        bridge?.updateScanProgressUI?.({
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
                bridge?.updateScanProgressUI?.({
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
                    bridge?.appendScanLog?.(`⏳ Reading: ${file.name} (${sizeKb} KB)`);
                    const base64Data = await readFileAsBase64(file);

                    const result = await bridge?.invoke?.('import_audio_file', {
                        name: file.name,
                        data_base64: base64Data,
                        dataBase64: base64Data
                    });
                    if (result) {
                        importedCount++;
                        bridge?.appendScanLog?.(`✅ Imported into SQLite: ${result.artist || 'Unknown'} - ${result.title || file.name}`);
                        bridge?.handleTrackImported?.(result);
                    } else {
                        throw new Error('No track returned from backend');
                    }
                } catch (err) {
                    const errMsg = (err && (err.message || String(err))) || 'Unknown error';
                    errors.push(`${file.name}: ${errMsg}`);
                    bridge?.appendScanLog?.(`❌ Failed to import ${file.name}: ${errMsg}`);
                    console.error(`Failed to scan file ${file.name}:`, err);
                }
            }
        } finally {
            input._scanning = false;
            input.value = '';
        }

        bridge?.finishScanProgressUI?.({ tracks_added: importedCount, errors });

        if (importedCount > 0) {
            bridge?.showToast?.(`Scan complete: ${importedCount} track(s) added to library!`, 'success');
            bridge?.appendScanLog?.(`🎉 Folder scan complete: ${importedCount} added. Reloading UI...`);
        } else {
            bridge?.showToast?.('No new audio tracks could be imported.', 'warning');
        }
        try {
            await bridge?.loadLibraryView?.();
            bridge?.refreshCurrentView?.();
        } catch (reloadErr) {
            console.error('Error reloading library view after folder scan:', reloadErr);
        }
    },

    /**
     * Handle audio import change event from SAF file input.
     */
    async handleAudioImport(ctx, input) {
        const bridge = ctx || (typeof window !== 'undefined' && window.Auralis && window.Auralis.bridge);
        if (!input || !input.files || input.files.length === 0) {
            bridge?.appendScanLog?.('ℹ️ Audio import picker closed without file selection');
            return;
        }
        if (input._importing) return;
        input._importing = true;

        const files = Array.from(input.files);
        bridge?.appendScanLog?.(`📥 Selected ${files.length} audio file(s) for direct import`);
        bridge?.showToast?.(`Importing ${files.length} audio file(s)...`, 'info');
        bridge?.updateScanProgressUI?.({
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
                bridge?.updateScanProgressUI?.({
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
                    bridge?.appendScanLog?.(`⏳ Reading bytes: ${file.name} (${sizeKb} KB)`);
                    const base64Data = await readFileAsBase64(file);

                    bridge?.appendScanLog?.(`📡 Sending buffer to Tauri backend: ${file.name} (${base64Data.length} chars base64)`);
                    const result = await bridge?.invoke?.('import_audio_file', {
                        name: file.name,
                        data_base64: base64Data,
                        dataBase64: base64Data
                    });

                    if (result) {
                        successCount++;
                        const trackTitle = result.title || file.name;
                        const trackArtist = result.artist || 'Unknown Artist';
                        bridge?.appendScanLog?.(`✅ SQLite inserted: "${trackTitle}" by ${trackArtist} (id: ${result.id})`);
                        bridge?.handleTrackImported?.(result);
                    } else {
                        throw new Error('No track returned from backend');
                    }
                } catch (err) {
                    const errMsg = (err && (err.message || String(err))) || 'Unknown error';
                    errors.push(`${file.name}: ${errMsg}`);
                    bridge?.appendScanLog?.(`❌ Ingestion failed for ${file.name}: ${errMsg}`);
                    console.error(`Failed to import ${file.name}:`, err);
                }
            }
        } finally {
            input._importing = false;
            input.value = '';
        }

        bridge?.finishScanProgressUI?.({ tracks_added: successCount, errors });

        if (successCount > 0) {
            bridge?.showToast?.(`Successfully imported ${successCount} track(s)!`, 'success');
            bridge?.appendScanLog?.(`🎉 Import session completed: ${successCount} track(s) added, ${errors.length} error(s). Reloading UI...`);
        } else if (errors.length > 0) {
            bridge?.showToast?.(`Import failed: ${errors[0]}`, 'error', 7000);
            bridge?.appendScanLog?.(`⚠️ Import session finished with 0 tracks imported (${errors.length} error(s))`);
        } else {
            bridge?.showToast?.('No audio files were imported.', 'warning');
        }

        try {
            await bridge?.loadLibraryView?.();
            bridge?.refreshCurrentView?.();
        } catch (reloadErr) {
            console.error('Error reloading library view after import:', reloadErr);
        }
    }
};
