/**
 * Sync Module
 * P2P synchronization, device pairing, and LAN peer connections.
 */

export const syncMethods = {
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
