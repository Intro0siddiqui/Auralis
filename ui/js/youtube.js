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

/**
 * Native HTTP fetch bridge that delegates requests to Rust's reqwest client
 * when running inside Tauri, completely bypassing Android WebView CORS restrictions.
 */
async function nativeFetch(input, init = {}) {
    let url = typeof input === 'string' ? input : (input?.url || String(input));
    let method = init?.method || input?.method || 'GET';
    const headers = {};

    if (input?.headers) {
        if (typeof input.headers.forEach === 'function') {
            input.headers.forEach((v, k) => { headers[k] = v; });
        } else if (typeof input.headers === 'object') {
            Object.assign(headers, input.headers);
        }
    }

    if (init?.headers) {
        if (typeof init.headers.forEach === 'function') {
            init.headers.forEach((v, k) => { headers[k] = v; });
        } else if (Array.isArray(init.headers)) {
            for (const [k, v] of init.headers) headers[k] = v;
        } else if (typeof init.headers === 'object') {
            Object.assign(headers, init.headers);
        }
    }

    let body = null;
    const rawBody = init?.body !== undefined ? init.body : input?.body;
    if (rawBody !== undefined && rawBody !== null) {
        if (typeof rawBody === 'string') {
            body = rawBody;
        } else if (rawBody instanceof Uint8Array || rawBody instanceof ArrayBuffer) {
            body = new TextDecoder().decode(rawBody);
        } else if (typeof rawBody === 'object') {
            try { body = JSON.stringify(rawBody); } catch (_) { body = String(rawBody); }
        } else {
            body = String(rawBody);
        }
    }

    try {
        const invoke =
            window.__TAURI__?.core?.invoke ||
            window.__TAURI__?.invoke ||
            window.Auralis?.bridge?.invoke;

        if (typeof invoke === 'function') {
            const resp = await invoke('http_fetch', {
                request: { url, method, headers, body }
            });

            return new Response(resp.body, {
                status: resp.status,
                statusText: resp.status_text,
                headers: new Headers(resp.headers),
            });
        }
    } catch (err) {
        console.warn('Native http_fetch failed or unavailable, falling back to window.fetch:', err);
    }

    return window.fetch(input, init);
}

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
        const { Platform } = mod;
        const Ctor = mod.default || mod.Innertube || mod.YouTube;
        if (!Ctor) throw new Error('youtube.js failed to expose Innertube/YouTube');

        // Ensure Platform evaluation & native network fetching are configured
        if (Platform && typeof Platform.load === 'function' && Platform.shim) {
            Platform.load({
                ...Platform.shim,
                fetch: nativeFetch,
                eval: async (data, env) => {
                    const fn = new Function(...Object.keys(env || {}), data.output);
                    return fn(...Object.values(env || {}));
                }
            });
        }

        const key = [opts.cookie || '', opts.poToken || ''].join('|');
        if (!this._clients[key]) {
            const cfg = {
                generate_session_locally: true,
                retrieve_player: true,
            };
            if (opts.cookie) cfg.cookie = opts.cookie;
            if (opts.poToken) cfg.poToken = opts.poToken;

            try {
                if (typeof Ctor.create === 'function') {
                    this._clients[key] = await Ctor.create(cfg);
                } else {
                    this._clients[key] = new Ctor(cfg);
                }
            } catch (err) {
                console.warn('Failed to initialize Innertube with options, falling back to default create:', err);
                if (typeof Ctor.create === 'function') {
                    this._clients[key] = await Ctor.create();
                } else {
                    this._clients[key] = new Ctor();
                }
            }
        }
        return this._clients[key];
    }

    isDirectAudio(url) {
        return /\.(mp3|m4a|aac|ogg|oga|opus|wav|flac|webm)(\?.*)?$/i.test(url);
    }

    extFromMime(mime) {
        if (!mime) return 'm4a';
        const m = String(mime).toLowerCase();
        if (m.includes('mp4') || m.includes('m4a') || m.includes('aac')) return 'm4a';
        if (m.includes('webm') || m.includes('opus')) return 'webm';
        if (m.includes('ogg')) return 'ogg';
        if (m.includes('wav')) return 'wav';
        if (m.includes('flac')) return 'flac';
        if (m.includes('mpeg') || m.includes('mp3')) return 'mp3';
        return 'm4a';
    }

    extFromUrl(url) {
        const m = url.split('?')[0].match(/\.([a-z0-9]+)$/i);
        return m ? m[1].toLowerCase() : 'mp3';
    }

    pickThumb(thumb) {
        if (!thumb) return null;
        if (typeof thumb === 'string') return thumb;
        try {
            if (Array.isArray(thumb) && thumb.length) {
                return thumb[thumb.length - 1]?.url || thumb[0]?.url || null;
            }
            if (Array.isArray(thumb.contents) && thumb.contents.length) {
                return thumb.contents[thumb.contents.length - 1]?.url || thumb.contents[0]?.url || null;
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

    extractVideoId(rawUrl) {
        const url = (rawUrl || '').trim();
        const idMatch = url.match(/(?:v=|youtu\.be\/|shorts\/|embed\/|^)([a-zA-Z0-9_-]{11})/);
        return idMatch ? idMatch[1] : url;
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
        const videoId = this.extractVideoId(url);

        let info = null;
        let lastErr = null;

        const hasAudioFormats = (r) => {
            if (!r) return false;
            if (typeof r.chooseFormat === 'function') {
                try {
                    const f = r.chooseFormat({ type: 'audio', quality: 'best' });
                    if (f && (f.url || f.signature_cipher || f.cipher)) return true;
                } catch (_) {}
            }
            const sd = r.streaming_data;
            if (sd) {
                if (sd.adaptive_formats?.some((f) => f && f.has_audio)) return true;
                if (sd.formats?.some((f) => f && f.has_audio)) return true;
            }
            return false;
        };

        // Attempt multiple client contexts (Default/WEB, Android, Mobile Web, Music, iOS)
        const clientAttempts = [
            async () => client.getInfo(videoId),
            async () => client.getInfo(videoId, { client: 'ANDROID' }),
            async () => client.getInfo(videoId, { client: 'MWEB' }),
            async () => client.music ? client.music.getInfo(videoId) : null,
            async () => client.getInfo(videoId, { client: 'IOS' }),
        ];

        let fallbackInfo = null;
        for (const attempt of clientAttempts) {
            try {
                const res = await attempt();
                if (res) {
                    if (!fallbackInfo && (res.basic_info || res.streaming_data)) {
                        fallbackInfo = res;
                    }
                    if (hasAudioFormats(res)) {
                        info = res;
                        break;
                    }
                }
            } catch (e) {
                lastErr = e;
            }
        }

        if (!info) {
            info = fallbackInfo;
        }

        if (!info) {
            throw new Error(`Failed to retrieve video stream: ${lastErr?.message || 'Video unavailable'}`);
        }

        const bi = info.basic_info || {};
        const sd = info.streaming_data || {};

        const container = opts.container === 'mp4' || opts.container === 'webm' ? opts.container : null;
        const quality = opts.quality || 'best';
        let fmt = null;

        if (typeof info.chooseFormat === 'function') {
            const attempts = container ? [container, null] : ['mp4', null];
            for (const c of attempts) {
                if (fmt) break;
                const attempt = { type: 'audio', quality };
                if (c) attempt.format = c;
                try {
                    const cand = info.chooseFormat(attempt);
                    if (cand && (cand.url || cand.signature_cipher || cand.cipher)) fmt = cand;
                } catch (_) {}
            }
        }

        if (!fmt) {
            fmt =
                sd.adaptive_formats?.find((f) => f && f.has_audio && !f.has_video) ||
                sd.formats?.find((f) => f && f.has_audio) ||
                sd.adaptive_formats?.[0] ||
                sd.formats?.[0];
        }

        if (!fmt) throw new Error('No audio stream found for this video');

        let streamUrl = fmt.url;
        if (!streamUrl && typeof fmt.decipher === 'function' && client.session?.player) {
            try {
                streamUrl = await fmt.decipher(client.session.player);
            } catch (decErr) {
                console.warn('Decipher attempt failed on format:', decErr);
            }
        }

        if (!streamUrl) {
            // Fallback: iterate over all available formats looking for any decipherable audio stream
            const allCandidates = [...(sd.adaptive_formats || []), ...(sd.formats || [])];
            for (const candidate of allCandidates) {
                if (candidate && candidate.has_audio) {
                    if (candidate.url) {
                        streamUrl = candidate.url;
                        fmt = candidate;
                        break;
                    }
                    if (typeof candidate.decipher === 'function' && client.session?.player) {
                        try {
                            const deciphered = await candidate.decipher(client.session.player);
                            if (deciphered) {
                                streamUrl = deciphered;
                                fmt = candidate;
                                break;
                            }
                        } catch (_) {}
                    }
                }
            }
        }

        if (!streamUrl) throw new Error('Unable to extract playable audio URL');

        const ext = this.extFromMime(fmt.mime_type);
        const title = String(bi.title || 'YouTube Audio').trim() || 'YouTube Audio';
        const thumb = this.pickThumb(bi.thumbnail);
        const totalRaw = fmt.content_length ? Number(fmt.content_length) : NaN;
        const total = isNaN(totalRaw) ? null : totalRaw;

        return {
            kind: 'track',
            stream_url: streamUrl,
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
        const playlistId = match[1];

        let playlist = null;
        try {
            playlist = await client.getPlaylist(playlistId);
        } catch (_) {
            if (client.music) {
                playlist = await client.music.getPlaylist(playlistId);
            }
        }
        if (!playlist) throw new Error('Playlist not found');

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
        
        let res = null;
        try {
            res = await client.search(q);
        } catch (_) {
            if (client.music) {
                res = await client.music.search(q);
            }
        }
        if (!res) return [];

        const raw = res.videos || res.results || res.contents || [];
        const out = [];
        for (const r of raw) {
            const id = r.id || r.videoId;
            if (!id) continue;
            const title = typeof r.title === 'string' ? r.title : (r.title?.text || String(r.title || 'Unknown'));
            const author = typeof r.author === 'string' ? r.author : (r.author?.name || (r.artists && r.artists[0]?.name) || '');
            out.push({
                id,
                title: String(title).trim() || 'Unknown',
                url: `https://www.youtube.com/watch?v=${id}`,
                channel: String(author).trim(),
            });
            if (out.length >= 10) break;
        }
        return out;
    }
}

window.AuralisYouTube = new YouTubeResolver();