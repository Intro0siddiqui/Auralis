/**
 * Library Module
 * Handles file importing, folder scanning, tag editing, and library updates.
 * Storage and scanning operations are delegated to platform-specific storage adapters.
 */

import { getStorageAdapter } from './storage/index.js';

export const libraryMethods = {
    /**
     * Trigger library scan over configured or given paths.
     * Delegates to active platform storage adapter (desktop vs android).
     */
    async scanLibrary(paths = null) {
        return await getStorageAdapter().scanLibrary(this, paths);
    },

    /**
     * Trigger audio file import.
     * Desktop: native OS file dialog via 'pick_audio_files_and_import'.
     * Android: SAF file picker via hidden <input type="file" accept="audio/*">.
     */
    async triggerAudioImport() {
        return await getStorageAdapter().triggerAudioImport(this);
    },

    /**
     * Trigger music folder scan.
     * Desktop: native OS folder picker via 'pick_folder_and_scan'.
     * Android: MediaStore/sandboxed storage scan + SAF folder picker.
     */
    async triggerFolderScan() {
        return await getStorageAdapter().triggerFolderScan(this);
    },

    /**
     * Handle folder scan change event from file input.
     * Ingests files into the library via backend.
     */
    async handleFolderScan(input) {
        return await getStorageAdapter().handleFolderScan(this, input);
    },

    /**
     * Handle direct audio file import from file input.
     * Ingests files into the library via backend.
     */
    async handleAudioImport(input) {
        return await getStorageAdapter().handleAudioImport(this, input);
    },

    /**
     * Ensure file input elements exist in DOM and are bound to change listeners.
     */
    ensureFileInput() {
        return getStorageAdapter().ensureFileInput(this);
    },

    /**
     * Ingest or update imported track into in-memory track collection and refresh UI.
     */
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
