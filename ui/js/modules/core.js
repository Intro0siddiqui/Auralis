/**
 * Core Bridge Module
 * Handles constructor, state initialization, Tauri events listener setup,
 * event emitter (on/emit), invoke wrapper, and DOM lifecycle events.
 */

export class Bridge {
    constructor() {
        this.listeners = {};
        this.tauriAvailable = false;
        this.tracks = [];
        this.activeView = 'home';
        this.initialized = false;
        this.currentSettings = null;
        this._lastSearchResults = [];
        this.init();
    }
}

export const coreMethods = {
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

        this.initGlobalErrorHandlers();
        this.initKeyboardHandler();
        this.bindHTMXEvents();
        this.ensureFileInput();
        this.refreshCurrentView();
    },

    initGlobalErrorHandlers() {
        if (window._globalErrorHandlersBound) return;
        window._globalErrorHandlersBound = true;

        window.addEventListener('error', (event) => {
            const msg = event.message || 'JavaScript execution error';
            console.error('Captured window error:', event);
            if (typeof this.showToast === 'function') {
                this.showToast(`Error: ${msg}`, 'error');
            }
            if (typeof this.appendScanLog === 'function') {
                this.appendScanLog(`❌ JS Error: ${msg} (${event.filename || 'app'}:${event.lineno || '0'})`);
            }
        });

        window.addEventListener('unhandledrejection', (event) => {
            const reason = event.reason?.message || event.reason || 'Unhandled Promise Error';
            console.error('Captured unhandled rejection:', event.reason);
            if (typeof this.showToast === 'function') {
                this.showToast(`Error: ${reason}`, 'error');
            }
            if (typeof this.appendScanLog === 'function') {
                this.appendScanLog(`❌ Promise Rejection: ${reason}`);
            }
        });
    },

    ensureFileInput() {
        // Guarantee clean <input type="file"> elements exist in the DOM with direct event listeners
        // for Android 16 WebView / Scoped Storage compatibility without clipped/z-index issues.
        if (!document || typeof document.getElementById !== 'function' || typeof document.createElement !== 'function') return;

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
        if (audioInput && !audioInput.dataset.bound) {
            audioInput.dataset.bound = 'true';
            audioInput.addEventListener('change', (e) => this.handleAudioImport(e.target));
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
        if (folderInput && !folderInput.dataset.bound) {
            folderInput.dataset.bound = 'true';
            folderInput.addEventListener('change', (e) => this.handleFolderScan(e.target));
        }

        // Global fallback listener on document for change events on file inputs
        if (document.body && !document.body.dataset.audioImportFallbackBound) {
            document.body.dataset.audioImportFallbackBound = 'true';
            document.addEventListener('change', (e) => {
                const target = e.target;
                if (!target) return;
                if (target.id === 'global-audio-import-input') {
                    this.handleAudioImport(target);
                } else if (target.id === 'global-folder-scan-input') {
                    this.handleFolderScan(target);
                }
            });
        }
    },

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
    },

    initKeyboardHandler() {
        if (window.visualViewport) {
            window.visualViewport.addEventListener('resize', () => {
                const isKeyboardOpen = window.visualViewport.height < window.innerHeight - 150;
                if (document.body && document.body.classList) {
                    document.body.classList.toggle('keyboard-open', isKeyboardOpen);
                }
            });
        }
    },

    bindHTMXEvents() {
        if (document.body && typeof document.body.addEventListener === 'function') {
            document.body.addEventListener('htmx:afterSwap', () => {
                if (window.lucide) window.lucide.createIcons();
                this.ensureFileInput();
                this.refreshCurrentView();
            });
        }
    },

    on(event, callback) {
        if (!this.listeners[event]) this.listeners[event] = [];
        this.listeners[event].push(callback);
    },

    emit(event, data) {
        if (this.listeners[event]) {
            this.listeners[event].forEach(cb => cb(data));
        }
    }
};
