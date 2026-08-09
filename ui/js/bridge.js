class Bridge {
    constructor() {
        this.listeners = {};
        this.tauriAvailable = false;
        this.init();
    }

    async init() {
        try {
            if (window.__TAURI_INTERNALS__) {
                const { listen } = window.__TAURI_INTERNALS__.tauri;
                this.tauriAvailable = true;

                await listen('playback:state_changed', (event) => {
                    this.emit('playback:state', event.payload);
                });

                await listen('playback:track_changed', (event) => {
                    this.emit('playback:track', event.payload);
                    this.updatePlayerBar(event.payload);
                });

                await listen('playback:progress', (event) => {
                    this.emit('playback:progress', event.payload);
                    this.updateProgress(event.payload);
                });

                await listen('playback:queue_updated', (event) => {
                    this.emit('playback:queue', event.payload);
                });

                await listen('download:progress', (event) => {
                    this.emit('download:progress', event.payload);
                });

                await listen('download:completed', (event) => {
                    this.emit('download:completed', event.payload);
                    this.showToast('Download complete', 'success');
                });

                await listen('library:scan_complete', (event) => {
                    this.emit('library:scan', event.payload);
                    this.showToast(`Library scan complete: ${event.payload.new_tracks} new tracks`, 'info');
                });

                await listen('sync:device_discovered', (event) => {
                    this.emit('sync:device', event.payload);
                });

                await listen('sync:pairing_request', (event) => {
                    this.emit('sync:pairing', event.payload);
                    this.showToast('Pairing request received', 'info');
                });
            }
        } catch (e) {
            console.warn('Tauri bridge not available:', e);
        }
    }

    on(event, callback) {
        if (!this.listeners[event]) this.listeners[event] = [];
        this.listeners[event].push(callback);
    }

    emit(event, data) {
        if (this.listeners[event]) {
            this.listeners[event].forEach(cb => cb(data));
        }
    }

    updatePlayerBar(track) {
        const title = document.getElementById('track-title');
        const artist = document.getElementById('track-artist');
        const artwork = document.getElementById('current-artwork');

        if (title) title.textContent = track.title || 'No track playing';
        if (artist) artist.textContent = track.artist || 'Select a song';
        if (artwork && track.album_art_path) {
            artwork.innerHTML = `<img src="${track.album_art_path}" alt="${track.title}">`;
        }
    }

    updateProgress(data) {
        const fill = document.getElementById('progress-fill');
        const handle = document.getElementById('progress-handle');
        const currentTime = document.getElementById('time-current');
        const totalTime = document.getElementById('time-total');

        const pct = data.duration > 0 ? (data.position / data.duration) * 100 : 0;

        if (fill) fill.style.width = `${pct}%`;
        if (handle) handle.style.left = `${pct}%`;
        if (currentTime) currentTime.textContent = this.formatTime(data.position);
        if (totalTime) totalTime.textContent = this.formatTime(data.duration);
    }

    formatTime(secs) {
        const m = Math.floor(secs / 60);
        const s = Math.floor(secs % 60);
        return `${m}:${s.toString().padStart(2, '0')}`;
    }

    showToast(message, type = 'info') {
        const container = document.getElementById('toast-container') || document.body;
        const toast = document.createElement('div');
        toast.className = `toast toast-${type}`;
        toast.textContent = message;
        container.appendChild(toast);
        setTimeout(() => {
            toast.classList.add('toast-exit');
            setTimeout(() => toast.remove(), 300);
        }, 3000);
    }
}

window.Auralis = window.Auralis || {};
window.Auralis.bridge = new Bridge();
document.addEventListener('DOMContentLoaded', () => {
    window.Auralis.bridge.init();
});
