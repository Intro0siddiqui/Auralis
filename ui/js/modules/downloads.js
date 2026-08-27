/**
 * Downloads Module
 * Handles download queue, YouTube resolution glue, and sync triggers.
 */

export const downloadMethods = {
    // Pending download contexts for 403 auto-retry (id -> { resolved, opts, originalUrl, format, retryCount })
    _pendingDownloadContexts: null,
    _downloadRetryListenerBound: false,

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
                // fire-and-forget; internal handler is async
                this._handle403AutoRetry(p).catch((e) => console.warn('[Downloads] 403 auto-retry handler error', e?.message || e));
            });
        } catch (_) {}
    },

    async _handle403AutoRetry(p) {
        if (!p || p.status !== 'failed') return;
        const errRaw = p.error || p.error_message || '';
        if (!errRaw.includes('403') && !errRaw.includes('Forbidden') && !errRaw.includes('HTTP 403')) return;
        const map = this._ensurePendingMap();
        const ctx = map.get(p.id);
        if (!ctx) return;
        if (ctx.retryCount >= 1) return; // auto-retry once only
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

    async loadDownloadView() {
        const form = document.getElementById('download-form');
        const searchForm = document.getElementById('youtube-search-form');
        if (!form) return;

        await this.ensureSettings();

        // Rebind every swap: handles htmx history cache restoring data-bound without listeners (00:40.5 Download→Home)
        if (form._auralisSubmitHandler) form.removeEventListener('submit', form._auralisSubmitHandler);
        const _handler = async (e) => {
            e.preventDefault();
            e.stopPropagation();
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
        };
        form._auralisSubmitHandler = _handler;
        form.addEventListener('submit', _handler);
        form.dataset.bound = 'true';

        if (searchForm) {
            if (searchForm._auralisSearchHandler) searchForm.removeEventListener('submit', searchForm._auralisSearchHandler);
            const _searchHandler = async (ev) => {
                ev.preventDefault();
                ev.stopPropagation();
                const q = searchForm.querySelector('input[name="q"]');
                if (!q || !q.value.trim()) return;
                await this.performYouTubeSearch(q.value.trim(), this.getDownloadOptions(form || searchForm));
            };
            searchForm._auralisSearchHandler = _searchHandler;
            searchForm.addEventListener('submit', _searchHandler);
            searchForm.dataset.bound = 'true';
        }
    },

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
            const msg2 = err && err.message ? err.message : String(err);
            console.error('DIAGNOSTIC search_failed', { query, error: msg2, stack: err && err.stack });
            console.error(err);
            this.showToast(`Search failed: ${msg2}`, 'error', 6000);
        }
    },

    async downloadSearchResult(index) {
        const results = this._lastSearchResults || [];
        const item = results[index];
        if (!item) return;
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
        }
    },

    async startPlaylistDownloads(items, format, opts, urlInput) {
        if (!items || items.length === 0) {
            this.showToast('Playlist contained no tracks', 'error');
            return;
        }
        const capped = items.slice(0, 20);
        this.showToast(`Resolving playlist (${capped.length} tracks)…`, 'info');

        this._ensureDownloadRetryListener();
        let started = 0;
        for (const item of capped) {
            try {
                const t = await window.AuralisYouTube.resolve(item.url, opts);
                if (t.kind !== 'track') continue;
                const result = await this.downloadResolvedTrack(t, format, opts, item.url);
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
    },

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
        const errRaw = progress.error || progress.error_message || '';
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
                 <button type="button" class="btn btn-secondary btn-sm" onclick="navigator.clipboard&&navigator.clipboard.writeText(${JSON.stringify(errRaw).replace(/"/g,'&quot;')}).then(()=>window.Auralis&&window.Auralis.bridge&&window.Auralis.bridge.showToast&&window.Auralis.bridge.showToast('Copied error to clipboard','success')).catch(()=>window.Auralis&&window.Auralis.bridge&&window.Auralis.bridge.showToast&&window.Auralis.bridge.showToast('Copy failed','error'))" title="Copy full error (for bug report)">Copy error</button>
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
