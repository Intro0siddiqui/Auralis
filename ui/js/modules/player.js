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

    async playTrack(trackId) {
        if (!trackId) return;
        try {
            // Populate Rust queue context-aware before play so Next/Prev work (fixes empty queue dead Next)
            try {
                const ids = (this.tracks && this.tracks.length) ? this.tracks.map(t => t.id) : [trackId];
                if (ids.length > 1) {
                    await this.setQueue(ids, trackId);
                } else if (ids.length === 1) {
                    // Single track view — still set queue so Next wraps correctly
                    const cur = this.tracks.find(t => String(t.id) === String(trackId));
                    if (cur) await this.setQueue([String(cur.id)], String(cur.id));
                }
            } catch (_) {}
            const nowPlaying = await this.invoke('play', { track_id: trackId, trackId });
            if (nowPlaying && nowPlaying.track) {
                this.updatePlayerBar(nowPlaying.track);
            } else {
                const track = this.tracks.find(t => t.id === trackId);
                if (track) this.updatePlayerBar(track);
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
        const track = this.tracks.find(t => t.id === trackId);
        const currentFav = track ? Boolean(track.is_favorite) : (buttonEl && buttonEl.classList.contains('liked'));
        const nextFav = !currentFav;

        if (track) {
            track.is_favorite = nextFav;
        }

        if (buttonEl) {
            buttonEl.classList.toggle('liked', nextFav);
            buttonEl.style.color = nextFav ? 'var(--like)' : '';
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
            if (buttonEl) {
                buttonEl.classList.toggle('liked', currentFav);
                buttonEl.style.color = currentFav ? 'var(--like)' : '';
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
            if (actualTrack.album_art_path) {
                artwork.innerHTML = this.artImgTag(actualTrack.album_art_path, actualTrack.title);
            } else {
                artwork.innerHTML = `<i data-lucide="music"></i>`;
                if (window.lucide) window.lucide.createIcons();
            }
        }
    }
};
