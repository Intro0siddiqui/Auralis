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
        this.bindFullScreenPlayerListeners();
        this.initBridgeListeners();
    }

    initBridgeListeners() {
        if (!window.Auralis || !window.Auralis.bridge || this._bridgeBound) return;
        this._bridgeBound = true;

        window.Auralis.bridge.on('playback:state', (state) => {
            this.isPlaying = state.is_playing;
            this.updatePlayButton();
        });

        window.Auralis.bridge.on('playback:track', (track) => {
            this.currentTrack = track;
            this.duration = track.duration_secs || 0;
            this.progress = 0;
            this.isLiked = Boolean(track.is_favorite);
            this.updateLikeUI();
            this.updateProgressUI();
            this.updatePlayButton();
            this.updateFullScreenMetadata();
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

    bindFullScreenPlayerListeners() {
        const observer = new MutationObserver(() => {
            this.wireFullScreenElements();
        });

        const overlayRoot = document.getElementById('overlay-root');
        if (overlayRoot) {
            observer.observe(overlayRoot, { childList: true, subtree: true });
        }
        document.body.addEventListener('htmx:afterSwap', () => {
            this.wireFullScreenElements();
        });
        this.wireFullScreenElements();
    }

    wireFullScreenElements() {
        const fullPlayer = document.getElementById('player-full');
        if (!fullPlayer || fullPlayer.dataset.wired) return;
        fullPlayer.dataset.wired = 'true';

        this.updateFullScreenMetadata();
        this.updatePlayButton();
        this.updateProgressUI();
        this.updateVolumeUI();
        this.updateLikeUI();
        this.updateShuffleRepeatUI();

        // 1. Play / Pause
        const fullPlay = document.getElementById('player-full-play');
        if (fullPlay) {
            fullPlay.addEventListener('click', (e) => {
                e.stopPropagation();
                this.togglePlay();
            });
        }

        // 2. Previous / Next
        const fullPrev = document.getElementById('player-full-prev');
        if (fullPrev) {
            fullPrev.addEventListener('click', (e) => {
                e.stopPropagation();
                this.previous();
            });
        }
        const fullNext = document.getElementById('player-full-next');
        if (fullNext) {
            fullNext.addEventListener('click', (e) => {
                e.stopPropagation();
                this.next();
            });
        }

        // 3. Shuffle / Repeat
        const fullShuffle = document.getElementById('player-full-shuffle');
        if (fullShuffle) {
            fullShuffle.addEventListener('click', (e) => {
                e.stopPropagation();
                this.toggleShuffle();
            });
        }
        const fullRepeat = document.getElementById('player-full-repeat');
        if (fullRepeat) {
            fullRepeat.addEventListener('click', (e) => {
                e.stopPropagation();
                this.cycleRepeat();
            });
        }

        // 4. Like / Favorite
        const fullLike = document.getElementById('player-full-like');
        if (fullLike) {
            fullLike.addEventListener('click', (e) => {
                e.stopPropagation();
                this.toggleLike();
            });
        }

        // 5. Progress Seek
        const fullProgress = document.getElementById('player-full-progress');
        if (fullProgress) {
            let isSeeking = false;
            const getPercent = (e) => {
                const rect = fullProgress.getBoundingClientRect();
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

            fullProgress.addEventListener('mousedown', startSeek);
            document.addEventListener('mousemove', moveSeek);
            document.addEventListener('mouseup', endSeek);

            fullProgress.addEventListener('touchstart', startSeek, { passive: true });
            fullProgress.addEventListener('touchmove', moveSeek, { passive: true });
            fullProgress.addEventListener('touchend', endSeek);
        }

        // 6. Volume Slider
        const fullVolume = document.getElementById('player-full-volume');
        if (fullVolume) {
            let isDragging = false;
            const getPercent = (e) => {
                const rect = fullVolume.getBoundingClientRect();
                const clientX = e.touches ? e.touches[0].clientX : e.clientX;
                return Math.max(0, Math.min(1, (clientX - rect.left) / rect.width));
            };

            const setVol = (e) => {
                this.setVolumeLevel(getPercent(e));
            };

            fullVolume.addEventListener('mousedown', (e) => {
                isDragging = true;
                setVol(e);
            });
            document.addEventListener('mousemove', (e) => { if (isDragging) setVol(e); });
            document.addEventListener('mouseup', () => { isDragging = false; });

            fullVolume.addEventListener('touchstart', (e) => {
                isDragging = true;
                setVol(e);
            }, { passive: true });
            fullVolume.addEventListener('touchmove', (e) => { if (isDragging) setVol(e); }, { passive: true });
            fullVolume.addEventListener('touchend', () => { isDragging = false; });
        }

        if (window.lucide) window.lucide.createIcons();
    }

    updateFullScreenMetadata() {
        const fullTitle = document.getElementById('player-full-title');
        const fullArtist = document.getElementById('player-full-artist');
        const fullArt = document.getElementById('player-full-art');

        if (fullTitle) {
            fullTitle.textContent = (this.currentTrack && this.currentTrack.title) || 'No Track Selected';
        }
        if (fullArtist) {
            fullArtist.textContent = (this.currentTrack && this.currentTrack.artist) || 'Select a song to play';
        }
        if (fullArt && this.currentTrack) {
            if (this.currentTrack.album_art_path) {
                const src = (window.Auralis && window.Auralis.assetUrl)
                    ? window.Auralis.assetUrl(this.currentTrack.album_art_path)
                    : this.currentTrack.album_art_path;
                const jsonPath = JSON.stringify(this.currentTrack.album_art_path).replace(/</g, '\\u003c');
                fullArt.innerHTML = `<img src="${src}" alt="${this.escapeHtml(this.currentTrack.title)}" style="width: 100%; height: 100%; object-fit: cover;" onerror="if(!this.dataset.fb){this.dataset.fb='1';window.Auralis.bridge.embedArt(this, ${jsonPath})}">`;
            } else {
                fullArt.innerHTML = `<div class="artwork-placeholder"><i data-lucide="disc-3"></i></div>`;
                if (window.lucide) window.lucide.createIcons();
            }
        }
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
                    this.setVolumeLevel(Math.min(1, this.volume + 0.05));
                    break;
                case 'ArrowDown':
                    e.preventDefault();
                    this.setVolumeLevel(Math.max(0, this.volume - 0.05));
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
        this.updateShuffleRepeatUI();
        if (window.Auralis && window.Auralis.bridge) {
            window.Auralis.bridge.invoke('set_shuffle', { enabled: this.shuffle });
        }
    }

    cycleRepeat() {
        const modes = ['off', 'all', 'one'];
        const idx = modes.indexOf(this.repeatMode);
        this.repeatMode = modes[(idx + 1) % modes.length];
        this.updateShuffleRepeatUI();
        if (window.Auralis && window.Auralis.bridge) {
            window.Auralis.bridge.invoke('set_repeat_mode', { mode: this.repeatMode });
        }
    }

    updateShuffleRepeatUI() {
        if (this.shuffleBtn) {
            this.shuffleBtn.classList.toggle('active', this.shuffle);
        }
        const fullShuffle = document.getElementById('player-full-shuffle');
        if (fullShuffle) {
            fullShuffle.classList.toggle('active', this.shuffle);
        }

        if (this.repeatBtn) {
            this.repeatBtn.classList.toggle('active', this.repeatMode !== 'off');
        }
        const fullRepeat = document.getElementById('player-full-repeat');
        if (fullRepeat) {
            fullRepeat.classList.toggle('active', this.repeatMode !== 'off');
        }
    }

    updatePlayButton() {
        // Player bar button
        if (this.playBtn) {
            const icon = this.playBtn.querySelector('i');
            if (icon) {
                icon.setAttribute('data-lucide', this.isPlaying ? 'pause' : 'play');
            }
        }
        // Full screen player button
        const fullPlay = document.getElementById('player-full-play');
        if (fullPlay) {
            const icon = fullPlay.querySelector('i');
            if (icon) {
                icon.setAttribute('data-lucide', this.isPlaying ? 'pause' : 'play');
            }
        }
        if (window.lucide) window.lucide.createIcons();
    }

    updateProgressUI() {
        const pct = this.duration > 0 ? (this.progress / this.duration) * 100 : 0;
        const currentStr = this.formatTime(this.progress);
        const totalStr = this.formatTime(this.duration);

        // Player bar
        if (this.progressFill) {
            this.progressFill.style.width = `${pct}%`;
        }
        if (this.progressHandle) {
            this.progressHandle.style.left = `${pct}%`;
        }
        if (this.timeCurrent) {
            this.timeCurrent.textContent = currentStr;
        }
        if (this.timeTotal) {
            this.timeTotal.textContent = totalStr;
        }

        // Full screen player
        const fullFill = document.getElementById('player-full-progress-fill');
        if (fullFill) {
            fullFill.style.width = `${pct}%`;
        }
        const fullHandle = document.getElementById('player-full-progress-handle');
        if (fullHandle) {
            fullHandle.style.left = `${pct}%`;
        }
        const fullCurrent = document.getElementById('full-time-current');
        if (fullCurrent) {
            fullCurrent.textContent = currentStr;
        }
        const fullTotal = document.getElementById('full-time-total');
        if (fullTotal) {
            fullTotal.textContent = totalStr;
        }
    }

    async toggleLike() {
        if (!this.currentTrack) {
            this.isLiked = !this.isLiked;
            this.updateLikeUI();
            return;
        }

        const newFavoriteState = !this.isLiked;
        this.isLiked = newFavoriteState;
        this.currentTrack.is_favorite = newFavoriteState;
        this.updateLikeUI();

        if (window.Auralis && window.Auralis.bridge) {
            try {
                await window.Auralis.bridge.invoke('set_track_favorite', {
                    id: this.currentTrack.id,
                    favorite: newFavoriteState
                });
                window.Auralis.bridge.showToast(
                    newFavoriteState ? 'Added to Liked Songs' : 'Removed from Liked Songs',
                    'info'
                );
                // Also update any matching track row heart icon in the UI
                const rowHearts = document.querySelectorAll(`[data-track-id="${this.currentTrack.id}"] .track-row-actions .btn:nth-child(2)`);
                rowHearts.forEach(btn => {
                    btn.classList.toggle('liked', newFavoriteState);
                    if (newFavoriteState) btn.style.color = 'var(--like)';
                    else btn.style.color = '';
                });
            } catch (err) {
                console.error('Failed to set favorite:', err);
                // Rollback on error
                this.isLiked = !newFavoriteState;
                this.currentTrack.is_favorite = !newFavoriteState;
                this.updateLikeUI();
                window.Auralis.bridge.showToast(`Failed to update favorite: ${err}`, 'error');
            }
        }
    }

    updateLikeUI() {
        if (this.likeBtn) {
            this.likeBtn.classList.toggle('liked', this.isLiked);
            if (this.isLiked) {
                this.likeBtn.style.color = 'var(--like)';
            } else {
                this.likeBtn.style.color = '';
            }
        }
        const fullLike = document.getElementById('player-full-like');
        if (fullLike) {
            fullLike.classList.toggle('liked', this.isLiked);
            if (this.isLiked) {
                fullLike.style.color = 'var(--like)';
            } else {
                fullLike.style.color = '';
            }
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
        const pct = `${this.volume * 100}%`;
        if (this.volumeFill) {
            this.volumeFill.style.width = pct;
        }
        const fullVolFill = document.getElementById('player-full-volume-fill');
        if (fullVolFill) {
            fullVolFill.style.width = pct;
        }

        let iconName = 'volume-2';
        if (this.volume === 0) iconName = 'volume-x';
        else if (this.volume < 0.5) iconName = 'volume-1';

        if (this.volumeBtn) {
            const icon = this.volumeBtn.querySelector('i');
            if (icon) {
                icon.setAttribute('data-lucide', iconName);
            }
        }
        const fullVolIcon = document.getElementById('player-full-volume-icon');
        if (fullVolIcon) {
            fullVolIcon.setAttribute('data-lucide', iconName);
        }
        if (window.lucide) window.lucide.createIcons();
    }

    startTimeTracking() {
        // No-op: the real playback position is driven by `playback:progress`
        // events emitted from the Rust playback watcher.
    }

    stopTimeTracking() {
        // No-op: retained for call-site compatibility.
    }

    formatTime(secs) {
        if (!secs || isNaN(secs)) return '0:00';
        const m = Math.floor(secs / 60);
        const s = Math.floor(secs % 60);
        return `${m}:${s.toString().padStart(2, '0')}`;
    }

    escapeHtml(str) {
        if (!str) return '';
        return str.replace(/[&<>"']/g, match => ({
            '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;'
        }[match]));
    }
}

document.addEventListener('DOMContentLoaded', () => {
    window.Auralis = window.Auralis || {};
    window.Auralis.player = new PlayerController();
});
