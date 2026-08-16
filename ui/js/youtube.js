/*
 * youtube.js resolver
 * -------------------
 * Thin wrapper around the `youtube.js` library (loaded on demand from a CDN as
 * an ES module). It turns a user-facing URL (YouTube video/playlist, or a
 * direct audio file link) into a resolved object the Rust `download_audio`
 * command can stream directly:
 *
 *   { kind: 'track', stream_url, title, ext, total_bytes, thumbnail, platform }
 *   { kind: 'playlist', items: [ { url, title }, ... ] }
 *
 * Resolution (InnerTube/signature handling) stays in JS; the Rust side only
 * fetches bytes, so no yt-dlp / ffmpeg sidecars are required.
 */

class YouTubeResolver {
    constructor() {
        this._module = null;
        this._client = null;
    }

    async _module() {
        if (!this._module) {
            this._module = await import('https://esm.sh/youtube.js@latest');
        }
        return this._module;
    }

    async _client() {
        if (!this._client) {
            const mod = await this._module();
            const YouTube = mod.YouTube || (mod.default && mod.default.YouTube);
            if (!YouTube) throw new Error('youtube.js failed to expose YouTube');
            this._client = new YouTube();
        }
        return this._client;
    }

    isDirectAudio(url) {
        return /\.(mp3|m4a|aac|ogg|oga|opus|wav|flac|webm)(\?.*)?$/i.test(url);
    }

    extFromMime(mime) {
        if (!mime) return 'webm';
        const m = String(mime).toLowerCase();
        if (m.includes('webm') || m.includes('opus')) return 'webm';
        if (m.includes('mp4') || m.includes('m4a') || m.includes('aac')) return 'm4a';
        if (m.includes('ogg')) return 'ogg';
        if (m.includes('wav')) return 'wav';
        if (m.includes('flac')) return 'flac';
        if (m.includes('mpeg') || m.includes('mp3')) return 'mp3';
        return 'webm';
    }

    extFromUrl(url) {
        const m = url.split('?')[0].match(/\.([a-z0-9]+)$/i);
        return m ? m[1].toLowerCase() : 'mp3';
    }

    pickThumb(thumb) {
        if (!thumb) return null;
        if (typeof thumb === 'string') return thumb;
        try {
            if (Array.isArray(thumb.contents) && thumb.contents.length) {
                return thumb.contents[0].url || null;
            }
            if (thumb.url) return thumb.url;
        } catch (_) {}
        return null;
    }

    basename(url) {
        try {
            const u = new URL(url);
            const last = u.pathname.split('/').filter(Boolean).pop() || 'audio_track';
            return decodeURIComponent(last);
        } catch (_) {
            return 'audio_track';
        }
    }

    isPlaylistUrl(url) {
        return /[?&]list=([^&]+)/.test(url) && !/watch\?/.test(url);
    }

    async resolve(rawUrl) {
        const url = (rawUrl || '').trim();
        if (!url) throw new Error('Empty URL');

        if (this.isDirectAudio(url)) {
            return {
                kind: 'track',
                stream_url: url,
                title: this.basename(url),
                ext: this.extFromUrl(url),
                total_bytes: null,
                thumbnail: null,
                platform: 'direct',
            };
        }

        if (this.isPlaylistUrl(url)) {
            const items = await this.resolvePlaylist(url);
            return { kind: 'playlist', items };
        }

        const client = await this._client();
        const info = await client.getInfo(url);
        const bi = info.basic_info || {};
        const sd = info.streaming_data || {};

        let fmt = null;
        if (typeof sd.chooseFormat === 'function') {
            // Prefer an mp4 (m4a/aac) audio stream so the downloaded file is
            // playable by the in-app rodio backend, which has no opus/webm
            // decoder. Fall back to the best audio stream if mp4 is unavailable.
            try {
                fmt = sd.chooseFormat({ type: 'audio', quality: 'best', format: 'mp4' });
            } catch (_) {}
            if (!fmt) {
                try {
                    fmt = sd.chooseFormat({ type: 'audio', quality: 'best' });
                } catch (_) {}
            }
        }
        if (!fmt && Array.isArray(sd.adaptive_formats)) {
            fmt =
                sd.adaptive_formats.find(
                    (f) => f && f.type && String(f.type).startsWith('audio')
                ) || sd.adaptive_formats[0];
        }
        if (!fmt || !fmt.url) throw new Error('No audio stream found for this video');

        const ext = this.extFromMime(fmt.mime_type);
        const title = String(bi.title || 'YouTube Audio').trim() || 'YouTube Audio';
        const thumb = this.pickThumb(bi.thumbnail);
        const totalRaw = fmt.content_length ? Number(fmt.content_length) : NaN;
        const total = isNaN(totalRaw) ? null : totalRaw;

        return {
            kind: 'track',
            stream_url: fmt.url,
            title,
            ext,
            total_bytes: total,
            thumbnail: thumb,
            platform: 'youtube',
        };
    }

    async resolvePlaylist(rawUrl) {
        const url = (rawUrl || '').trim();
        const client = await this._client();
        const match = url.match(/[?&]list=([^&]+)/);
        if (!match) throw new Error('Not a playlist URL');
        const playlist = await client.getPlaylist(match[1]);

        const source = playlist.videos || playlist.contents || [];
        const items = [];
        for (const v of source) {
            let id = v.id;
            if (!id && v.url) {
                const m = v.url.match(/[?&]v=([^&]+)/);
                if (m) id = m[1];
            }
            if (!id) continue;
            const videoUrl =
                typeof id === 'string'
                    ? `https://www.youtube.com/watch?v=${id}`
                    : `https://www.youtube.com/watch?v=${id[1]}`;
            items.push({ url: videoUrl, title: String(v.title || 'Unknown') });
        }
        if (items.length === 0) throw new Error('Playlist contained no videos');
        return items;
    }
}

window.AuralisYouTube = new YouTubeResolver();
