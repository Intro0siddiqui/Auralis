/**
 * Desktop Storage Adapter
 * Handles desktop native folder dialog via Tauri 'pick_folder_and_scan',
 * native audio file picker dialog via 'pick_audio_files_and_import',
 * and direct path scanning.
 */

import { androidStorageAdapter } from './android.js';

export const desktopStorageAdapter = {
    name: 'desktop',

    /**
     * Ensure file input elements exist in DOM for fallback or automated tests.
     */
    ensureFileInput(ctx) {
        return androidStorageAdapter.ensureFileInput(ctx);
    },

    /**
     * Scan desktop library paths (dirs::audio_dir(), download_dir(), app_data_dir/music).
     */
    async scanLibrary(ctx, paths = null) {
        const bridge = ctx || (typeof window !== 'undefined' && window.Auralis && window.Auralis.bridge);
        bridge?.appendScanLog?.('🚀 Initiating desktop storage scan...');
        bridge?.showToast?.('Scanning storage for audio tracks...', 'info');
        bridge?.updateScanProgressUI?.({
            title: 'Scanning Storage',
            subtitle: 'Searching audio paths...',
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
                    bridge?.showToast?.('Scan complete: library is up to date', 'info');
                    bridge?.appendScanLog?.('ℹ️ Desktop library scan completed — standard audio directories checked');
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
     * Trigger native desktop OS folder dialog (rfd/GTK/KDE/Explorer/Finder) via 'pick_folder_and_scan'.
     */
    async triggerFolderScan(ctx) {
        const bridge = ctx || (typeof window !== 'undefined' && window.Auralis && window.Auralis.bridge);
        bridge?.appendScanLog?.('📂 Opening native folder dialog...');

        try {
            bridge?.showToast?.('Select a folder to scan for music...', 'info');
            const summary = await bridge?.invoke?.('pick_folder_and_scan');
            if (summary !== undefined && summary !== null) {
                const added = summary.tracks_added ?? 0;
                const updated = summary.tracks_updated ?? 0;
                bridge?.appendScanLog?.(`🎉 Scan complete: ${added} added, ${updated} updated`);
                bridge?.showToast?.(`Scan complete: ${added} added, ${updated} updated`, 'success');
                await bridge?.loadLibraryView?.();
                bridge?.refreshCurrentView?.();
                return summary;
            } else if (summary === null) {
                bridge?.appendScanLog?.('ℹ️ Folder picker was cancelled');
                return null;
            }
        } catch (err) {
            console.warn('pick_folder_and_scan error:', err);
            bridge?.appendScanLog?.(`⚠️ Folder picker error: ${err}, falling back to standard scan`);
            return await this.scanLibrary(bridge);
        }
    },

    /**
     * Trigger native desktop OS audio file picker dialog via 'pick_audio_files_and_import'.
     */
    async triggerAudioImport(ctx) {
        const bridge = ctx || (typeof window !== 'undefined' && window.Auralis && window.Auralis.bridge);
        bridge?.appendScanLog?.('📥 Opening native audio file picker...');

        try {
            bridge?.showToast?.('Select audio files to import...', 'info');
            const summary = await bridge?.invoke?.('pick_audio_files_and_import');
            if (summary !== undefined && summary !== null) {
                const added = summary.tracks_added ?? 0;
                const updated = summary.tracks_updated ?? 0;
                bridge?.appendScanLog?.(`🎉 Import complete: ${added} added, ${updated} updated`);
                bridge?.showToast?.(`Import complete: ${added} added, ${updated} updated`, 'success');
                await bridge?.loadLibraryView?.();
                bridge?.refreshCurrentView?.();
                return summary;
            } else if (summary === null) {
                bridge?.appendScanLog?.('ℹ️ Audio picker was cancelled');
                return null;
            }
        } catch (err) {
            const msg = err && err.message ? err.message : String(err);
            bridge?.appendScanLog?.(`⚠️ Audio picker error: ${msg}`);
            bridge?.showToast?.(`Audio picker error: ${msg}`, 'error');
            console.warn('pick_audio_files_and_import error:', err);

            // Fallback to DOM input if native picker failed or unsupported
            this.ensureFileInput(bridge);
            const input = typeof document !== 'undefined' ? document.getElementById('global-audio-import-input') : null;
            if (input) {
                input.value = '';
                input.click();
            }
        }
    },

    /**
     * Handle folder scan change event (delegates to standard file ingestion logic).
     */
    async handleFolderScan(ctx, input) {
        return await androidStorageAdapter.handleFolderScan(ctx, input);
    },

    /**
     * Handle audio import change event (delegates to standard file ingestion logic).
     */
    async handleAudioImport(ctx, input) {
        return await androidStorageAdapter.handleAudioImport(ctx, input);
    }
};
