/**
 * Player Module
 * Handles track playback triggers, player bar updates, favorites, and transport commands.
 */

export const playerMethods = {
    async playTrack(trackId) {
        if (!trackId) return;
        try {
            const nowPlaying = await this.invoke('play', { track_id: trackId, trackId });
            if (nowPlaying && nowPlaying.track) {
                this.updatePlayerBar(nowPlaying.track);
            } else {
                const track = this.tracks.find(t => t.id === trackId);
                if (track) this.updatePlayerBar(track);
            }
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
        const title = document.getElementById('track-title');
        const artist = document.getElementById('track-artist');
        const artwork = document.getElementById('current-artwork');

        if (title) title.textContent = track.title || 'No track playing';
        if (artist) artist.textContent = track.artist || 'Select a song';
        if (artwork) {
            if (track.album_art_path) {
                artwork.innerHTML = this.artImgTag(track.album_art_path, track.title);
            } else {
                artwork.innerHTML = `<i data-lucide="music"></i>`;
                if (window.lucide) window.lucide.createIcons();
            }
        }
    }
};
