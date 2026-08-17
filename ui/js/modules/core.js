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

        this.initKeyboardHandler();
        this.bindHTMXEvents();
        this.ensureFileInput();
        this.refreshCurrentView();
    },

    ensureFileInput() {
        // Guarantee a real <input type="file"> is in the DOM so that
        // <label for="global-audio-import-input"> fires the change event
        // on Android 16 WebView without triggering synthetic .click().
        if (!document || typeof document.getElementById !== 'function' || typeof document.createElement !== 'function') return;
        if (!document.getElementById('global-audio-import-input')) {
            const input = document.createElement('input');
            input.type = 'file';
            input.id = 'global-audio-import-input';
            input.accept = 'audio/*';
            input.multiple = true;
            input.style.cssText = 'position:fixed;top:0;left:0;width:1px;height:1px;opacity:0.01;clip-path:inset(50%);z-index:-1;';
            input.addEventListener('change', (e) => this.handleAudioImport(e.target));
            if (document.body && typeof document.body.appendChild === 'function') {
                document.body.appendChild(input);
            }
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
