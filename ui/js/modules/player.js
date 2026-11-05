/**
 * Player Module
 * Handles track playback triggers, player bar updates, favorites, and transport commands.
 */

export const playerMethods = {
    async setQueue(trackIds, currentId = null) {
        try {
            await this.invoke('set_queue', {
                trackIds: trackIds,
                currentId: currentId,
                track_ids: trackIds,
                current_id: currentId,
            });
        } catch (err) {
            console.error('Failed to set queue:', err);
        }
    },

    async playTrack(trackId, queueIds = null) {
        if (!trackId) return;

        // Cancel and stop any active or in-flight YouTube stream
        window.__auralisStreamRequestId = (window.__auralisStreamRequestId || 0) + 1;
        window.__auralisStreamInFlight = null;
        if (typeof this._stopStreamAudio === 'function') {
            this._stopStreamAudio();
        } else if (window.Auralis?._stopStreamAudio) {
            window.Auralis._stopStreamAudio();
        } else if (window.Auralis?.bridge?._stopStreamAudio) {
            window.Auralis.bridge._stopStreamAudio();
        } else if (window._auralisStreamAudio) {
            try {
                window._auralisStreamAudio.pause();
                window._auralisStreamAudio.currentTime = 0;
                window._auralisStreamAudio.removeAttribute('src');
                window._auralisStreamAudio.load();
            } catch (_) {}
            window._auralisStreamAudio = null;
        }

        let track = (this.tracks && this.tracks.find(t => String(t.id) === String(trackId)));

        // Optimistic DOM extraction if track is not in memory (fixes Bug 3 artwork & title latency)
        if (!track) {
            const rowEl = document.querySelector(`[data-track-id="${trackId}"]`);
            if (rowEl) {
                const titleEl = rowEl.querySelector('.track-row-title');
                const subEl = rowEl.querySelector('.track-row-subtitle');
                const imgEl = rowEl.querySelector('.track-row-artwork img');
                track = {
                    id: trackId,
                    title: titleEl ? titleEl.textContent : 'Playing…',
                    artist: subEl ? subEl.textContent.split('—')[0].trim() : '',
                    album_art_path: imgEl ? (imgEl.dataset.artPath || imgEl.src) : null,
                    _cachedImgSrc: imgEl ? imgEl.src : null,
                };
            }
        }

        // Instant visual feedback — zero delay
        if (window.Auralis?.player) {
            window.Auralis.player.highlightActiveTrack?.(trackId, true);
            window.Auralis.player.isPlaying = true;
            window.Auralis.player.updatePlayButton();
        }
        if (track) {
            this.updatePlayerBar(track);
            if (track.file_path && String(track.file_path).startsWith('sync://')) {
                this.showToast(`Fetching “${track.title || 'track'}” from paired device…`, 'info', 4000);
            }
        }

        try {
            // Seed queue in parallel so playback invocation is never blocked
            let ids = (Array.isArray(queueIds) && queueIds.length > 0)
                ? queueIds
                : (this.tracks && this.tracks.length)
                    ? this.tracks.map(t => t.id)
                    : [];

            if (ids.length <= 1) {
                // Collect surrounding tracks from visible DOM list
                const domIds = Array.from(document.querySelectorAll('.page-library [data-track-id], .track-list [data-track-id], #playlist-track-list [data-track-id]'))
                    .map(el => el.dataset.trackId)
                    .filter((v, i, a) => v && a.indexOf(v) === i);
                if (domIds.includes(String(trackId))) {
                    ids = domIds;
                }
            }

            if (ids.length === 0) {
                ids = [trackId];
            }

            const queuePromise = (ids.length > 1)
                ? this.setQueue(ids, trackId).catch(() => {})
                : this.setQueue([trackId], trackId).catch(() => {});

            const nowPlaying = await this.invoke('play', { track_id: trackId, trackId });
            await queuePromise;

            if (nowPlaying && nowPlaying.track) {
                this.updatePlayerBar(nowPlaying.track);
            } else if (track) {
                this.updatePlayerBar(track);
            }
            // Immediate bar sync for mobile where HTMX swap may not have fired yet
            try { if (window.Auralis && window.Auralis.player) window.Auralis.player.hydrateState().catch(()=>{}); } catch (_) {}
        } catch (err) {
            const msg = String(err || 'unknown playback error');
            console.error(`[playTrack] failed id=${trackId}:`, msg);
            try { window.__auralisLastPlaybackError = { at: new Date().toISOString(), trackId, msg }; } catch (_) {}
            // Show full error, keep on screen longer so user can copy
            this.showToast(`Playback failed [${String(trackId).slice(0,8)}]: ${msg}`, 'error', 8000);
            // Also surface in player bar subtitle for persistent visibility
            const artistEl = document.getElementById('track-artist');
            if (artistEl) {
                artistEl.textContent = msg.slice(0, 140);
                artistEl.style.color = 'var(--danger, #ff4d4f)';
                artistEl.title = msg;
            }
        }
    },

    async addToQueue(trackId) {
        if (!trackId) return;
        try {
            await this.invoke('add_to_queue', { track_id: trackId, trackId });
            this.showToast('Added to queue', 'success');
        } catch (err) {
            console.error('Failed to add to queue:', err);
            this.showToast(`Failed to add to queue: ${err}`, 'error');
        }
    },

    async playNextTrack(trackId) {
        if (!trackId) return;
        try {
            await this.invoke('play_next', { track_id: trackId, trackId });
            this.showToast('Playing next', 'success');
        } catch (err) {
            console.error('Failed to insert next in queue:', err);
            this.showToast(`Failed to set play next: ${err}`, 'error');
        }
    },

    async removeFromQueue(target) {
        try {
            let index = target;
            if (typeof target === 'string') {
                const parsed = parseInt(target, 10);
                if (!isNaN(parsed) && String(parsed) === target) {
                    index = parsed;
                } else {
                    const q = await this.invoke('get_queue');
                    if (q && q.tracks) {
                        const found = q.tracks.findIndex(t => String(t.id) === String(target));
                        if (found !== -1) {
                            index = found;
                        } else {
                            return;
                        }
                    }
                }
            }
            await this.invoke('remove_from_queue', { index });
            if (window.Auralis && window.Auralis.player && typeof window.Auralis.player.renderQueuePanel === 'function') {
                window.Auralis.player.renderQueuePanel();
            }
        } catch (err) {
            console.error('Failed to remove from queue:', err);
            this.showToast(`Failed to remove from queue: ${err}`, 'error');
        }
    },

    closeQueue() {
        if (window.Auralis && window.Auralis.player) {
            if (typeof window.Auralis.player.toggleFullScreenQueue === 'function') {
                window.Auralis.player.toggleFullScreenQueue(false);
            }
            if (typeof window.Auralis.player.toggleQueue === 'function') {
                window.Auralis.player.toggleQueue(false);
            }
        }
        const drawer = document.getElementById('player-full-queue-drawer');
        if (drawer) drawer.classList.remove('open');
        const backdrop = document.getElementById('player-full-queue-backdrop');
        if (backdrop) backdrop.classList.remove('open');
        const queuePanel = document.getElementById('queue-panel');
        if (queuePanel) queuePanel.classList.remove('open');
        const queueBtn = document.getElementById('player-full-queue-btn') || document.getElementById('queue-btn');
        if (queueBtn) queueBtn.classList.remove('active');
    },

    async clearQueue() {
        try {
            await this.invoke('clear_queue');
            this.showToast('Queue cleared', 'info');
            if (window.Auralis && window.Auralis.player && typeof window.Auralis.player.renderQueuePanel === 'function') {
                window.Auralis.player.renderQueuePanel();
            }
        } catch (err) {
            console.error('Failed to clear queue:', err);
            this.showToast(`Failed to clear queue: ${err}`, 'error');
        }
    },

    async playNext() {
        try {
            if (window.Auralis && window.Auralis.player && typeof window.Auralis.player.next === 'function') {
                window.Auralis.player.next();
            } else {
                await this.invoke('next_track');
            }
        } catch (err) {
            console.error('Play next error:', err);
        }
    },

    async playPrevious() {
        try {
            if (window.Auralis && window.Auralis.player && typeof window.Auralis.player.previous === 'function') {
                window.Auralis.player.previous();
            } else {
                await this.invoke('previous_track');
            }
        } catch (err) {
            console.error('Play previous error:', err);
        }
    },

    async togglePlayPause() {
        try {
            if (window.Auralis && window.Auralis.player && typeof window.Auralis.player.togglePlay === 'function') {
                window.Auralis.player.togglePlay();
            } else {
                await this.invoke('pause');
            }
        } catch (err) {
            console.error('Toggle play/pause error:', err);
        }
    },

    async toggleTrackFavorite(trackId, buttonEl) {
        let el = null;
        if (buttonEl instanceof Element) {
            el = buttonEl;
        } else if (typeof buttonEl === 'string' && buttonEl !== 'this' && buttonEl.trim()) {
            el = document.getElementById(buttonEl);
        }

        const track = (this.tracks && this.tracks.find(t => String(t.id) === String(trackId)));
        const currentFav = track ? Boolean(track.is_favorite) : Boolean(el?.classList?.contains('liked'));
        const nextFav = !currentFav;

        if (track) {
            track.is_favorite = nextFav;
        }

        if (el && el.classList) {
            el.classList.toggle('liked', nextFav);
            el.style.color = nextFav ? 'var(--like)' : '';
        }

        if (window.Auralis && window.Auralis.player && window.Auralis.player.currentTrack && window.Auralis.player.currentTrack.id === trackId) {
            window.Auralis.player.isLiked = nextFav;
            window.Auralis.player.currentTrack.is_favorite = nextFav;
            window.Auralis.player.updateLikeUI();
        }

        try {
            await this.invoke('set_track_favorite', { id: trackId, favorite: nextFav });
            this.showToast(nextFav ? 'Added to Liked Songs' : 'Removed from Liked Songs', 'info');
        } catch (err) {
            console.error('Failed to update track favorite:', err);
            if (track) track.is_favorite = currentFav;
            if (el && el.classList) {
                el.classList.toggle('liked', currentFav);
                el.style.color = currentFav ? 'var(--like)' : '';
            }
            this.showToast(`Failed to update favorite: ${err}`, 'error');
        }
    },

    updatePlayerBar(track) {
        const actualTrack = (track && track.track) ? track.track : track;
        if (!actualTrack) return;
        const title = document.getElementById('track-title');
        const artist = document.getElementById('track-artist');
        const artwork = document.getElementById('current-artwork');

        if (title) title.textContent = actualTrack.title || 'No track playing';
        if (artist) {
            artist.textContent = actualTrack.artist || 'Select a song';
            artist.style.color = '';
        }
        if (artwork) {
            const artPath = actualTrack.album_art_path;
            const cachedSrc = actualTrack._cachedImgSrc;
            if (cachedSrc || artPath) {
                const src = cachedSrc || this.assetUrl(artPath);
                const existingImg = artwork.querySelector('img');
                if (!existingImg || existingImg.getAttribute('src') !== src) {
                    artwork.innerHTML = `<img src="${src}" alt="${this.escapeHtml(actualTrack.title || '')}" data-art-path="${this.escapeHtml(artPath || '')}" style="width:100%;height:100%;object-fit:cover;border-radius:inherit;" onerror="this.style.display='none'">`;
                }
            } else {
                artwork.innerHTML = `<div class="artwork-placeholder" style="width:100%;height:100%;display:flex;align-items:center;justify-content:center;"><i data-lucide="disc"></i></div>`;
                if (window.lucide) window.lucide.createIcons();
            }
        }
    },

    async requestNotificationPermission() {
        try {
            await this.invoke('request_notification_permission');
        } catch (err) {
            console.warn('request_notification_permission failed:', err);
        }
    },

    async openFullScreenPlayer() {
        if (window.Auralis?.player && typeof window.Auralis.player.openFullScreenPlayer === 'function') {
            return await window.Auralis.player.openFullScreenPlayer();
        }
        if (window.AuralisPlayer && typeof window.AuralisPlayer.openFullScreenPlayer === 'function') {
            return await window.AuralisPlayer.openFullScreenPlayer();
        }
    }
};
