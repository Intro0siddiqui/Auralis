/**
 * Sync Module
 * P2P synchronization, device pairing, and Iroh QUIC peer connections.
 */

import { copyWithToast, copyText } from './clipboard.js';

let scannerStream = null;
let scannerFacingMode = 'environment';
let scannerRafId = null;
let isScanning = false;
let isDetecting = false;
let lastScanTimestamp = 0;
let barcodeDetector = null;
let torchActive = false;

async function getBarcodeDetector() {
    if (barcodeDetector !== null) return barcodeDetector;
    if (typeof window !== 'undefined' && 'BarcodeDetector' in window) {
        try {
            const formats = await window.BarcodeDetector.getSupportedFormats();
            if (formats && formats.includes('qr_code')) {
                barcodeDetector = new window.BarcodeDetector({ formats: ['qr_code'] });
                return barcodeDetector;
            }
        } catch (e) {
            console.warn('[QR Scanner] Native BarcodeDetector init failed:', e);
        }
    }
    barcodeDetector = false;
    return null;
}

export const syncMethods = {
    async loadSyncView() {
        try {
            const pairingInfo = await this.invoke('start_pairing');
            if (pairingInfo) {
                const deviceIdEl = document.getElementById('sync-device-id');
                if (deviceIdEl) {
                    if (pairingInfo.ticket) {
                        window._currentPairingTicket = pairingInfo.ticket;
                        if (deviceIdEl.tagName === 'INPUT') {
                            deviceIdEl.value = pairingInfo.ticket;
                        } else {
                            deviceIdEl.textContent = `Ticket: ${pairingInfo.ticket}`;
                        }
                    } else {
                        if (deviceIdEl.tagName === 'INPUT') {
                            deviceIdEl.value = `PIN: ${pairingInfo.pin}`;
                        } else {
                            deviceIdEl.textContent = `PIN: ${pairingInfo.pin}`;
                        }
                    }
                }
                const pinEl = document.getElementById('sync-pairing-pin');
                if (pinEl && pairingInfo.pin) {
                    pinEl.textContent = `PIN: ${pairingInfo.pin}`;
                }
                const qrContainer = document.getElementById('sync-qr-container');
                if (qrContainer && pairingInfo.qr_image) {
                    const src = pairingInfo.qr_image.startsWith('data:')
                        ? pairingInfo.qr_image
                        : `data:image/png;base64,${pairingInfo.qr_image}`;
                    qrContainer.innerHTML = `<img src="${src}" alt="Pairing QR" style="width: 100%; height: 100%; object-fit: contain; border-radius: var(--radius-sm);">`;
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
                                    <div style="font-size: var(--text-xs); color: var(--text-3); font-family: monospace;">${d.peer_id ? (d.peer_id.slice(0, 16) + '...') : (d.ip_address || 'P2P Peer')} · Status: ${d.status || 'paired'}</div>
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
                            <p style="color: var(--text-3); font-size: var(--text-xs);">Scan the QR code or share your ticket to sync music libraries seamlessly.</p>
                        </div>
                    `;
                    if (window.lucide) window.lucide.createIcons();
                }
            }
        } catch (e) {
            console.error('Failed to load paired devices:', e);
        }
    },

    async copyPairingTicket() {
        const ticket = window._currentPairingTicket;
        if (!ticket) {
            this.showToast('No active pairing ticket available', 'warning');
            return;
        }
        await copyWithToast({
            text: ticket,
            label: 'pairing ticket',
            showToast: (m, k) => this.showToast(m, k)
        });
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

    async connectDirectPeer(addressOverride) {
        let address = (typeof addressOverride === 'string') ? addressOverride.trim() : '';
        const input = document.getElementById('direct-peer-address');
        if (!address) {
            if (!input || !input.value.trim()) {
                this.showToast('Please enter an Endpoint Ticket or URL', 'warning');
                return;
            }
            address = input.value.trim();
        } else if (input) {
            input.value = address;
        }

        this.showToast('Connecting to peer via Iroh QUIC...', 'info');
        try {
            const res = await this.invoke('connect_peer_address', { address });
            this.showToast(res || 'Peer connection established!', 'success');
            if (input) input.value = '';
            this.loadSyncView();
        } catch (err) {
            this.showToast(`Peer connection failed: ${err}`, 'error');
        }
    },

    async openQrScanner() {
        if (!navigator.mediaDevices || !navigator.mediaDevices.getUserMedia) {
            this.showToast('Camera access is not supported on this platform/device', 'warning');
            return;
        }

        const modal = document.getElementById('qr-scanner-modal');
        if (!modal) return;

        modal.style.display = 'flex';
        const reticle = document.getElementById('scanner-reticle');
        if (reticle) reticle.classList.remove('success');

        const statusEl = document.getElementById('scanner-status');
        if (statusEl) {
            statusEl.textContent = 'Starting camera...';
            statusEl.classList.remove('success');
        }

        if (window.lucide) window.lucide.createIcons();

        if (scannerStream) {
            try {
                scannerStream.getTracks().forEach(t => t.stop());
            } catch (_) {}
            scannerStream = null;
        }

        try {
            const constraints = {
                video: {
                    facingMode: { ideal: scannerFacingMode || 'environment' },
                    width: { ideal: 1280 },
                    height: { ideal: 720 }
                },
                audio: false
            };

            scannerStream = await navigator.mediaDevices.getUserMedia(constraints);
            const video = document.getElementById('qr-scanner-video');
            if (!video) {
                this.closeQrScanner();
                return;
            }

            video.srcObject = scannerStream;
            video.setAttribute('playsinline', 'true');
            await video.play();

            const track = scannerStream.getVideoTracks()[0];
            const capabilities = (track && typeof track.getCapabilities === 'function') ? track.getCapabilities() : {};
            const torchBtn = document.getElementById('btn-toggle-torch');
            if (torchBtn) {
                torchBtn.style.display = capabilities.torch ? 'inline-flex' : 'none';
            }
            torchActive = false;

            isScanning = true;
            if (statusEl) statusEl.textContent = 'Align QR code inside the frame';
            this._scanQrFrame();
        } catch (err) {
            console.error('[QR Scanner] Camera access failed:', err);
            this.closeQrScanner();
            const name = err.name || '';
            if (name === 'NotAllowedError' || name === 'PermissionDeniedError') {
                this.showToast('Camera permission denied', 'warning');
            } else if (name === 'NotFoundError' || name === 'DevicesNotFoundError') {
                this.showToast('No camera found on device', 'warning');
            } else {
                this.showToast(`Unable to start camera: ${err.message || err}`, 'error');
            }
        }
    },

    closeQrScanner() {
        isScanning = false;
        isDetecting = false;
        lastScanTimestamp = 0;
        if (scannerRafId) {
            cancelAnimationFrame(scannerRafId);
            scannerRafId = null;
        }
        if (scannerStream) {
            try {
                scannerStream.getTracks().forEach(t => t.stop());
            } catch (_) {}
            scannerStream = null;
        }
        const video = document.getElementById('qr-scanner-video');
        if (video) {
            video.srcObject = null;
        }
        const modal = document.getElementById('qr-scanner-modal');
        if (modal) {
            modal.style.display = 'none';
        }
        const reticle = document.getElementById('scanner-reticle');
        if (reticle) reticle.classList.remove('success');
        torchActive = false;
    },

    async switchScannerCamera() {
        scannerFacingMode = (scannerFacingMode === 'user') ? 'environment' : 'user';
        const modal = document.getElementById('qr-scanner-modal');
        if (modal && modal.style.display !== 'none') {
            await this.openQrScanner();
        }
    },

    async toggleScannerTorch() {
        if (!scannerStream) return;
        const track = scannerStream.getVideoTracks()[0];
        if (!track || typeof track.applyConstraints !== 'function') return;
        try {
            torchActive = !torchActive;
            await track.applyConstraints({
                advanced: [{ torch: torchActive }]
            });
        } catch (err) {
            console.warn('[QR Scanner] Toggle torch failed:', err);
        }
    },

    async _scanQrFrame() {
        if (!isScanning) return;
        const video = document.getElementById('qr-scanner-video');
        if (!video || video.readyState < HTMLMediaElement.HAVE_CURRENT_DATA) {
            if (isScanning) {
                scannerRafId = requestAnimationFrame(() => this._scanQrFrame());
            }
            return;
        }

        // Throttle scans to every 140ms so the camera video preview stays buttery-smooth at 60/120 FPS
        const now = performance.now();
        if (now - lastScanTimestamp < 140 || isDetecting) {
            if (isScanning) {
                scannerRafId = requestAnimationFrame(() => this._scanQrFrame());
            }
            return;
        }
        lastScanTimestamp = now;
        isDetecting = true;

        let detectedText = null;

        try {
            // 1. Primary: Native hardware-accelerated BarcodeDetector
            const detector = await getBarcodeDetector();
            if (detector) {
                try {
                    const barcodes = await detector.detect(video);
                    if (barcodes && barcodes.length > 0 && barcodes[0].rawValue) {
                        detectedText = barcodes[0].rawValue;
                    }
                } catch (_) {}
            }

            // 2. Fallback: jsQR via downscaled canvas capture (360px max dimension for 10x faster execution)
            if (!detectedText) {
                const jsQR = window.jsQR || (typeof jsQR !== 'undefined' ? jsQR : null);
                if (typeof jsQR === 'function') {
                    const canvas = document.getElementById('qr-scanner-canvas');
                    if (canvas) {
                        const rawW = video.videoWidth || 640;
                        const rawH = video.videoHeight || 480;
                        const scale = Math.min(1, 360 / Math.max(rawW, rawH));
                        const w = Math.round(rawW * scale);
                        const h = Math.round(rawH * scale);
                        if (w > 0 && h > 0) {
                            canvas.width = w;
                            canvas.height = h;
                            const ctx = canvas.getContext('2d', { willReadFrequently: true });
                            if (ctx) {
                                ctx.drawImage(video, 0, 0, w, h);
                                const imgData = ctx.getImageData(0, 0, w, h);
                                const qr = jsQR(imgData.data, imgData.width, imgData.height, {
                                    inversionAttempts: 'dontInvert'
                                });
                                if (qr && qr.data) {
                                    detectedText = qr.data;
                                }
                            }
                        }
                    }
                }
            }
        } finally {
            isDetecting = false;
        }

        if (detectedText && isScanning) {
            await this._handleQrDetected(detectedText);
            return;
        }

        if (isScanning) {
            scannerRafId = requestAnimationFrame(() => this._scanQrFrame());
        }
    },

    async _handleQrDetected(rawText) {
        if (!isScanning) return;
        isScanning = false;
        if (scannerRafId) {
            cancelAnimationFrame(scannerRafId);
            scannerRafId = null;
        }

        // Haptic feedback
        if (typeof navigator !== 'undefined' && navigator.vibrate) {
            try { navigator.vibrate(80); } catch (_) {}
        }

        // Visual feedback
        const reticle = document.getElementById('scanner-reticle');
        if (reticle) reticle.classList.add('success');
        const statusEl = document.getElementById('scanner-status');
        if (statusEl) {
            statusEl.textContent = 'QR Code Detected!';
            statusEl.classList.add('success');
        }

        const trimmed = (rawText || '').trim();

        // Brief delay so the user sees the green reticle confirmation
        await new Promise(r => setTimeout(r, 400));

        this.closeQrScanner();

        if (trimmed) {
            this.showToast('Connecting to paired peer...', 'info');
            await this.connectDirectPeer(trimmed);
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

if (typeof document !== 'undefined') {
    const closeScannerIfLeaving = () => {
        if (isScanning && window.Auralis && window.Auralis.bridge && typeof window.Auralis.bridge.closeQrScanner === 'function') {
            window.Auralis.bridge.closeQrScanner();
        }
    };
    window.addEventListener('view:leave', closeScannerIfLeaving);
    window.addEventListener('view:enter', closeScannerIfLeaving);
    document.addEventListener('view:leave', closeScannerIfLeaving);
    document.addEventListener('view:enter', closeScannerIfLeaving);

    // Delegated click handler — immune to CSP inline event handler restrictions
    document.addEventListener('click', (e) => {
        const actionEl = e.target.closest('[data-action]');
        if (!actionEl) return;
        const action = actionEl.getAttribute('data-action');
        if (!action) return;

        const bridge = window.Auralis?.bridge;
        if (!bridge) return;

        if (action === 'open-qr-scanner') {
            e.preventDefault();
            e.stopPropagation();
            bridge.openQrScanner();
        } else if (action === 'close-qr-scanner') {
            e.preventDefault();
            e.stopPropagation();
            bridge.closeQrScanner();
        } else if (action === 'switch-scanner-camera') {
            e.preventDefault();
            e.stopPropagation();
            bridge.switchScannerCamera();
        } else if (action === 'toggle-scanner-torch') {
            e.preventDefault();
            e.stopPropagation();
            bridge.toggleScannerTorch();
        } else if (action === 'connect-direct-peer') {
            e.preventDefault();
            e.stopPropagation();
            bridge.connectDirectPeer();
        } else if (action === 'copy-pairing-ticket') {
            e.preventDefault();
            e.stopPropagation();
            bridge.copyPairingTicket();
        } else if (action === 'sync-now') {
            e.preventDefault();
            e.stopPropagation();
            bridge.syncNow();
        }
    });
}
