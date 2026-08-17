/**
 * Downloads Module
 * Handles download queue, YouTube resolution glue, and sync triggers.
 */

export const downloadMethods = {
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
            },
        };
    },

    async downloadResolvedTrack(resolved, format) {
        if (!resolved || resolved.kind !== 'track') throw new Error('Not a downloadable track');
        const result = await this.invoke('download_audio', this.buildDownloadPayload(resolved, format));
        if (result) this.updateDownloadProgressUI(result);
        return result;
    },

    async loadDownloadView() {
        const form = document.getElementById('download-form');
        const searchForm = document.getElementById('youtube-search-form');
        if (!form || form.dataset.bound) return;
        form.dataset.bound = 'true';

        await this.ensureSettings();

        form.addEventListener('submit', async (e) => {
            e.preventDefault();
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
                const resolved = await window.AuralisYouTube.resolve(url, opts);
                if (resolved.kind === 'playlist') {
                    await this.startPlaylistDownloads(resolved.items, 'm4a', opts, urlInput);
                    return;
                }
                const result = await this.downloadResolvedTrack(resolved, 'm4a');
                if (result) {
                    this.showToast('Download started!', 'success');
                    urlInput.value = '';
                }
            } catch (err) {
                console.error(err);
                this.showToast(`Resolve failed: ${err && err.message ? err.message : err}`, 'error');
            }
        });

        if (searchForm && !searchForm.dataset.bound) {
            searchForm.dataset.bound = 'true';
            searchForm.addEventListener('submit', async (ev) => {
                ev.preventDefault();
                const q = searchForm.querySelector('input[name="q"]');
                if (!q || !q.value.trim()) return;
                await this.performYouTubeSearch(q.value.trim(), this.getDownloadOptions(form || searchForm));
            });
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
            console.error(err);
            this.showToast(`Search failed: ${err && err.message ? err.message : err}`, 'error');
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
            const resolved = await window.AuralisYouTube.resolve(item.url, opts);
            if (resolved.kind !== 'track') throw new Error('Not a track');
            const result = await this.downloadResolvedTrack(resolved, 'm4a');
            if (result) this.showToast('Download started!', 'success');
        } catch (err) {
            console.error(err);
            this.showToast(`Resolve failed: ${err && err.message ? err.message : err}`, 'error');
        }
    },

    async startPlaylistDownloads(items, format, opts, urlInput) {
        if (!items || items.length === 0) {
            this.showToast('Playlist contained no tracks', 'error');
            return;
        }
        const capped = items.slice(0, 20);
        this.showToast(`Resolving playlist (${capped.length} tracks)…`, 'info');

        let started = 0;
        for (const item of capped) {
            try {
                const t = await window.AuralisYouTube.resolve(item.url, opts);
                if (t.kind !== 'track') continue;
                const result = await this.downloadResolvedTrack(t, format);
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
        row.innerHTML = `
            <div class="track-row-info">
                <div class="track-row-title">${this.escapeHtml(progress.title || progress.url || 'Downloading...')}</div>
                <div class="track-row-subtitle">${progress.status} • ${pct}%</div>
            </div>
            <div class="progress-track neu-inset" style="width: 120px; height: 6px;">
                <div class="progress-fill" style="width: ${pct}%; background: var(--accent); height: 100%;"></div>
            </div>
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
