/*
 * youtube.js resolver
 * -------------------
 * Thin wrapper around the vendored `youtubei.js` library (ui/vendor/, loaded
 * on demand as an ES module — no CDN dependency). It turns a user-facing URL
 * (YouTube video/playlist, or a direct audio file link) into a resolved object
 * the Rust `download_audio` command can stream directly:
 *
 *   { kind: 'track', stream_url, title, ext, total_bytes, thumbnail, platform }
 *   { kind: 'playlist', items: [ { url, title }, ... ] }
 *
 * Resolution (InnerTube/signature handling) stays in JS; the Rust side only
 * fetches bytes, so no yt-dlp / ffmpeg sidecars are required.
 */

class YouTubeResolver {
    constructor() {
        this._modulePromise = null;
        this._clients = {};
    }

    async _loadModule() {
        if (!this._modulePromise) {
            this._modulePromise = import('../vendor/youtubei.esm.mjs').catch(() => import('./vendor/youtubei.esm.mjs'));
        }
        return this._modulePromise;
    }

    async _client(opts = {}) {
        const mod = await this._loadModule();
        const YouTube = mod.default || mod.YouTube || (mod.default && mod.default.YouTube);
        if (!YouTube) throw new Error('youtube.js failed to expose YouTube');

        const key = [opts.cookie || '', opts.poToken || ''].join('|');
        if (!this._clients[key]) {
            const cfg = {};
            if (opts.cookie) cfg.cookie = opts.cookie;
            if (opts.poToken) cfg.poToken = opts.poToken;
            try {
                this._clients[key] = new YouTube(cfg);
            } catch (_) {
                this._clients[key] = new YouTube();
            }
        }
        return this._clients[key];
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

    async resolve(rawUrl, opts = {}) {
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
            const items = await this.resolvePlaylist(url, opts);
            return { kind: 'playlist', items };
        }

        const client = await this._client(opts);
        const info = await client.getInfo(url);
        const bi = info.basic_info || {};
        const sd = info.streaming_data || {};

        const container = opts.container === 'mp4' || opts.container === 'webm' ? opts.container : null;
        const quality = opts.quality || 'best';
        let fmt = null;

        if (typeof sd.chooseFormat === 'function') {
            // Prefer an mp4 (m4a/aac) stream so downloaded files stay playable
            // by the in-app rodio backend (no opus/webm decoder). When the user
            // pins a container, honour it first and fall back to any audio.
            const attempts = container ? [container, null] : ['mp4', null];
            for (const c of attempts) {
                if (fmt) break;
                const attempt = { type: 'audio', quality };
                if (c) attempt.format = c;
                try {
                    const cand = sd.chooseFormat(attempt);
                    if (cand && cand.url) fmt = cand;
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

    async resolvePlaylist(rawUrl, opts = {}) {
        const url = (rawUrl || '').trim();
        const client = await this._client(opts);
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

    async search(query, opts = {}) {
        const q = (query || '').trim();
        if (!q) return [];
        const client = await this._client(opts);
        const res = await client.search(q);

        const raw = res.results || res.contents || [];
        const out = [];
        for (const r of raw) {
            const isVideo = r && (r.type === 'video' || r.type === 'reel');
            if (!isVideo || !r.id) continue;
            out.push({
                id: r.id,
                title: String(r.title || 'Unknown').trim() || 'Unknown',
                url: `https://www.youtube.com/watch?v=${r.id}`,
                channel: (r.author && r.author.name) ? String(r.author.name) : '',
            });
            if (out.length >= 10) break;
        }
        return out;
    }
}

window.AuralisYouTube = new YouTubeResolver();