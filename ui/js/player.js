class PlayerController {
    constructor() {
        this.isPlaying = false;
        this.currentTrack = null;
        this.progress = 0;
        this.duration = 0;
        this.volume = 0.7;
        this.previousVolume = 0.7;
        this.isLiked = false;
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
                this.isLiked = false;
                if (this.likeBtn) {
                    this.likeBtn.classList.remove('liked');
                }
                this.updateProgressUI();
                this.updatePlayButton();
                if (this.queuePanel && this.queuePanel.classList.contains('open')) {
                    this.renderQueuePanel();
                }
            });

            window.Auralis.bridge.on('playback:progress', (data) => {
                this.progress = data.position;
                this.duration = data.duration;
                this.updateProgressUI();
            });

            window.Auralis.bridge.on('playback:queue', () => {
                if (this.queuePanel && this.queuePanel.classList.contains('open')) {
                    this.renderQueuePanel();
                }
            });
        }
    }

    cacheElements() {
        this.playBtn = document.getElementById('play-pause-btn');
        this.prevBtn = document.getElementById('prev-btn');
        this.nextBtn = document.getElementById('next-btn');
        this.shuffleBtn = document.getElementById('shuffle-btn');
        this.repeatBtn = document.getElementById('repeat-btn');
        this.likeBtn = document.getElementById('track-like');
        this.volumeBtn = document.getElementById('volume-btn');
        this.queueBtn = document.getElementById('queue-btn');
        this.queuePanel = document.getElementById('queue-panel');
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
        if (this.likeBtn) {
            this.likeBtn.addEventListener('click', () => this.toggleLike());
        }
        if (this.volumeBtn) {
            this.volumeBtn.addEventListener('click', () => this.toggleMute());
        }
        if (this.queueBtn) {
            this.queueBtn.addEventListener('click', () => this.toggleQueue());
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
            this.setVolumeLevel(getPercent(e));
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

    seek(secs) {
        if (!this.duration || this.duration <= 0) return;
        const pct = Math.max(0, Math.min(1, secs / this.duration));
        this.seekToPercent(pct);
    }

    seekToPercent(pct) {
        this.progress = pct * (this.duration || 0);
        this.updateProgressUI();
        if (window.Auralis && window.Auralis.bridge) {
            window.Auralis.bridge.invoke('seek', { request: { position_secs: Math.floor(this.progress) } });
        }
    }

    seekRelative(delta) {
        if (!this.duration) return;
        this.progress = Math.max(0, Math.min(this.duration, this.progress + delta));
        this.seekToPercent(this.progress / this.duration);
    }

    toggleShuffle() {
        this.shuffle = !this.shuffle;
        if (this.shuffleBtn) {
            this.shuffleBtn.classList.toggle('active', this.shuffle);
        }
        if (window.Auralis && window.Auralis.bridge) {
            window.Auralis.bridge.invoke('set_shuffle', { enabled: this.shuffle });
        }
    }

    cycleRepeat() {
        const modes = ['off', 'all', 'one'];
        const idx = modes.indexOf(this.repeatMode);
        this.repeatMode = modes[(idx + 1) % modes.length];
        if (this.repeatBtn) {
            this.repeatBtn.classList.toggle('active', this.repeatMode !== 'off');
        }
        if (window.Auralis && window.Auralis.bridge) {
            window.Auralis.bridge.invoke('set_repeat_mode', { mode: this.repeatMode });
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

    toggleLike() {
        this.isLiked = !this.isLiked;
        if (this.likeBtn) {
            this.likeBtn.classList.toggle('liked', this.isLiked);
        }
        if (window.Auralis && window.Auralis.bridge) {
            window.Auralis.bridge.showToast(this.isLiked ? 'Added to Liked Songs' : 'Removed from Liked Songs', 'info');
        }
    }

    toggleMute() {
        if (this.volume > 0) {
            this.previousVolume = this.volume;
            this.setVolumeLevel(0);
        } else {
            this.setVolumeLevel(this.previousVolume > 0 ? this.previousVolume : 0.7);
        }
    }

    setVolumeLevel(vol) {
        this.volume = Math.max(0, Math.min(1, vol));
        this.updateVolumeUI();
        if (window.Auralis && window.Auralis.bridge) {
            window.Auralis.bridge.invoke('set_volume', { volume: this.volume }).catch(() => {});
        }
    }

    toggleQueue() {
        if (!this.queuePanel) this.queuePanel = document.getElementById('queue-panel');
        if (!this.queuePanel) return;

        const isOpen = this.queuePanel.classList.toggle('open');
        if (this.queueBtn) {
            this.queueBtn.classList.toggle('active', isOpen);
        }
        if (isOpen) {
            this.renderQueuePanel();
        }
    }

    async renderQueuePanel() {
        if (!this.queuePanel) this.queuePanel = document.getElementById('queue-panel');
        if (!this.queuePanel) return;

        let queueTracks = [];
        try {
            if (window.Auralis && window.Auralis.bridge) {
                const q = await window.Auralis.bridge.invoke('get_queue');
                if (q && q.tracks) queueTracks = q.tracks;
            }
        } catch (e) {
            console.warn('Failed to fetch queue:', e);
        }

        const escapeHtml = (str) => {
            if (!str) return '';
            return str.replace(/[&<>"']/g, match => ({
                '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;'
            }[match]));
        };

        this.queuePanel.innerHTML = `
            <div style="padding: var(--space-4); height: 100%; display: flex; flex-direction: column;">
                <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: var(--space-4);">
                    <h3 style="font-size: var(--text-lg); font-weight: var(--font-semibold); color: var(--text-1);">Queue</h3>
                    <button class="btn btn-ghost btn-icon btn-sm" id="close-queue-btn" style="padding: var(--space-1);">
                        <i data-lucide="x"></i>
                    </button>
                </div>
                <div class="queue-content" style="flex: 1; overflow-y: auto;">
                    ${this.currentTrack ? `
                        <div style="font-size: var(--text-xs); color: var(--text-3); text-transform: uppercase; margin-bottom: var(--space-2); font-weight: var(--font-semibold);">Now Playing</div>
                        <div class="track-row neu-glass" style="margin-bottom: var(--space-4); border-radius: var(--radius-md); padding: var(--space-2) var(--space-3);">
                            <div class="track-row-info">
                                <div class="track-row-title" style="color: var(--accent);">${escapeHtml(this.currentTrack.title)}</div>
                                <div class="track-row-subtitle">${escapeHtml(this.currentTrack.artist || 'Unknown Artist')}</div>
                            </div>
                        </div>
                    ` : ''}
                    <div style="font-size: var(--text-xs); color: var(--text-3); text-transform: uppercase; margin-bottom: var(--space-2); font-weight: var(--font-semibold);">Next Up (${queueTracks.length})</div>
                    ${queueTracks.length > 0 ? queueTracks.map((t, i) => `
                        <div class="track-row neu-glass" style="margin-bottom: var(--space-2); border-radius: var(--radius-md); padding: var(--space-2) var(--space-3); display: flex; justify-content: space-between; align-items: center;">
                            <div class="track-row-info" style="flex: 1; overflow: hidden;">
                                <div class="track-row-title">${escapeHtml(t.title)}</div>
                                <div class="track-row-subtitle">${escapeHtml(t.artist || 'Unknown Artist')}</div>
                            </div>
                            <button class="btn btn-ghost btn-icon" onclick="window.Auralis.player.removeFromQueue(${i})" title="Remove">
                                <i data-lucide="trash-2"></i>
                            </button>
                        </div>
                    `).join('') : `
                        <div class="empty-state glass neu" style="padding: var(--space-4); text-align: center; border-radius: var(--radius-md);">
                            <p style="color: var(--text-3); font-size: var(--text-xs);">No tracks in queue</p>
                        </div>
                    `}
                </div>
                ${queueTracks.length > 0 ? `
                    <div style="padding-top: var(--space-3); border-top: 1px solid var(--glass-border);">
                        <button class="btn btn-secondary btn-sm neu" style="width: 100%; justify-content: center;" onclick="window.Auralis.player.clearQueue()">
                            <i data-lucide="trash-2"></i>
                            Clear Queue
                        </button>
                    </div>
                ` : ''}
            </div>
        `;

        const closeBtn = this.queuePanel.querySelector('#close-queue-btn');
        if (closeBtn) {
            closeBtn.addEventListener('click', () => this.toggleQueue());
        }
        if (window.lucide) window.lucide.createIcons();
    }

    async clearQueue() {
        if (window.Auralis && window.Auralis.bridge) {
            try {
                await window.Auralis.bridge.invoke('clear_queue');
                this.renderQueuePanel();
            } catch (e) {
                console.error('Failed to clear queue:', e);
            }
        }
    }

    async removeFromQueue(index) {
        if (window.Auralis && window.Auralis.bridge) {
            try {
                await window.Auralis.bridge.invoke('remove_from_queue', { index });
                this.renderQueuePanel();
            } catch (e) {
                console.error('Failed to remove from queue:', e);
            }
        }
    }

    updateVolumeUI() {
        if (this.volumeFill) {
            this.volumeFill.style.width = `${this.volume * 100}%`;
        }
        if (this.volumeBtn) {
            const icon = this.volumeBtn.querySelector('i');
            if (icon) {
                let iconName = 'volume-2';
                if (this.volume === 0) iconName = 'volume-x';
                else if (this.volume < 0.5) iconName = 'volume-1';
                icon.setAttribute('data-lucide', iconName);
                if (window.lucide) window.lucide.createIcons();
            }
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
