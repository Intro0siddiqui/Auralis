class PlayerController {
    constructor() {
        this.isPlaying = false;
        this.currentTrack = null;
        this.progress = 0;
        this.duration = 0;
        this.volume = 0.7;
        this.repeatMode = 'off';
        this.shuffle = false;
        this.progressInterval = null;
        this.init();
    }

    init() {
        this.cacheElements();
        this.bindControls();
        this.bindProgress();
        this.bindVolume();
        this.bindKeyboard();

        if (window.Auralis && window.Auralis.bridge) {
            window.Auralis.bridge.on('playback:state', (state) => {
                this.isPlaying = state.is_playing;
                this.updatePlayButton();
            });

            window.Auralis.bridge.on('playback:track', (track) => {
                this.currentTrack = track;
                this.duration = track.duration_secs || 0;
                this.progress = 0;
                this.updateProgressUI();
                this.updatePlayButton();
            });

            window.Auralis.bridge.on('playback:progress', (data) => {
                this.progress = data.position;
                this.duration = data.duration;
                this.updateProgressUI();
            });
        }
    }

    cacheElements() {
        this.playBtn = document.getElementById('play-pause-btn');
        this.prevBtn = document.getElementById('prev-btn');
        this.nextBtn = document.getElementById('next-btn');
        this.shuffleBtn = document.getElementById('shuffle-btn');
        this.repeatBtn = document.getElementById('repeat-btn');
        this.progressTrack = document.getElementById('progress-track');
        this.progressFill = document.getElementById('progress-fill');
        this.progressHandle = document.getElementById('progress-handle');
        this.timeCurrent = document.getElementById('time-current');
        this.timeTotal = document.getElementById('time-total');
        this.volumeSlider = document.getElementById('volume-slider');
        this.volumeFill = document.getElementById('volume-fill');
    }

    bindControls() {
        if (this.playBtn) {
            this.playBtn.addEventListener('click', () => this.togglePlay());
        }
        if (this.prevBtn) {
            this.prevBtn.addEventListener('click', () => this.previous());
        }
        if (this.nextBtn) {
            this.nextBtn.addEventListener('click', () => this.next());
        }
        if (this.shuffleBtn) {
            this.shuffleBtn.addEventListener('click', () => this.toggleShuffle());
        }
        if (this.repeatBtn) {
            this.repeatBtn.addEventListener('click', () => this.cycleRepeat());
        }
    }

    bindProgress() {
        if (!this.progressTrack) return;

        let isSeeking = false;

        const getPercent = (e) => {
            const rect = this.progressTrack.getBoundingClientRect();
            const clientX = e.touches ? e.touches[0].clientX : e.clientX;
            return Math.max(0, Math.min(1, (clientX - rect.left) / rect.width));
        };

        const startSeek = (e) => {
            isSeeking = true;
            this.seekToPercent(getPercent(e));
        };

        const moveSeek = (e) => {
            if (isSeeking) this.seekToPercent(getPercent(e));
        };

        const endSeek = () => {
            isSeeking = false;
        };

        this.progressTrack.addEventListener('mousedown', startSeek);
        document.addEventListener('mousemove', moveSeek);
        document.addEventListener('mouseup', endSeek);

        this.progressTrack.addEventListener('touchstart', startSeek, { passive: true });
        this.progressTrack.addEventListener('touchmove', moveSeek, { passive: true });
        this.progressTrack.addEventListener('touchend', endSeek);
    }

    bindVolume() {
        if (!this.volumeSlider) return;

        const getPercent = (e) => {
            const rect = this.volumeSlider.getBoundingClientRect();
            const clientX = e.touches ? e.touches[0].clientX : e.clientX;
            return Math.max(0, Math.min(1, (clientX - rect.left) / rect.width));
        };

        const setVolume = (e) => {
            this.volume = getPercent(e);
            this.updateVolumeUI();
        };

        let isDragging = false;

        this.volumeSlider.addEventListener('mousedown', () => { isDragging = true; });
        document.addEventListener('mousemove', (e) => { if (isDragging) setVolume(e); });
        document.addEventListener('mouseup', () => { isDragging = false; });

        this.volumeSlider.addEventListener('touchstart', () => { isDragging = true; }, { passive: true });
        this.volumeSlider.addEventListener('touchmove', (e) => { if (isDragging) setVolume(e); }, { passive: true });
        this.volumeSlider.addEventListener('touchend', () => { isDragging = false; });
    }

    bindKeyboard() {
        document.addEventListener('keydown', (e) => {
            if (e.target.matches('input, textarea, select, [contenteditable="true"]')) return;

            switch (e.code) {
                case 'Space':
                    e.preventDefault();
                    this.togglePlay();
                    break;
                case 'ArrowLeft':
                    if (e.ctrlKey) this.previous();
                    else this.seekRelative(-5);
                    break;
                case 'ArrowRight':
                    if (e.ctrlKey) this.next();
                    else this.seekRelative(5);
                    break;
                case 'ArrowUp':
                    e.preventDefault();
                    this.volume = Math.min(1, this.volume + 0.05);
                    this.updateVolumeUI();
                    break;
                case 'ArrowDown':
                    e.preventDefault();
                    this.volume = Math.max(0, this.volume - 0.05);
                    this.updateVolumeUI();
                    break;
                case 'KeyS':
                    this.toggleShuffle();
                    break;
                case 'KeyR':
                    this.cycleRepeat();
                    break;
            }
        });

        this.bindMediaSession();
    }

    bindMediaSession() {
        if (!('mediaSession' in navigator)) return;

        try {
            navigator.mediaSession.setActionHandler('play', () => this.play());
            navigator.mediaSession.setActionHandler('pause', () => this.pause());
            navigator.mediaSession.setActionHandler('previoustrack', () => this.previous());
            navigator.mediaSession.setActionHandler('nexttrack', () => this.next());
            navigator.mediaSession.setActionHandler('seekto', (details) => {
                if (details.seekTime !== undefined && this.duration) {
                    this.seek(details.seekTime);
                }
            });
        } catch (err) {
            console.warn('MediaSession handler error:', err);
        }
    }

    togglePlay() {
        if (this.isPlaying) {
            this.pause();
        } else {
            this.play();
        }
    }

    play() {
        this.isPlaying = true;
        this.updatePlayButton();
        this.startTimeTracking();
        if (window.__TAURI_INTERNALS__) {
            window.__TAURI_INTERNALS__.invoke('resume').catch(() => {});
        }
    }

    pause() {
        this.isPlaying = false;
        this.updatePlayButton();
        this.stopTimeTracking();
        if (window.__TAURI_INTERNALS__) {
            window.__TAURI_INTERNALS__.invoke('pause').catch(() => {});
        }
    }

    next() {
        if (window.__TAURI_INTERNALS__) {
            window.__TAURI_INTERNALS__.invoke('next_track').catch(() => {});
        }
    }

    previous() {
        if (window.__TAURI_INTERNALS__) {
            window.__TAURI_INTERNALS__.invoke('previous_track').catch(() => {});
        }
    }

    seekToPercent(pct) {
        this.progress = pct * this.duration;
        this.updateProgressUI();
        if (window.__TAURI_INTERNALS__) {
            window.__TAURI_INTERNALS__.invoke('seek', { position_secs: Math.floor(this.progress) }).catch(() => {});
        }
    }

    seekRelative(delta) {
        this.progress = Math.max(0, Math.min(this.duration, this.progress + delta));
        this.seekToPercent(this.progress / this.duration);
    }

    toggleShuffle() {
        this.shuffle = !this.shuffle;
        if (this.shuffleBtn) {
            this.shuffleBtn.classList.toggle('active', this.shuffle);
        }
    }

    cycleRepeat() {
        const modes = ['off', 'all', 'one'];
        const idx = modes.indexOf(this.repeatMode);
        this.repeatMode = modes[(idx + 1) % modes.length];
        if (this.repeatBtn) {
            this.repeatBtn.classList.toggle('active', this.repeatMode !== 'off');
        }
    }

    updatePlayButton() {
        if (!this.playBtn) return;
        const icon = this.playBtn.querySelector('i');
        if (icon) {
            icon.setAttribute('data-lucide', this.isPlaying ? 'pause' : 'play');
            if (window.lucide) lucide.createIcons({ icon });
        }
    }

    updateProgressUI() {
        if (this.progressFill) {
            this.progressFill.style.width = `${this.duration > 0 ? (this.progress / this.duration) * 100 : 0}%`;
        }
        if (this.progressHandle) {
            this.progressHandle.style.left = `${this.duration > 0 ? (this.progress / this.duration) * 100 : 0}%`;
        }
        if (this.timeCurrent) {
            this.timeCurrent.textContent = this.formatTime(this.progress);
        }
        if (this.timeTotal) {
            this.timeTotal.textContent = this.formatTime(this.duration);
        }
    }

    updateVolumeUI() {
        if (this.volumeFill) {
            this.volumeFill.style.width = `${this.volume * 100}%`;
        }
    }

    startTimeTracking() {
        this.stopTimeTracking();
        this.progressInterval = setInterval(() => {
            if (this.isPlaying && this.progress < this.duration) {
                this.progress += 1;
                this.updateProgressUI();
            }
        }, 1000);
    }

    stopTimeTracking() {
        if (this.progressInterval) {
            clearInterval(this.progressInterval);
            this.progressInterval = null;
        }
    }

    formatTime(secs) {
        if (!secs || isNaN(secs)) return '0:00';
        const m = Math.floor(secs / 60);
        const s = Math.floor(secs % 60);
        return `${m}:${s.toString().padStart(2, '0')}`;
    }
}

document.addEventListener('DOMContentLoaded', () => {
    window.Auralis = window.Auralis || {};
    window.Auralis.player = new PlayerController();
});
