/**
 * Downloads Module
 * Handles download queue, YouTube resolution glue, and sync triggers.
 */

export const downloadMethods = {
    // Pending download contexts for 403 auto-retry (id -> { resolved, opts, originalUrl, format, retryCount })
    _pendingDownloadContexts: null,
    _downloadRetryListenerBound: false,
    _downloadSubmitListenersBound: false,

    _ensurePendingMap() {
        if (!this._pendingDownloadContexts) this._pendingDownloadContexts = new Map();
        // expose globally so core.js can read same map (both run on same Bridge instance)
        try { window.__auralisPendingDownloadContexts = this._pendingDownloadContexts; } catch (_) {}
        return this._pendingDownloadContexts;
    },

    _ensureDownloadRetryListener() {
        if (this._downloadRetryListenerBound) return;
        this._downloadRetryListenerBound = true;
        // Subscribe to Bridge's download:completed for 403 auto-retry (once, minimal invasive)
        try {
            this.on('download:completed', (p) => {
                if (!p) return;
                // Ensure failed UI is surfaced even when completed carries failed status
                if (p.status === 'failed') {
                    try { this.updateDownloadProgressUI(p); } catch (_) {}
                }
                const map = this._ensurePendingMap();
                if (p.status === 'completed' || p.status === 'cancelled') {
                    map.delete(p.id);
                    try { if (window.__auralisDownloadRetryingIds) window.__auralisDownloadRetryingIds.delete(p.id); } catch (_) {}
                    return;
                }
                // fire-and-forget; internal handler is async
                this._handle403AutoRetry(p).catch((e) => console.warn('[Downloads] 403 auto-retry handler error', e?.message || e));
            });
            // Also handle dedicated download:failed events — surface full error including "unplayable" / "0s duration"
            this.on('download:failed', (p) => {
                if (!p) return;
                try { this.updateDownloadProgressUI({ ...p, status: p.status || 'failed' }); } catch (_) {}
                this._handle403AutoRetry({ ...p, status: 'failed' }).catch((e) => console.warn('[Downloads] 403 auto-retry handler (failed event) error', e?.message || e));
            });
            // Guard: progress may also carry failed status (backend emits progress before completed)
            this.on('download:progress', (p) => {
                if (p && p.status === 'failed') {
                    try { this.updateDownloadProgressUI(p); } catch (_) {}
                }
            });
        } catch (_) {}
    },

    _ensureDownloadSubmitListeners() {
        if (this._downloadSubmitListenersBound) return;
        this._downloadSubmitListenersBound = true;
        document.addEventListener('auralis:submit:download', (e) => {
            const form = (e.detail && e.detail.form) || document.getElementById('download-form');
            this.handleDownloadFormSubmit(e, form);
        });
        document.addEventListener('auralis:submit:search', (e) => {
            const form = (e.detail && e.detail.form) || document.getElementById('youtube-search-form');
            this.handleSearchFormSubmit(e, form);
        });
    },

    async _handle403AutoRetry(p) {
        if (!p || p.status !== 'failed') return;
        const map = this._ensurePendingMap();
        const errRaw = typeof this.extractErrorMessage === 'function'
            ? this.extractErrorMessage(p, '')
            : (p.error || p.error_message || '');
        if (!errRaw.includes('403') && !errRaw.includes('Forbidden') && !errRaw.includes('HTTP 403')) {
            map.delete(p.id);
            return;
        }
        const ctx = map.get(p.id);
        if (!ctx) return;
        if (ctx.retryCount >= 1 || ctx._retrying) return; // auto-retry once only & guard concurrent calls
        const resolved = ctx.resolved;
        const nextClient = resolved?.retryClients?.[0]
            || (() => {
                const oc = resolved?.orderedClients || [];
                const idx = oc.indexOf(resolved?.client || resolved?.winningClient);
                return idx >= 0 && idx + 1 < oc.length ? oc[idx + 1] : null;
            })();
        if (!nextClient) {
            console.warn(`[Downloads] 403 auto-retry no next client for ${p.id} winningClient=${resolved?.client}`);
            return;
        }
        // Mark retrying to suppress duplicate toast in core.js
        ctx.retryCount += 1;
        ctx._retrying = true;
        try { window.__auralisDownloadRetryingIds = window.__auralisDownloadRetryingIds || new Set(); window.__auralisDownloadRetryingIds.add(p.id); } catch (_) {}
        console.warn(`[Downloads] DIAGNOSTIC 403 auto-retry id=${p.id} ${resolved?.client} → ${nextClient} (retry ${ctx.retryCount}/1)`);
        this.showToast(`403 on ${resolved?.client || 'TV'} (rr1---sn-gwpa-cived), retrying with ${nextClient}…`, 'info', 5000);
        try {
            // Build opts for re-resolve: exclude the failing client, force next
            const baseOpts = ctx.opts || this.getDownloadOptions(document.getElementById('download-form')) || {};
            // Ensure we keep cookie/poToken from settings but allow minting
            const retryOpts = {
                ...baseOpts,
                forceClient: nextClient,
                excludeClient: resolved?.client || resolved?.winningClient,
                // Keep original orderedClients hint so youtube.js can rotate correctly
                // Also ensure poToken is considered (youtube.js will mint if needed)
            };
            // If retrying toward ANDROID/IOS, ensure poToken mint path is taken (youtube.js does it)
            if (!retryOpts.poToken && (nextClient === 'ANDROID' || nextClient === 'IOS')) {
                // Trigger poToken mint via youtube.js internal logic (it will import po_token.js)
                // No extra action needed; youtube.js will attempt generatePoTokenForVideo
            }
            const originalUrl = ctx.originalUrl || ctx.resolved?.originalUrl || p.url;
            if (!originalUrl || !window.AuralisYouTube) throw new Error('No original URL/client for retry');
            const reResolved = await window.AuralisYouTube.resolve(originalUrl, retryOpts);
            if (!reResolved || reResolved.kind !== 'track') throw new Error('Re-resolve did not return track');
            console.log(`[Downloads] 403 retry re-resolved ${originalUrl} via ${nextClient} -> ${reResolved.stream_url?.slice(0,80)}`);
            // Re-invoke download with new URL; this creates a new download id and new pending entry
            await this.downloadResolvedTrack(reResolved, ctx.format || 'm4a', retryOpts, originalUrl);
        } catch (e) {
            const msg = e?.message || String(e);
            console.error(`[Downloads] 403 auto-retry re-resolve failed for ${p.id}:`, msg);
            this.showToast(`Retry with ${nextClient} failed: ${msg}`, 'error', 6000);
            map.delete(p.id);
        } finally {
            ctx._retrying = false;
        }
    },

    async ensureSettings() {
        if (this.currentSettings) return this.currentSettings;
        try {
            this.currentSettings = await this.invoke('get_settings');
        } catch (_) {}
        return this.currentSettings;
    },

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
    },

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
                headers: resolved.headers || null,
            },
        };
    },

    async downloadResolvedTrack(resolved, format, opts = null, originalUrl = null) {
        if (!resolved || resolved.kind !== 'track') throw new Error('Not a downloadable track');
        // Ensure 403 auto-retry listener is bound once
        this._ensureDownloadRetryListener();
        try {
            const payload = this.buildDownloadPayload(resolved, format);
            console.log('[Downloads] Invoking download_audio', { title: resolved.title, url: resolved.stream_url?.slice(0,120), host: (()=>{try{return new URL(resolved.stream_url).host}catch(_){return 'unknown'}})(), headers: Object.keys(resolved.headers||{}), client: resolved.client, orderedClients: resolved.orderedClients, retryClients: resolved.retryClients });
            const result = await this.invoke('download_audio', payload);
            if (result) {
                this.updateDownloadProgressUI(result);
                // Store context for 403 auto-retry: id -> { resolved, opts, originalUrl, format }
                try {
                    const map = this._ensurePendingMap();
                    const ctxOpts = opts || this.getDownloadOptions(document.getElementById('download-form')) || resolved.resolveOpts || {};
                    const ctxUrl = originalUrl || resolved.originalUrl || resolved.stream_url;
                    map.set(result.id, {
                        resolved: { ...resolved },
                        opts: { ...ctxOpts },
                        originalUrl: ctxUrl,
                        format,
                        retryCount: 0,
                        _retrying: false,
                    });
                    // Also store reverse lookup by stream_url in case completed payload uses different id? not needed
                } catch (_) {}
            }
            return result;
        } catch (err) {
            const msg = typeof err === 'string' ? err : (err && err.message ? err.message : String(err));
            console.groupCollapsed(`%c[download_audio invoke failed] ${resolved.title || resolved.stream_url}`, 'color:#ff4d4f');
            console.error('DIAGNOSTIC download_invoke_failed', { resolved, format, error: msg, stack: err && err.stack });
            console.error(err);
            console.groupEnd();
            this.showToast(`Download start failed: ${msg}`, 'error', 7000);
            throw err;
        }
    },

    async handleDownloadFormSubmit(e, form) {
        if (e) {
            e.preventDefault();
            e.stopPropagation();
        }
        form = form || document.getElementById('download-form');
        if (!form) return;
        const urlInput = form.querySelector('input[name="url"]');
        if (!urlInput || !urlInput.value) return;

        let url = urlInput.value.trim();
        if (!/^https?:\/\//i.test(url)) {
            if (url.includes('youtube.com') || url.includes('youtu.be')) {
                url = 'https://' + url;
                urlInput.value = url;
            } else if (/^[a-zA-Z0-9_-]{11}$/.test(url)) {
                url = `https://www.youtube.com/watch?v=${url}`;
                urlInput.value = url;
            }
        }
        if (!url.startsWith('https://')) {
            this.showToast('Only secure HTTPS URLs are supported', 'error');
            return;
        }
        if (!window.AuralisYouTube) {
            this.showToast('YouTube resolver unavailable', 'error');
            return;
        }

        const opts = this.getDownloadOptions(form);

        // Check if URL is a playlist
        if (window.AuralisYouTube.isPlaylistUrl(url)) {
            this.showToast('Fetching playlist preview…', 'info');
            try {
                const pl = await window.AuralisYouTube.getPlaylist(url, opts);
                if (pl && pl.items && pl.items.length > 0) {
                    this.renderPlaylistPreview(pl, form);
                    this.showToast(`Found ${pl.items.length} track(s) in playlist`, 'success');
                    return;
                }
            } catch (plErr) {
                console.warn('getPlaylist failed, falling back to direct resolve:', plErr);
            }
        }

        this.showToast('Resolving source…', 'info');
        try {
            this._ensureDownloadRetryListener();
            const resolved = await window.AuralisYouTube.resolve(url, opts);
            if (resolved.kind === 'playlist') {
                await this.startPlaylistDownloads(resolved.items, 'm4a', opts, urlInput);
                return;
            }
            const result = await this.downloadResolvedTrack(resolved, 'm4a', opts, url);
            if (result) {
                this.showToast('Download started!', 'success');
                urlInput.value = '';
            }
        } catch (err) {
            const msg = err && err.message ? err.message : String(err);
            console.groupCollapsed(`%c[YouTube Resolve Failed] ${url}`, 'color:#ff7a45');
            console.error('DIAGNOSTIC resolve_failed', { url, opts, error: msg, stack: err && err.stack });
            console.error(err);
            console.groupEnd();
            try { window.__auralisDownloadDiagnostics = window.__auralisDownloadDiagnostics || []; window.__auralisDownloadDiagnostics.push({ at: new Date().toISOString(), kind: 'resolve_failed', url, error: msg }); } catch (_) {}
            this.showToast(`Resolve failed: ${msg}`, 'error', 6000);
        }
    },

    async handleSearchFormSubmit(e, searchForm) {
        if (e) {
            e.preventDefault();
            e.stopPropagation();
        }
        searchForm = searchForm || document.getElementById('youtube-search-form');
        if (!searchForm) return;
        const q = searchForm.querySelector('input[name="q"]');
        if (!q || !q.value.trim()) return;
        const form = document.getElementById('download-form');
        await this.performYouTubeSearch(q.value.trim(), this.getDownloadOptions(form || searchForm));
    },

    _bindSearchBridgeMethods() {
        if (typeof window !== 'undefined') {
            window.Auralis = window.Auralis || {};
            const target = window.Auralis.bridge || this;
            if (target) {
                target.streamYouTubeSearchResult = this.streamYouTubeSearchResult.bind(this);
                target.downloadSearchResult = this.downloadSearchResult.bind(this);
            }
        }
    },

    async loadDownloadView() {
        this._ensureDownloadSubmitListeners();
        this._bindSearchBridgeMethods();
        const form = document.getElementById('download-form');
        if (!form) return;

        await this.ensureSettings();
    },

    async performYouTubeSearch(query, opts) {
        if (!window.AuralisYouTube) {
            this.showToast('YouTube resolver unavailable', 'error');
            return;
        }
        const resultsEl = document.getElementById('youtube-search-results');
        const spinnerEl = document.getElementById('youtube-search-spinner');
        const searchBtn = document.getElementById('youtube-search-btn');
        if (!resultsEl) return;

        this.showToast('Searching YouTube…', 'info');
        if (spinnerEl) spinnerEl.style.display = 'block';
        resultsEl.style.display = 'block';
        resultsEl.innerHTML = `
            <div class="track-row neu-glass" style="display: flex; align-items: center; justify-content: center; padding: var(--space-6); border-radius: var(--radius-md);">
                <i data-lucide="loader-2" class="spin" style="width: 24px; height: 24px; color: var(--accent);"></i>
                <span style="margin-left: var(--space-2); color: var(--text-3); font-size: var(--text-sm);">Searching YouTube for “${this.escapeHtml(query)}”…</span>
            </div>
        `;
        if (window.lucide) window.lucide.createIcons();
        if (searchBtn) searchBtn.disabled = true;

        try {
            const results = await window.AuralisYouTube.search(query, opts);
            if (spinnerEl) spinnerEl.style.display = 'none';

            if (!results || results.length === 0) {
                resultsEl.innerHTML = `
                    <div class="empty-state" style="padding: var(--space-4); text-align: center;">
                        <p style="color: var(--text-3); font-size: var(--text-sm);">No results found for “${this.escapeHtml(query)}”.</p>
                    </div>`;
                return;
            }
            this._lastSearchResults = results;
            this._bindSearchBridgeMethods();

            resultsEl.innerHTML = results.map((r, i) => {
                const durText = r.duration_text || (r.duration ? this.formatTime(r.duration) : '');
                const durationPill = durText ? `
                    <div class="track-row-duration" style="margin-right: var(--space-2); flex-shrink: 0;">
                        <span class="neu-inset" style="padding: 2px 8px; font-size: var(--text-xs); color: var(--text-3); font-variant-numeric: tabular-nums;">${this.escapeHtml(durText)}</span>
                    </div>
                ` : '';

                const thumbContent = r.thumbnail
                    ? `<img src="${this.escapeHtml(r.thumbnail)}" alt="${this.escapeHtml(r.title)}" style="width: 100%; height: 100%; object-fit: cover;" onerror="this.onerror=null;this.parentElement.innerHTML='<i data-lucide=\\'music\\'></i>';if(window.lucide)window.lucide.createIcons();">`
                    : `<i data-lucide="music"></i>`;

                return `
                    <div class="track-row neu-glass" style="cursor: pointer; display: flex; align-items: center; gap: var(--space-3); padding: var(--space-2) var(--space-3); border-radius: var(--radius-md); margin-bottom: var(--space-2);">
                        <div class="track-row-artwork" style="width: 44px; height: 44px; border-radius: var(--radius-sm); overflow: hidden; flex-shrink: 0; background: var(--glass-weak); display: flex; align-items: center; justify-content: center;">
                            ${thumbContent}
                        </div>
                        <div class="track-row-info" style="flex: 1; min-width: 0;" onclick="window.Auralis.bridge.streamYouTubeSearchResult(${i})">
                            <div class="track-row-title" style="font-weight: var(--font-medium); font-size: var(--text-sm); color: var(--text-1); overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">${this.escapeHtml(r.title)}</div>
                            <div class="track-row-subtitle" style="font-size: var(--text-xs); color: var(--text-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">${this.escapeHtml(r.channel || 'YouTube')}</div>
                        </div>
                        ${durationPill}
                        <div class="track-row-actions" style="display: flex; align-items: center; gap: var(--space-2); opacity: 1; flex-shrink: 0;">
                            <button class="btn btn-ghost btn-icon play-yt-btn" title="Stream Now" onclick="event.stopPropagation(); window.Auralis.bridge.streamYouTubeSearchResult(${i})">
                                <i data-lucide="play"></i>
                            </button>
                            <button class="btn btn-primary btn-sm neu download-yt-btn" title="Download Audio" onclick="event.stopPropagation(); window.Auralis.bridge.downloadSearchResult(${i})">
                                <i data-lucide="download"></i> Download
                            </button>
                        </div>
                    </div>
                `;
            }).join('');
            if (window.lucide) window.lucide.createIcons();
        } catch (err) {
            if (spinnerEl) spinnerEl.style.display = 'none';
            const msg2 = err && err.message ? err.message : String(err);
            console.error('DIAGNOSTIC search_failed', { query, error: msg2, stack: err && err.stack });
            console.error(err);
            this.showToast(`Search failed: ${msg2}`, 'error', 6000);
            resultsEl.innerHTML = `
                <div class="empty-state" style="padding: var(--space-4); text-align: center;">
                    <p style="color: var(--danger, #ff4d4f); font-size: var(--text-sm);">Search failed: ${this.escapeHtml(msg2)}</p>
                </div>`;
        } finally {
            if (searchBtn) searchBtn.disabled = false;
        }
    },

    async streamYouTubeSearchResult(index) {
        const results = this._lastSearchResults || [];
        const item = results[index];
        if (!item) return;

        // If this same stream item is already loaded, toggle play/pause
        if (window._auralisStreamAudio && window.Auralis?.player?.currentTrack?.title === item.title) {
            if (window._auralisStreamAudio.paused) {
                try {
                    await window._auralisStreamAudio.play();
                } catch (e) {
                    console.warn('Stream play toggle failed:', e);
                }
            } else {
                window._auralisStreamAudio.pause();
            }
            return;
        }

        const rows = document.querySelectorAll('#youtube-search-results .track-row');
        const row = rows[index];
        const playBtn = row ? row.querySelector('.play-yt-btn') : null;
        if (playBtn) {
            playBtn.innerHTML = '<i data-lucide="loader-2" class="spin"></i>';
            if (window.lucide) window.lucide.createIcons();
        }

        this.showToast(`Connecting stream for “${this.escapeHtml(item.title)}”…`, 'info');

        try {
            // Pause any backend Rust playback
            if (window.Auralis && window.Auralis.player) {
                try { window.Auralis.player.pause(); } catch (_) {}
            }
            await this.invoke('pause').catch(() => {});

            // Stop any existing streaming audio element
            if (window._auralisStreamAudio) {
                try {
                    window._auralisStreamAudio.pause();
                    window._auralisStreamAudio.removeAttribute('src');
                    window._auralisStreamAudio.load();
                } catch (_) {}
                window._auralisStreamAudio = null;
            }

            const form = document.getElementById('download-form');
            const opts = this.getDownloadOptions(form);
            const resolved = await window.AuralisYouTube.resolve(item.url, opts);
            if (!resolved || resolved.kind !== 'track' || !resolved.stream_url) {
                throw new Error('Could not resolve playable stream URL');
            }

            const trackObj = {
                title: item.title || resolved.title || 'YouTube Audio',
                artist: item.channel || resolved.author || 'YouTube',
                duration_secs: item.duration || 0,
                album_art_path: item.thumbnail || resolved.thumbnail || null,
            };

            // Set current track and player bar reactive state
            if (window.Auralis && window.Auralis.player) {
                window.Auralis.player.currentTrack = trackObj;
                window.Auralis.player.duration = item.duration || 0;
                window.Auralis.player.progress = 0;
                window.Auralis.player.isPlaying = true;
                window.Auralis.player.updatePlayButton();
                window.Auralis.player.updateProgressUI();
                if (typeof window.Auralis.player.updateFullScreenMetadata === 'function') {
                    window.Auralis.player.updateFullScreenMetadata();
                }
                if (typeof window.Auralis.player.updateMediaSessionMetadata === 'function') {
                    window.Auralis.player.updateMediaSessionMetadata(trackObj);
                }
            }

            if (typeof this.updatePlayerBar === 'function') {
                this.updatePlayerBar(trackObj);
            } else if (window.Auralis?.bridge?.updatePlayerBar) {
                window.Auralis.bridge.updatePlayerBar(trackObj);
            }

            const audio = new Audio();
            audio.crossOrigin = 'anonymous';
            audio.src = resolved.stream_url;
            if (window.Auralis?.player && typeof window.Auralis.player.volume === 'number') {
                audio.volume = window.Auralis.player.volume;
            }
            window._auralisStreamAudio = audio;

            audio.addEventListener('loadedmetadata', () => {
                if (audio.duration && isFinite(audio.duration) && audio.duration > 0) {
                    trackObj.duration_secs = Math.round(audio.duration);
                    if (window.Auralis && window.Auralis.player) {
                        window.Auralis.player.duration = audio.duration;
                        window.Auralis.player.updateProgressUI();
                    }
                }
            });

            audio.addEventListener('timeupdate', () => {
                if (window.Auralis && window.Auralis.player && !window.Auralis.player.isSeeking) {
                    window.Auralis.player.progress = audio.currentTime;
                    if (audio.duration && isFinite(audio.duration) && audio.duration > 0) {
                        window.Auralis.player.duration = audio.duration;
                    }
                    window.Auralis.player.updateProgressUI();
                    window.Auralis.player.updatePositionState();
                }
            });

            audio.addEventListener('play', () => {
                if (window.Auralis && window.Auralis.player) {
                    window.Auralis.player.isPlaying = true;
                    window.Auralis.player.updatePlayButton();
                }
                if (playBtn) {
                    playBtn.innerHTML = '<i data-lucide="pause"></i>';
                    if (window.lucide) window.lucide.createIcons();
                }
            });

            audio.addEventListener('pause', () => {
                if (window.Auralis && window.Auralis.player) {
                    window.Auralis.player.isPlaying = false;
                    window.Auralis.player.updatePlayButton();
                }
                if (playBtn) {
                    playBtn.innerHTML = '<i data-lucide="play"></i>';
                    if (window.lucide) window.lucide.createIcons();
                }
            });

            audio.addEventListener('ended', () => {
                if (window.Auralis && window.Auralis.player) {
                    window.Auralis.player.isPlaying = false;
                    window.Auralis.player.progress = 0;
                    window.Auralis.player.updatePlayButton();
                    window.Auralis.player.updateProgressUI();
                }
                if (playBtn) {
                    playBtn.innerHTML = '<i data-lucide="play"></i>';
                    if (window.lucide) window.lucide.createIcons();
                }
            });

            audio.addEventListener('error', (e) => {
                console.error('Direct audio stream error:', e);
                this.showToast('Direct stream playback error', 'error', 6000);
                if (window.Auralis && window.Auralis.player) {
                    window.Auralis.player.isPlaying = false;
                    window.Auralis.player.updatePlayButton();
                }
                if (playBtn) {
                    playBtn.innerHTML = '<i data-lucide="play"></i>';
                    if (window.lucide) window.lucide.createIcons();
                }
            });

            await audio.play();
            this.showToast(`Streaming “${this.escapeHtml(trackObj.title)}”`, 'success');
        } catch (err) {
            const m = err && err.message ? err.message : String(err);
            console.error('DIAGNOSTIC streamYouTubeSearchResult failed', { item, error: m, stack: err && err.stack });
            this.showToast(`Stream failed: ${m}`, 'error', 6000);
            if (window.Auralis && window.Auralis.player) {
                window.Auralis.player.isPlaying = false;
                window.Auralis.player.updatePlayButton();
            }
            if (playBtn) {
                playBtn.innerHTML = '<i data-lucide="play"></i>';
                if (window.lucide) window.lucide.createIcons();
            }
        }
    },

    async downloadSearchResult(index) {
        const results = this._lastSearchResults || [];
        const item = results[index];
        if (!item) return;

        const rows = document.querySelectorAll('#youtube-search-results .track-row');
        const row = rows[index];
        const dlBtn = row ? row.querySelector('.download-yt-btn') : null;
        const prevContent = dlBtn ? dlBtn.innerHTML : null;
        if (dlBtn) {
            dlBtn.disabled = true;
            dlBtn.innerHTML = '<i data-lucide="loader-2" class="spin"></i> Starting…';
            if (window.lucide) window.lucide.createIcons();
        }

        const form = document.getElementById('download-form');
        const opts = this.getDownloadOptions(form);
        this.showToast('Resolving track…', 'info');
        try {
            this._ensureDownloadRetryListener();
            const resolved = await window.AuralisYouTube.resolve(item.url, opts);
            if (resolved.kind !== 'track') throw new Error('Not a track');
            const result = await this.downloadResolvedTrack(resolved, 'm4a', opts, item.url);
            if (result) this.showToast('Download started!', 'success');
        } catch (err) {
            const m = err && err.message ? err.message : String(err);
            console.error('DIAGNOSTIC downloadSearchResult failed', { item, error: m, stack: err && err.stack });
            console.error(err);
            this.showToast(`Resolve failed: ${m}`, 'error', 6000);
        } finally {
            if (dlBtn && prevContent) {
                dlBtn.disabled = false;
                dlBtn.innerHTML = prevContent;
                if (window.lucide) window.lucide.createIcons();
            }
        }
    },

    renderPlaylistPreview(playlistData, form) {
        const container = document.getElementById('youtube-playlist-preview');
        if (!container) return;

        this._currentPlaylistData = playlistData;
        const items = playlistData.items || [];

        container.style.display = 'block';
        container.innerHTML = `
            <div class="playlist-preview-header">
                <div>
                    <div class="playlist-preview-title">${this.escapeHtml(playlistData.title)}</div>
                    <div class="playlist-preview-stats">${playlistData.author ? `${this.escapeHtml(playlistData.author)} · ` : ''}${items.length} track(s) found</div>
                </div>
                <div style="display: flex; align-items: center; gap: var(--space-3);">
                    <label class="checkbox" style="font-size: var(--text-xs); cursor: pointer; display: inline-flex; align-items: center; gap: var(--space-1);">
                        <input type="checkbox" id="playlist-select-all" checked>
                        Select All
                    </label>
                </div>
            </div>
            <div class="playlist-preview-list" id="playlist-preview-items">
                ${items.map((item, idx) => `
                    <div class="playlist-item-row" data-index="${idx}">
                        <input type="checkbox" class="playlist-item-checkbox" data-index="${idx}" checked style="cursor: pointer;">
                        <div class="playlist-item-info">
                            <div class="playlist-item-title">${this.escapeHtml(item.title)}</div>
                            <div class="playlist-item-author">${item.channel ? this.escapeHtml(item.channel) : 'YouTube'}${item.duration ? ' · ' + this.formatTime(item.duration) : ''}</div>
                        </div>
                    </div>
                `).join('')}
            </div>
            <div style="display: flex; justify-content: space-between; align-items: center; margin-top: var(--space-3); padding-top: var(--space-3); border-top: 1px solid var(--glass-border);">
                <button type="button" class="btn btn-ghost btn-sm" id="btn-cancel-playlist-preview">
                    Cancel
                </button>
                <button type="button" class="btn btn-primary btn-sm neu" id="btn-download-selected">
                    <i data-lucide="download"></i>
                    <span id="btn-download-selected-label">Download All (${items.length})</span>
                </button>
            </div>
        `;

        if (window.lucide) window.lucide.createIcons();

        const selectAllCb = container.querySelector('#playlist-select-all');
        const itemCbs = container.querySelectorAll('.playlist-item-checkbox');
        const dlBtn = container.querySelector('#btn-download-selected');
        const dlBtnLabel = container.querySelector('#btn-download-selected-label');
        const cancelBtn = container.querySelector('#btn-cancel-playlist-preview');

        const updateSelectedCount = () => {
            const checkedCount = container.querySelectorAll('.playlist-item-checkbox:checked').length;
            if (dlBtnLabel) {
                dlBtnLabel.textContent = checkedCount === items.length
                    ? `Download All (${items.length})`
                    : `Download Selected (${checkedCount})`;
            }
            if (dlBtn) dlBtn.disabled = checkedCount === 0;
            if (selectAllCb) {
                selectAllCb.checked = checkedCount === items.length;
                selectAllCb.indeterminate = checkedCount > 0 && checkedCount < items.length;
            }
        };

        if (selectAllCb) {
            selectAllCb.addEventListener('change', () => {
                itemCbs.forEach(cb => { cb.checked = selectAllCb.checked; });
                updateSelectedCount();
            });
        }

        itemCbs.forEach(cb => {
            cb.addEventListener('change', updateSelectedCount);
        });

        if (cancelBtn) {
            cancelBtn.addEventListener('click', () => {
                container.style.display = 'none';
                container.innerHTML = '';
                this._currentPlaylistData = null;
            });
        }

        if (dlBtn) {
            dlBtn.addEventListener('click', async () => {
                const selected = [];
                container.querySelectorAll('.playlist-item-checkbox:checked').forEach(cb => {
                    const idx = parseInt(cb.dataset.index, 10);
                    if (items[idx]) selected.push(items[idx]);
                });
                if (selected.length === 0) {
                    this.showToast('No tracks selected for download', 'warning');
                    return;
                }
                container.style.display = 'none';
                container.innerHTML = '';
                const urlInput = form ? form.querySelector('input[name="url"]') : null;
                if (urlInput) urlInput.value = '';
                await this.downloadPlaylist(selected);
            });
        }
    },

    async downloadPlaylist(items) {
        if (!items || items.length === 0) {
            this.showToast('No tracks selected to download', 'warning');
            return;
        }
        const form = document.getElementById('download-form');
        const opts = this.getDownloadOptions(form);
        const format = 'm4a';
        this.showToast(`Starting batch download for ${items.length} track(s)...`, 'info');

        this._ensureDownloadRetryListener();
        let started = 0;
        for (const item of items) {
            try {
                const t = await window.AuralisYouTube.resolve(item.url, opts);
                if (t.kind !== 'track') continue;
                const result = await this.downloadResolvedTrack(t, format, opts, item.url);
                if (result) started++;
            } catch (err) {
                console.error(`Failed to start download for ${item.title}:`, err);
            }
        }

        if (started > 0) {
            this.showToast(`Started ${started} download(s) from playlist!`, 'success');
        } else {
            this.showToast('Could not start playlist downloads', 'error');
        }
    },

    async startPlaylistDownloads(items, format, opts, urlInput) {
        return this.downloadPlaylist(items);
    },

    updateDownloadProgressUI(progress) {
        const list = document.getElementById('downloads-list');
        if (!list) return;

        if (!list.dataset.copyBound) {
            list.dataset.copyBound = 'true';
            list.addEventListener('click', (e) => {
                const btn = e.target.closest && e.target.closest('[data-action="copy-download-error"]');
                if (!btn) return;
                const errText = btn.dataset.error || '';
                if (navigator.clipboard) {
                    navigator.clipboard.writeText(errText)
                        .then(() => this.showToast('Copied error to clipboard', 'success'))
                        .catch(() => this.showToast('Copy failed', 'error'));
                }
            });
        }

        let row = list.querySelector(`[data-download-id="${progress.id}"]`);
        if (!row) {
            row = document.createElement('div');
            row.className = 'track-row neu-glass';
            row.dataset.downloadId = progress.id;
            list.prepend(row);
        }

        const pct = Math.round((progress.progress || 0) * 100);
        const errRaw = typeof this.extractErrorMessage === 'function'
            ? this.extractErrorMessage(progress, '')
            : (progress.error || progress.error_message || '');
        const isFailed = progress.status === 'failed';
        const isCompleted = progress.status === 'completed';
        if (isFailed) {
            row.classList.add('download-failed');
            row.style.borderLeft = '3px solid #ff4d4f';
            // Log again for logcat visibility (progress event mirrors diagnostic)
            console.error(`[Downloads UI] DIAGNOSTIC failed row id=${progress.id} title=${progress.title} error=${errRaw} url=${progress.url}`);
        } else {
            row.classList.remove('download-failed');
            row.style.borderLeft = '';
        }
        const host = (() => { try { return new URL(progress.url || '').host || ''; } catch (_) { return ''; } })();
        const subtitle = isFailed
            ? `<span style="color:#ff4d4f;font-weight:600">failed • ${host ? host + ' • ' : ''}${pct}%</span>`
            : `${this.escapeHtml(progress.status)}${host ? ' • ' + this.escapeHtml(host) : ''} • ${pct}%`;
        const errBlock = isFailed && errRaw
            ? `<div style="margin-top:6px;padding:8px 10px;background:rgba(255,77,79,0.08);border:1px solid rgba(255,77,79,0.25);border-radius:8px;font-family:monospace;font-size:11px;line-height:1.4;white-space:pre-wrap;word-break:break-all;user-select:text;max-height:120px;overflow:auto;color:var(--text-2)">${this.escapeHtml(errRaw)}</div>
               <div style="margin-top:6px;display:flex;gap:8px;align-items:center;flex-wrap:wrap">
                 <button type="button" class="btn btn-secondary btn-sm" data-action="copy-download-error" data-error="${this.escapeHtml(errRaw)}" title="Copy full error (for bug report)">Copy error</button>
                  <span style="font-size:11px;color:var(--text-3)">${errRaw.includes('403') ? '403 [rr1---sn-gwpa-cived] Jio now gates TV too — auto-retrying with ANDROID+pot / WEB_SAFARI; if still fails set youtube_po_token via BgUtils mint or Settings cookie.' : errRaw.includes('timeout') || errRaw.includes('stalled') ? 'Network timeout — retry on stable connection.' : errRaw.includes('404') ? 'URL expired — resolve again.' : 'Tap Copy and include in bug report; also check adb logcat chromium.'}</span>
               </div>`
            : '';
        const pctBar = isFailed
            ? `<div class="progress-track neu-inset" style="width:120px;height:6px;opacity:0.5"><div class="progress-fill" style="width:${pct}%;background:#ff4d4f;height:100%"></div></div>`
            : `<div class="progress-track neu-inset" style="width: 120px; height: 6px;"><div class="progress-fill" style="width: ${pct}%; background: var(--accent); height: 100%;"></div></div>`;
        row.innerHTML = `
            <div class="track-row-info" style="min-width:0;flex:1">
                <div class="track-row-title" style="white-space:nowrap;overflow:hidden;text-overflow:ellipsis">${this.escapeHtml(progress.title || progress.url || 'Downloading...')}</div>
                <div class="track-row-subtitle">${subtitle}</div>
                ${errBlock}
            </div>
            ${pctBar}
        `;
    },

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
    },

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
    },

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
    },

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
};

try {
    if (typeof window !== 'undefined') {
        window.Auralis = window.Auralis || {};
        if (window.Auralis.bridge) {
            window.Auralis.bridge.streamYouTubeSearchResult = downloadMethods.streamYouTubeSearchResult.bind(window.Auralis.bridge);
            window.Auralis.bridge.downloadSearchResult = downloadMethods.downloadSearchResult.bind(window.Auralis.bridge);
        }
    }
} catch (_) {}
