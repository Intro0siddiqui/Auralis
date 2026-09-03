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
        this.isSeeking = false;
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
        this.hydrateState();
    }

    initBridgeListeners() {
        if (!window.Auralis || !window.Auralis.bridge || this._bridgeBound) return;
        this._bridgeBound = true;

        window.Auralis.bridge.on('playback:state', (state) => {
            this.isPlaying = state.is_playing;
            if (state.position_secs !== undefined && !state.is_playing) {
                this.progress = state.position_secs;
                this.updateProgressUI();
            }
            this.updatePlayButton();
        });

        window.Auralis.bridge.on('playback:track', (payload) => {
            if (window._auralisStreamAudio) {
                try {
                    window._auralisStreamAudio.pause();
                    window._auralisStreamAudio.removeAttribute('src');
                    window._auralisStreamAudio.load();
                } catch (_) {}
                window._auralisStreamAudio = null;
            }
            const track = (payload && payload.track) ? payload.track : payload;
            if (!track) return;
            this.currentTrack = track;
            this.duration = track.duration_secs || track.duration || 0;
            this.progress = 0;
            this.isLiked = Boolean(track.is_favorite);
            this.updateLikeUI();
            this.updateProgressUI();
            this.updatePlayButton();
            this.updateFullScreenMetadata();
            this.updateMediaSessionMetadata(track);
            this.updatePositionState();
            if (this.queuePanel && this.queuePanel.classList.contains('open')) {
                this.renderQueuePanel();
            }
        });

        window.Auralis.bridge.on('playback:progress', (data) => {
            if (this.isSeeking || !data) return;
            this.progress = data.position_secs !== undefined ? data.position_secs : (data.position || 0);
            this.duration = data.duration_secs !== undefined ? data.duration_secs : (data.duration || 0);
            this.updateProgressUI();
            this.updatePositionState();
        });

        window.Auralis.bridge.on('playback:queue', () => {
            const drawer = document.getElementById('player-full-queue-drawer');
            if ((this.queuePanel && this.queuePanel.classList.contains('open')) || (drawer && drawer.classList.contains('open'))) {
                this.renderQueuePanel();
            }
        });

        window.Auralis.bridge.on('playback:error', (msg) => {
            console.error('[playback:error]', msg);
            if (window.Auralis && window.Auralis.bridge && typeof window.Auralis.bridge.showToast === 'function') {
                window.Auralis.bridge.showToast(`Playback failed: ${msg}`, 'error', 7000);
            }
            // Keep bar in error state until next successful track
            try { window.__auralisLastPlaybackError = { at: new Date().toISOString(), msg }; } catch (_) {}
            // Also surface in player bar subtitle if no track
            const artistEl = document.getElementById('track-artist');
            if (artistEl && msg && !this.currentTrack) {
                artistEl.textContent = String(msg).slice(0, 120);
                artistEl.style.color = 'var(--danger, #ff4d4f)';
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
        this.mobileProgressFill = document.getElementById('mobile-progress-fill');
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
            this.isSeeking = true;
            const pct = getPercent(e);
            this.progress = pct * (this.duration || 0);
            this.updateProgressUI();
            this.updatePositionState();
        };

        const moveSeek = (e) => {
            if (isSeeking) {
                const pct = getPercent(e);
                this.progress = pct * (this.duration || 0);
                this.updateProgressUI();
                this.updatePositionState();
            }
        };

        const endSeek = () => {
            if (isSeeking) {
                isSeeking = false;
                this.isSeeking = false;
                this.commitSeek();
            }
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
        document.body.addEventListener('htmx:afterSwap', (e) => {
            const target = e && e.detail && e.detail.target;
            const isPlayerSwap = target && (target.id === 'overlay-root' || (target.querySelector && target.querySelector('#player-full')) || target.id === 'player-full');
            // Also re-sync bar when any view swaps
            if (window.Auralis && window.Auralis.player) {
                window.Auralis.player.hydrateState().catch(() => {});
            }
            if (isPlayerSwap || document.getElementById('player-full')) {
                const fp = document.getElementById('player-full');
                if (fp) fp.dataset.wired = '';
                this.wireFullScreenElements();
                this.hydrateState().catch(() => {});
            }
        });

        // Also wire on explicit overlay click (player-bar hx-get target)
        const playerBar = document.querySelector('.player-track[hx-get]');
        if (playerBar) {
            playerBar.addEventListener('click', () => {
                // Hydrate before HTMX fetches partial so metadata is ready when it swaps
                this.hydrateState().catch(() => {});
            });
        }
        this.wireFullScreenElements();
    }

    wireFullScreenElements() {
        const fullPlayer = document.getElementById('player-full');
        if (!fullPlayer) return;
        // Always refresh metadata even if already wired (fixes 01:27 modal desync)
        const wasWired = fullPlayer.dataset.wired === 'true';
        fullPlayer.dataset.wired = 'true';

        this.refreshFullScreenUI();
        if (wasWired) return;

        this.wireFullScreenControls();

        if (window.lucide) window.lucide.createIcons();
    }

    refreshFullScreenUI() {
        this.updateFullScreenMetadata();
        this.updatePlayButton();
        this.updateProgressUI();
        this.updateVolumeUI();
        this.updateLikeUI();
        this.updateShuffleRepeatUI();
    }

    wireFullScreenControls() {
        this.wireFullScreenButtons();
        this.wireFullScreenProgressSlider();
        this.wireFullScreenVolumeSlider();
    }

    wireFullScreenButtons() {
        this.wireFullScreenCloseButton();
        this.wireFullScreenQueueButton();
        this.wireFullScreenPlayButton();
        this.wireFullScreenNavButtons();
        this.wireFullScreenModeButtons();
        this.wireFullScreenLikeButton();
    }

    wireFullScreenQueueButton() {
        const fullQueueBtn = document.getElementById('player-full-queue-btn');
        if (fullQueueBtn) {
            fullQueueBtn.addEventListener('click', (e) => {
                e.stopPropagation();
                this.toggleFullScreenQueue();
            });
        }
    }

    toggleFullScreenQueue(forceOpen) {
        const drawer = document.getElementById('player-full-queue-drawer');
        const backdrop = document.getElementById('player-full-queue-backdrop');
        const btn = document.getElementById('player-full-queue-btn');
        if (!drawer) return;

        let isOpen;
        if (typeof forceOpen === 'boolean') {
            isOpen = forceOpen;
            drawer.classList.toggle('open', isOpen);
        } else {
            isOpen = drawer.classList.toggle('open');
        }

        if (backdrop) backdrop.classList.toggle('open', isOpen);
        if (btn) btn.classList.toggle('active', isOpen);
        if (isOpen) {
            this.renderQueuePanel();
        }
    }

    wireFullScreenCloseButton() {
        const fullClose = document.getElementById('player-full-close') || document.querySelector('.player-full-close');
        if (fullClose) {
            fullClose.addEventListener('click', (e) => {
                e.stopPropagation();
                const overlay = document.getElementById('overlay-root');
                if (overlay) overlay.innerHTML = '';
            });
        }
    }

    wireFullScreenPlayButton() {
        const fullPlay = document.getElementById('player-full-play');
        if (fullPlay) {
            fullPlay.addEventListener('click', (e) => {
                e.stopPropagation();
                this.togglePlay();
            });
        }
    }

    wireFullScreenNavButtons() {
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
    }

    wireFullScreenModeButtons() {
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
    }

    wireFullScreenLikeButton() {
        const fullLike = document.getElementById('player-full-like');
        if (fullLike) {
            fullLike.addEventListener('click', (e) => {
                e.stopPropagation();
                this.toggleLike();
            });
        }
    }

    wireFullScreenProgressSlider() {
        // 5. Progress Seek
        const fullProgress = document.getElementById('player-full-progress');
        if (!fullProgress) return;

        let isSeeking = false;
        const getPercent = (e) => {
            const rect = fullProgress.getBoundingClientRect();
            const clientX = e.touches ? e.touches[0].clientX : e.clientX;
            return Math.max(0, Math.min(1, (clientX - rect.left) / rect.width));
        };

        const startSeek = (e) => {
            isSeeking = true;
            this.isSeeking = true;
            const pct = getPercent(e);
            this.progress = pct * (this.duration || 0);
            this.updateProgressUI();
            this.updatePositionState();
        };
        const moveSeek = (e) => {
            if (isSeeking) {
                const pct = getPercent(e);
                this.progress = pct * (this.duration || 0);
                this.updateProgressUI();
                this.updatePositionState();
            }
        };
        const endSeek = () => {
            if (isSeeking) {
                isSeeking = false;
                this.isSeeking = false;
                this.commitSeek();
            }
        };

        fullProgress.addEventListener('mousedown', startSeek);
        document.addEventListener('mousemove', moveSeek);
        document.addEventListener('mouseup', endSeek);

        fullProgress.addEventListener('touchstart', startSeek, { passive: true });
        fullProgress.addEventListener('touchmove', moveSeek, { passive: true });
        fullProgress.addEventListener('touchend', endSeek);
    }

    wireFullScreenVolumeSlider() {
        // 6. Volume Slider
        const fullVolume = document.getElementById('player-full-volume');
        if (!fullVolume) return;

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

    updateFullScreenMetadata() {
        const fullTitle = document.getElementById('player-full-title');
        const fullArtist = document.getElementById('player-full-artist');
        const fullArt = document.getElementById('player-full-art');

        if (fullTitle) {
            fullTitle.textContent = (this.currentTrack && this.currentTrack.title) || 'No Track Selected';
        }
        if (fullArtist) {
            fullArtist.textContent = this.currentTrack
                ? (this.currentTrack.artist || 'Unknown Artist')
                : 'Select a song to play';
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
            if (e.target.matches('input, textarea, select, button, [contenteditable="true"], [role="switch"], .toggle, .theme-option')) return;

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
                    if (!e.ctrlKey && !e.metaKey && !e.altKey) return;
                    e.preventDefault();
                    this.setVolumeLevel(Math.min(1, this.volume + 0.05));
                    break;
                case 'ArrowDown':
                    if (!e.ctrlKey && !e.metaKey && !e.altKey) return;
                    e.preventDefault();
                    this.setVolumeLevel(Math.max(0, this.volume - 0.05));
                    break;
                case 'KeyS':
                    this.toggleShuffle();
                    break;
                case 'KeyR':
                    this.cycleRepeat();
                    break;
                case 'Escape':
                    const drawer = document.getElementById('player-full-queue-drawer');
                    if (drawer && drawer.classList.contains('open')) {
                        this.toggleFullScreenQueue(false);
                    } else {
                        const overlay = document.getElementById('overlay-root');
                        if (overlay && overlay.innerHTML.trim() !== '') {
                            overlay.innerHTML = '';
                        }
                    }
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
            try { navigator.mediaSession.setActionHandler('seekbackward', (details) => { const off = (details && details.seekOffset) || 10; this.seekRelative(-off); }); } catch (_) {}
            try { navigator.mediaSession.setActionHandler('seekforward', (details) => { const off = (details && details.seekOffset) || 10; this.seekRelative(off); }); } catch (_) {}
            try { navigator.mediaSession.setActionHandler('stop', () => { if (window.Auralis && window.Auralis.bridge) window.Auralis.bridge.invoke('stop').catch(()=>{}); this.isPlaying = false; this.updatePlayButton(); }); } catch (_) {}
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

    async play() {
        this.isPlaying = true;
        this.updatePlayButton();
        this.startTimeTracking();
        if (window._auralisStreamAudio) {
            try {
                await window._auralisStreamAudio.play();
            } catch (e) {
                console.warn('Stream resume failed:', e);
            }
            return;
        }
        if (!window.Auralis || !window.Auralis.bridge) return;
        // If we have a current track, resume is correct — it preserves position.
        if (this.currentTrack && this.currentTrack.id) {
            try {
                await window.Auralis.bridge.invoke('resume');
            } catch (err) {
                const msg = String(err || 'resume failed');
                console.warn('Resume failed:', msg);
                window.Auralis.bridge.showToast(`Resume failed: ${msg} — retrying track`, 'error', 6000);
                // fallback: replay the current track from start
                window.Auralis.bridge.playTrack(this.currentTrack.id);
            }
            return;
        }
        // No track loaded (fresh start / "No track playing") — resume is a no-op in Rust
        // (sink is None → Ok(())). Play the last queued track or the first library track.
        try {
            const q = await window.Auralis.bridge.invoke('get_queue');
            if (q && q.tracks && q.tracks.length > 0) {
                const idx = q.current_index ?? 0;
                const t = q.tracks[idx];
                if (t && t.id) return window.Auralis.bridge.playTrack(t.id);
            }
        } catch (_) {}
        try {
            const page = await window.Auralis.bridge.invoke('get_tracks', { filter: { limit: 1 } });
            const t = page && page.tracks && page.tracks[0];
            if (t && t.id) return window.Auralis.bridge.playTrack(t.id);
            // Library empty — nothing to play
            this.isPlaying = false;
            this.updatePlayButton();
            window.Auralis.bridge.showToast('No track to play — import audio or download a track first', 'info', 5000);
        } catch (err) {
            const msg = String(err || 'no track');
            this.isPlaying = false;
            this.updatePlayButton();
            console.warn('Play fallback failed:', msg);
            window.Auralis.bridge.showToast(`Play failed: ${msg}`, 'error', 6000);
        }
    }

    pause() {
        this.isPlaying = false;
        this.updatePlayButton();
        this.stopTimeTracking();
        if (window._auralisStreamAudio) {
            try {
                window._auralisStreamAudio.pause();
            } catch (e) {
                console.warn('Stream pause failed:', e);
            }
            return;
        }
        if (window.Auralis && window.Auralis.bridge) {
            window.Auralis.bridge.invoke('pause').catch((err) => {
                const msg = String(err || 'pause failed');
                console.warn('Pause failed:', msg);
                window.Auralis.bridge.showToast(`Pause failed: ${msg}`, 'error', 6000);
            });
        }
    }

    async next() {
        if (!window.Auralis || !window.Auralis.bridge) return;
        try {
            const res = await window.Auralis.bridge.invoke('next_track');
            if (res) return; // Rust advanced successfully
            // Queue empty — fallback to library order (fixes 01:44 Next dead when queue empty)
            const tracks = (window.Auralis.bridge.tracks && window.Auralis.bridge.tracks.length)
                ? window.Auralis.bridge.tracks
                : (await window.Auralis.bridge.invoke('get_tracks', { filter: { limit: 200 } }).then(p=>p.tracks||[]).catch(()=>[]));
            if (!tracks.length) {
                window.Auralis.bridge.showToast('No next track — queue empty and library empty', 'info', 4000);
                return;
            }
            const curId = this.currentTrack && this.currentTrack.id;
            let idx = tracks.findIndex(t => String(t.id) === String(curId));
            idx = idx >= 0 ? (idx + 1) % tracks.length : 0;
            const nxt = tracks[idx];
            if (nxt && nxt.id) await window.Auralis.bridge.playTrack(nxt.id);
        } catch (err) {
            const msg = String(err || 'next failed');
            console.warn('Next track failed:', msg);
            window.Auralis.bridge.showToast(`Next failed: ${msg}`, 'error', 6000);
        }
    }

    async previous() {
        if (!window.Auralis || !window.Auralis.bridge) return;
        try {
            const res = await window.Auralis.bridge.invoke('previous_track');
            if (res) return;
            const tracks = (window.Auralis.bridge.tracks && window.Auralis.bridge.tracks.length)
                ? window.Auralis.bridge.tracks
                : (await window.Auralis.bridge.invoke('get_tracks', { filter: { limit: 200 } }).then(p=>p.tracks||[]).catch(()=>[]));
            if (!tracks.length) {
                window.Auralis.bridge.showToast('No previous track', 'info', 4000);
                return;
            }
            const curId = this.currentTrack && this.currentTrack.id;
            let idx = tracks.findIndex(t => String(t.id) === String(curId));
            if (idx <= 0) idx = tracks.length - 1;
            else idx = idx - 1;
            if (idx < 0) idx = 0;
            const prv = tracks[idx];
            if (prv && prv.id) await window.Auralis.bridge.playTrack(prv.id);
        } catch (err) {
            const msg = String(err || 'previous failed');
            console.warn('Previous track failed:', msg);
            window.Auralis.bridge.showToast(`Previous failed: ${msg}`, 'error', 6000);
        }
    }

    seek(secs) {
        if (!this.duration || this.duration <= 0 || !isFinite(this.duration) || !isFinite(secs)) return;
        this.progress = Math.max(0, Math.min(this.duration, secs));
        this.updateProgressUI();
        this.commitSeek();
    }

    seekToPercent(pct) {
        if (!isFinite(pct) || !this.duration || this.duration <= 0 || !isFinite(this.duration)) return;
        this.progress = pct * (this.duration || 0);
        this.updateProgressUI();
        this.updatePositionState();
    }

    commitSeek() {
        if (!this.duration || this.duration <= 0 || !isFinite(this.duration) || !isFinite(this.progress)) return;
        const pos = Math.floor(this.progress);
        if (!isFinite(pos) || pos < 0) return;
        this.updatePositionState();
        if (window._auralisStreamAudio) {
            try {
                window._auralisStreamAudio.currentTime = pos;
            } catch (e) {
                console.warn('Stream seek failed:', e);
            }
            return;
        }
        if (window.Auralis && window.Auralis.bridge) {
            window.Auralis.bridge.invoke('seek', { request: { position_secs: pos } }).catch((err)=>{
                const msg = String(err || 'seek failed');
                console.warn('Seek (commit) failed:', msg);
                window.Auralis.bridge.showToast(`Seek failed: ${msg}`, 'error', 5000);
            });
        }
    }

    updateMediaSessionMetadata(track) {
        if (!('mediaSession' in navigator) || !track) return;
        try {
            const title = track.title || 'Unknown Title';
            const artist = track.artist || 'Unknown Artist';
            const album = track.album || '';
            let artwork = [];
            if (track.album_art_path) {
                const src = (window.Auralis && window.Auralis.assetUrl) ? window.Auralis.assetUrl(track.album_art_path) : track.album_art_path;
                artwork.push({ src, sizes: '512x512', type: 'image/jpeg' });
            }
            navigator.mediaSession.metadata = new MediaMetadata({ title, artist, album, artwork });
            if (track.album_art_path && window.Auralis && window.Auralis.bridge && typeof window.Auralis.bridge.invoke === 'function') {
                window.Auralis.bridge.invoke('media_data_url', { path: track.album_art_path }).then((dataUrl) => {
                    if (dataUrl && typeof dataUrl === 'string' && dataUrl.startsWith('data:')) {
                        try {
                            navigator.mediaSession.metadata = new MediaMetadata({ title, artist, album, artwork: [{ src: dataUrl, sizes: '512x512', type: 'image/jpeg' }] });
                        } catch (_) {}
                    }
                }).catch(()=>{});
            }
        } catch (err) {
            console.warn('MediaSession metadata error:', err);
        }
    }

    updatePositionState() {
        if (!('mediaSession' in navigator) || typeof navigator.mediaSession.setPositionState !== 'function') return;
        try {
            if (!this.duration || this.duration <= 0 || !isFinite(this.duration) || !isFinite(this.progress)) return;
            const pos = Math.min(Math.max(0, this.progress), this.duration);
            if (!isFinite(pos) || pos < 0) return;
            navigator.mediaSession.setPositionState({ duration: this.duration, playbackRate: 1, position: pos });
        } catch (_) {}
    }

    async hydrateState() {
        if (window._auralisStreamAudio && !window._auralisStreamAudio.paused) {
            return;
        }
        try {
            if (!window.Auralis || !window.Auralis.bridge || typeof window.Auralis.bridge.invoke !== 'function') return;
            let state = null;
            try { state = await window.Auralis.bridge.invoke('get_now_playing'); } catch (_) {}
            // get_playback_state not exposed — removed fallback to avoid error spam
            // (previously invoked non-existent command and relied on silent catch).
            if (!state) return;
            const np = state.track ? state : null;
            if (!np || !np.track) {
                if (typeof state.volume === 'number') { this.volume = state.volume; this.updateVolumeUI(); }
                if (typeof state.shuffle_enabled === 'boolean') { this.shuffle = state.shuffle_enabled; }
                if (state.repeat_mode) { this.repeatMode = state.repeat_mode; this.updateShuffleRepeatUI(); }
                return;
            }
            this.currentTrack = np.track;
            this.duration = np.track.duration_secs || 0;
            this.progress = typeof np.position_secs === 'number' ? np.position_secs : 0;
            if (typeof np.volume === 'number') this.volume = np.volume;
            if (typeof np.shuffle_enabled === 'boolean') this.shuffle = np.shuffle_enabled;
            if (np.repeat_mode) this.repeatMode = np.repeat_mode;
            this.isPlaying = !!np.is_playing;
            this.isLiked = Boolean(np.track.is_favorite);
            if (window.Auralis.bridge.updatePlayerBar) window.Auralis.bridge.updatePlayerBar(np.track);
            this.updateProgressUI();
            this.updateVolumeUI();
            this.updateShuffleRepeatUI();
            this.updatePlayButton();
            this.updateLikeUI();
            this.updateFullScreenMetadata();
            this.updateMediaSessionMetadata(np.track);
            this.updatePositionState();
        } catch (e) {
            console.warn('hydrateState failed', e);
        }
    }

    seekRelative(delta) {
        if (!this.duration) return;
        this.progress = Math.max(0, Math.min(this.duration, this.progress + delta));
        this.updateProgressUI();
        this.commitSeek();
    }

    toggleShuffle() {
        this.shuffle = !this.shuffle;
        this.updateShuffleRepeatUI();
        if (window.Auralis && window.Auralis.bridge) {
            window.Auralis.bridge.invoke('set_shuffle', { enabled: this.shuffle }).catch((err)=>{
                const msg = String(err || 'shuffle failed');
                console.warn('Shuffle failed:', msg);
                window.Auralis.bridge.showToast(`Shuffle failed: ${msg}`, 'error', 5000);
                // rollback UI
                this.shuffle = !this.shuffle;
                this.updateShuffleRepeatUI();
            });
        }
    }

    cycleRepeat() {
        const modes = ['off', 'all', 'one'];
        const idx = modes.indexOf(this.repeatMode);
        const prev = this.repeatMode;
        this.repeatMode = modes[(idx + 1) % modes.length];
        this.updateShuffleRepeatUI();
        if (window.Auralis && window.Auralis.bridge) {
            window.Auralis.bridge.invoke('set_repeat_mode', { mode: this.repeatMode }).catch((err)=>{
                const msg = String(err || 'repeat failed');
                console.warn('Repeat failed:', msg);
                window.Auralis.bridge.showToast(`Repeat failed: ${msg}`, 'error', 5000);
                this.repeatMode = prev;
                this.updateShuffleRepeatUI();
            });
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

        const repeatIconName = this.repeatMode === 'one' ? 'repeat-1' : 'repeat';

        if (this.repeatBtn) {
            this.repeatBtn.classList.toggle('active', this.repeatMode !== 'off');
            this.repeatBtn.innerHTML = `<i data-lucide="${repeatIconName}"></i>`;
        }
        const fullRepeat = document.getElementById('player-full-repeat');
        if (fullRepeat) {
            fullRepeat.classList.toggle('active', this.repeatMode !== 'off');
            fullRepeat.innerHTML = `<i data-lucide="${repeatIconName}"></i>`;
        }

        if (window.lucide) window.lucide.createIcons();
    }

    updatePlayButton() {
        const iconName = this.isPlaying ? 'pause' : 'play';
        // Player bar button
        if (this.playBtn) {
            this.playBtn.innerHTML = `<i data-lucide="${iconName}"></i>`;
        }
        // Full screen player button
        const fullPlay = document.getElementById('player-full-play');
        if (fullPlay) {
            fullPlay.innerHTML = `<i data-lucide="${iconName}"></i>`;
        }
        if (window.lucide) window.lucide.createIcons();
        this.updateMediaSessionState();
    }

    updateMediaSessionState() {
        if (!('mediaSession' in navigator)) return;
        try {
            navigator.mediaSession.playbackState = this.isPlaying ? 'playing' : (this.currentTrack ? 'paused' : 'none');
        } catch (_) {}
    }

    updateProgressUI() {
        const pct = this.duration > 0 ? (this.progress / this.duration) * 100 : 0;
        const currentStr = this.formatTime(this.progress);
        const totalStr = this.formatTime(this.duration);

        // Player bar
        if (this.progressFill) {
            this.progressFill.style.width = `${pct}%`;
        }
        if (this.mobileProgressFill) {
            this.mobileProgressFill.style.width = `${pct}%`;
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
        if (window._auralisStreamAudio) {
            window._auralisStreamAudio.volume = this.volume;
        }
        if (window.Auralis && window.Auralis.bridge) {
            window.Auralis.bridge.invoke('set_volume', { volume: this.volume }).catch((err)=>{
                const msg = String(err || 'volume failed');
                console.warn('Set volume failed:', msg);
                window.Auralis.bridge.showToast(`Volume failed: ${msg}`, 'error', 5000);
            });
        }
    }

    toggleQueue(forceOpen) {
        if (!this.queuePanel) this.queuePanel = document.getElementById('queue-panel');
        if (!this.queuePanel) return;

        let isOpen;
        if (typeof forceOpen === 'boolean') {
            isOpen = forceOpen;
            this.queuePanel.classList.toggle('open', isOpen);
        } else {
            isOpen = this.queuePanel.classList.toggle('open');
        }
        if (this.queueBtn) {
            this.queueBtn.classList.toggle('active', isOpen);
        }
        if (isOpen) {
            this.renderQueuePanel();
        }
    }

    async renderQueuePanel() {
        if (!this.queuePanel) this.queuePanel = document.getElementById('queue-panel');
        const drawer = document.getElementById('player-full-queue-drawer');
        const containers = document.querySelectorAll('#queue-container, .queue-body');
        if (!this.queuePanel && !drawer && containers.length === 0) return;

        try {
            if (window.Auralis && window.Auralis.bridge) {
                const html = await window.Auralis.bridge.invoke('get_queue_html');
                if (html) {
                    if (this.queuePanel) this.queuePanel.innerHTML = html;
                    if (drawer) drawer.innerHTML = html;
                    containers.forEach((c) => {
                        if (c !== this.queuePanel && c !== drawer) c.innerHTML = html;
                    });
                    if (window.lucide) window.lucide.createIcons();
                }
            }
        } catch (e) {
            console.warn('Failed to render queue HTML:', e);
        }
    }

    async clearQueue() {
        if (window.Auralis && window.Auralis.bridge) {
            try {
                await window.Auralis.bridge.invoke('clear_queue');
                this.renderQueuePanel();
            } catch (e) {
                const msg = String(e || 'clear queue failed');
                console.error('Failed to clear queue:', msg);
                window.Auralis.bridge.showToast(`Clear queue failed: ${msg}`, 'error', 6000);
            }
        }
    }

    async removeFromQueue(indexOrId) {
        if (window.Auralis && window.Auralis.bridge) {
            try {
                if (typeof indexOrId === 'number') {
                    await window.Auralis.bridge.invoke('remove_from_queue', { index: indexOrId });
                } else if (typeof indexOrId === 'string' && /^\d+$/.test(indexOrId)) {
                    await window.Auralis.bridge.invoke('remove_from_queue', { index: parseInt(indexOrId, 10) });
                } else {
                    const q = await window.Auralis.bridge.invoke('get_queue');
                    let idx = -1;
                    if (q && q.tracks) {
                        idx = q.tracks.findIndex(t => String(t.id) === String(indexOrId));
                    }
                    if (idx >= 0) {
                        await window.Auralis.bridge.invoke('remove_from_queue', { index: idx });
                    }
                }
                this.renderQueuePanel();
            } catch (e) {
                const msg = String(e || 'remove failed');
                console.error('Failed to remove from queue:', msg);
                window.Auralis.bridge.showToast(`Queue remove failed: ${msg}`, 'error', 6000);
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
