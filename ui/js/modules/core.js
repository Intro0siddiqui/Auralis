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
    extractErrorMessage(p, fallback = 'Stream error') {
        if (!p) return fallback;
        if (typeof p === 'string') return p;
        return p.error || p.error_message || p.message || fallback;
    },

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
                    const track = (event.payload && event.payload.track) ? event.payload.track : event.payload;
                    this.emit('playback:track', track);
                    this.updatePlayerBar(track);
                });

                await tauriListen('playback:queue_updated', (event) => {
                    this.emit('playback:queue', event.payload);
                });

                await tauriListen('playback:progress', (event) => {
                    this.emit('playback:progress', event.payload);
                });

                await tauriListen('playback:error', (event) => {
                    const msg = typeof event.payload === 'string'
                        ? event.payload
                        : this.extractErrorMessage(event.payload, JSON.stringify(event.payload));
                    this.emit('playback:error', msg);
                    console.error('[playback:error]', msg);
                    this.showToast(`Playback failed: ${msg}`, 'error', 6000);
                    try { window.__auralisLastPlaybackError = { at: new Date().toISOString(), msg }; } catch (_) {}
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
                        const rawErr = this.extractErrorMessage(p, 'Stream error');
                        // 403 auto-retry: if downloads.js has pending context with retryClients, suppress immediate failure toast
                        // (downloads.js listener already emitted via this.emit above and will re-resolve with next orderedClient TV→ANDROID+pot→WEB_SAFARI)
                        const is403 = rawErr.includes('403') || rawErr.includes('Forbidden') || rawErr.includes('HTTP 403');
                        if (is403) {
                            try {
                                const map = this._pendingDownloadContexts || window.__auralisPendingDownloadContexts;
                                const retrySet = window.__auralisDownloadRetryingIds;
                                const ctx = map ? map.get(p.id) : null;
                                const hasRetry = ctx && ctx.retryCount < 1 && (ctx.resolved?.retryClients?.length || ctx.resolved?.orderedClients?.length);
                                const isRetrying = retrySet ? retrySet.has(p.id) : false;
                                if (hasRetry || isRetrying) {
                                    // Defer failure toast: retry is in progress (downloads.js will toast retrying)
                                    console.warn(`[core] 403 for ${p.id} — suppressing failure toast, auto-retry in progress (hasRetry=${hasRetry}, isRetrying=${isRetrying})`);
                                    // Still update UI to show retrying state
                                    if (p) this.updateDownloadProgressUI(p);
                                    return;
                                }
                                // Fallback: if no context but error is 403, attempt direct retry via downloads.js helper if available
                                if (typeof this._handle403AutoRetry === 'function') {
                                    // Let the downloads.js handler (already fired via emit) attempt; if it returns without retry, fall through to toast
                                    // No-op here — handler already ran synchronously via emit
                                }
                            } catch (_) {}
                        }
                        // Fix: backend field is `error`, not `error_message` — support both.
                        // Truncate for toast but keep full error in console/UI.
                        const toastMsg = rawErr.length > 180 ? rawErr.slice(0, 180) + '…' : rawErr;
                        const host = (() => { try { return new URL(p.url || '').host || 'unknown'; } catch (_) { return 'unknown'; } })();
                        this.showToast(`Download failed [${host}]: ${toastMsg}`, 'error', 8000);
                        // Verbose, mirrored to logcat via webview-log-js-console-messages (chromium tag)
                        console.groupCollapsed(`%c[Download Failed] ${p.title || p.url || 'unknown'}`, 'color:#ff4d4f;font-weight:bold');
                        console.error('DIAGNOSTIC download_failed', {
                            id: p.id,
                            title: p.title,
                            url: p.url,
                            host,
                            status: p.status,
                            error: rawErr,
                            downloaded: p.downloaded_bytes,
                            total: p.total_bytes,
                            progress: p.progress,
                            platform: p.platform,
                            format: p.format,
                            output_path: p.output_path,
                        });
                        console.error(`Full error: ${rawErr}`);
                        console.error(`URL: ${p.url}`);
                        console.error(`Hint: ${rawErr.includes('403') ? '403 Forbidden [rr1---sn-gwpa-cived] — googlevideo rejected UA/Referer/Origin/PO-token or URL expired (2026 Jio now gates TV too). Try re-resolving with ANDROID+pot or WEB_SAFARI; set youtube_po_token via BgUtils mint or Settings cookie. This download used headers from youtube.js winningClient; check that UA matches InnerTube client. URL expires ~6h.' : rawErr.includes('timeout') ? 'Timeout — network stalled or host unreachable. Check connection & retry.' : 'See error body above; copy the full message from the Downloads list.'}`);
                        console.groupEnd();
                        // Keep a persistent in-memory log for "Copy diagnostics" button
                        try {
                            window.__auralisDownloadDiagnostics = window.__auralisDownloadDiagnostics || [];
                            window.__auralisDownloadDiagnostics.push({ at: new Date().toISOString(), payload: p, rawErr });
                            if (window.__auralisDownloadDiagnostics.length > 50) window.__auralisDownloadDiagnostics.shift();
                        } catch (_) {}
                    } else if (p && p.status === 'cancelled') {
                        this.showToast(`Download cancelled: ${p.title || 'track'}`, 'info');
                    }
                    if (p) {
                        this.updateDownloadProgressUI(p);
                    }
                });

                await tauriListen('download:diagnostic', (event) => {
                    const msg = typeof event.payload === 'string' ? event.payload : JSON.stringify(event.payload);
                    console.error(`[download:diagnostic] ${msg}`);
                    // also keep for adb logcat visibility
                    console.warn(`DIAGNOSTIC ${msg}`);
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
            const msg = typeof err === 'string' ? err : this.extractErrorMessage(err, err.message || JSON.stringify(err));
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
            document.body.addEventListener('htmx:afterSwap', (evt) => {
                if (window.htmx && evt.target) {
                    window.htmx.process(evt.target);
                }
                if (window.lucide) window.lucide.createIcons();
                this.ensureFileInput();
                this.refreshCurrentView();
            });
            document.body.addEventListener('htmx:restored', (evt) => {
                if (window.htmx && (evt.target || document.body)) {
                    window.htmx.process(evt.target || document.body);
                }
                if (window.lucide) window.lucide.createIcons();
                this.ensureFileInput();
                this.refreshCurrentView();
            });
        }
        // BF-cache restore when returning from external Files app (Android SAF) — htmx:afterSwap does not fire
        window.addEventListener('pageshow', (e) => {
            if (window.htmx) {
                window.htmx.process(document.body);
            }
            if (e.persisted) {
                if (window.lucide) window.lucide.createIcons();
                this.ensureFileInput();
                this.refreshCurrentView();
            }
        });
        document.addEventListener('visibilitychange', () => {
            if (document.visibilityState === 'visible' && document.querySelector('.page-downloads')) {
                // Re-ensure Download handlers after Files app returns without firing htmx events
                this.loadDownloadView();
            }
        });
        // Delegated form submit handler — dispatches custom document events for download and search forms
        document.addEventListener('submit', (e) => {
            const t = e.target;
            if (!t) return;
            if (t.id === 'download-form') {
                e.preventDefault();
                e.stopPropagation();
                document.dispatchEvent(new CustomEvent('auralis:submit:download', { detail: { form: t } }));
            } else if (t.id === 'youtube-search-form') {
                e.preventDefault();
                e.stopPropagation();
                document.dispatchEvent(new CustomEvent('auralis:submit:search', { detail: { form: t } }));
            }
        }, true);

        // Delegated track play handlers across all views (Home, Library, Search)
        if (document.body && !document.body.dataset.playDelegationBound) {
            document.body.dataset.playDelegationBound = 'true';
            let _lastDelegatedPlayTime = 0;
            const handlePlayDelegate = (e) => {
                if (e.target.closest && e.target.closest('.track-row-actions')) {
                    const playBtn = e.target.closest('[data-role="play-btn"]');
                    if (playBtn) {
                        e.preventDefault(); e.stopPropagation();
                        const tid = playBtn.dataset.trackId;
                        const now = Date.now();
                        if (tid && (now - _lastDelegatedPlayTime >= 350)) {
                            _lastDelegatedPlayTime = now;
                            this.playTrack(tid);
                        }
                    }
                    return;
                }
                const playEl = e.target.closest && e.target.closest('[data-role="play-row"], [data-role="play-card"]');
                if (!playEl) return;
                const tid = playEl.dataset.trackId;
                const now = Date.now();
                if (tid && (now - _lastDelegatedPlayTime >= 350)) {
                    e.preventDefault();
                    _lastDelegatedPlayTime = now;
                    this.playTrack(tid);
                }
            };
            document.addEventListener('click', handlePlayDelegate);
            document.addEventListener('touchend', handlePlayDelegate, { passive: false });
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
