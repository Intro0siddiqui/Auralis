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
    const cleanHeaders = {};

    const extractHeaders = (hdrs) => {
        if (!hdrs) return;
        if (typeof hdrs.forEach === 'function') {
            hdrs.forEach((v, k) => {
                if (v !== undefined && v !== null) cleanHeaders[String(k)] = String(v);
            });
        } else if (Array.isArray(hdrs)) {
            for (const [k, v] of hdrs) {
                if (v !== undefined && v !== null) cleanHeaders[String(k)] = String(v);
            }
        } else if (typeof hdrs === 'object') {
            for (const [k, v] of Object.entries(hdrs)) {
                if (v !== undefined && v !== null) cleanHeaders[String(k)] = String(v);
            }
        }
    };

    if (input?.headers) extractHeaders(input.headers);
    if (init?.headers) extractHeaders(init.headers);

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
            window.__TAURI_INTERNALS__?.invoke ||
            window.Auralis?.bridge?.invoke;

        if (typeof invoke === 'function') {
            const resp = await invoke('http_fetch', {
                request: { url, method, headers: cleanHeaders, body }
            });

            return new Response(resp.body, {
                status: resp.status,
                statusText: resp.status_text,
                headers: new Headers(resp.headers),
            });
        }
    } catch (err) {
        console.warn('[YouTubeResolver] Native http_fetch failed:', err?.message || err);
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
                retrieve_player: false,
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

        const isAudioFormat = (f) => {
            if (!f) return false;
            if (f.has_audio && !f.has_video) return true;
            if (typeof f.mime_type === 'string' && f.mime_type.startsWith('audio/')) return true;
            if (f.has_audio) return true;
            return false;
        };

        const hasDirectOrDecipherableAudio = (r) => {
            if (!r || !r.streaming_data) return false;
            const sd = r.streaming_data;
            const all = [...(sd.adaptive_formats || []), ...(sd.formats || [])];
            return all.some((f) => isAudioFormat(f) && Boolean(f.url || f.signature_cipher || f.cipher || typeof f.decipher === 'function'));
        };

        // 1. First attempt: Direct raw player API query across reliable mobile/embedded clients (IOS, ANDROID_VR, ANDROID, WEB)
        // This directly fetches raw streamingData with direct unthrottled HTTPS audio stream URLs without parser overhead.
        if (client.actions?.execute) {
            for (const cl of ['IOS', 'ANDROID_VR', 'ANDROID', 'WEB']) {
                try {
                    const raw = await client.actions.execute('/player', { videoId, client: cl });
                    const sd = raw?.data?.streamingData;
                    if (sd && (sd.adaptiveFormats?.length || sd.formats?.length)) {
                        const adapt = (sd.adaptiveFormats || []).map((f) => ({
                            ...f,
                            itag: f.itag,
                            mime_type: f.mimeType,
                            bitrate: f.bitrate,
                            url: f.url,
                            signature_cipher: f.signatureCipher || f.cipher,
                            has_audio: Boolean(f.mimeType?.startsWith('audio/') || f.audioQuality),
                            has_video: Boolean(f.mimeType?.startsWith('video/')),
                            content_length: f.contentLength ? parseInt(f.contentLength, 10) : undefined,
                        }));
                        const fmts = (sd.formats || []).map((f) => ({
                            ...f,
                            itag: f.itag,
                            mime_type: f.mimeType,
                            bitrate: f.bitrate,
                            url: f.url,
                            signature_cipher: f.signatureCipher || f.cipher,
                            has_audio: Boolean(f.mimeType?.startsWith('audio/') || f.audioQuality),
                            has_video: Boolean(f.mimeType?.startsWith('video/')),
                            content_length: f.contentLength ? parseInt(f.contentLength, 10) : undefined,
                        }));
                        const vd = raw.data?.videoDetails || {};
                        const parsed = {
                            basic_info: {
                                id: videoId,
                                title: vd.title || 'YouTube Track',
                                author: vd.author,
                                channel_id: vd.channelId,
                                duration: vd.lengthSeconds ? parseInt(vd.lengthSeconds, 10) : undefined,
                                thumbnail: vd.thumbnail?.thumbnails || [],
                            },
                            streaming_data: {
                                adaptive_formats: adapt,
                                formats: fmts,
                            },
                        };
                        if (hasDirectOrDecipherableAudio(parsed)) {
                            info = parsed;
                            break;
                        }
                    }
                } catch (e) {
                    lastErr = e;
                }
            }
        }

        // 2. Second attempt: High-level Innertube getInfo fallback
        if (!info) {
            const clientAttempts = [
                async () => client.getInfo(videoId, { client: 'IOS' }),
                async () => client.getInfo(videoId, { client: 'ANDROID_VR' }),
                async () => client.getInfo(videoId, { client: 'ANDROID' }),
                async () => client.getInfo(videoId, { client: 'TV' }),
                async () => client.getInfo(videoId, { client: 'MWEB' }),
                async () => client.getInfo(videoId),
            ];

            let fallbackInfo = null;
            for (const attempt of clientAttempts) {
                try {
                    const res = await attempt();
                    if (res && res.streaming_data) {
                        const sd = res.streaming_data;
                        const candidates = [...(sd.adaptive_formats || []), ...(sd.formats || [])];
                        if (candidates.length > 0) {
                            if (!fallbackInfo) fallbackInfo = res;
                            if (hasDirectOrDecipherableAudio(res)) {
                                info = res;
                                break;
                            }
                        }
                    }
                } catch (e) {
                    lastErr = e;
                }
            }

            if (!info) {
                info = fallbackInfo;
            }
        }

        if (!info) {
            try {
                const bi = await client.getBasicInfo(videoId);
                if (bi && bi.streaming_data) info = bi;
            } catch (_) {}
        }

        if (!info) {
            throw new Error(`Failed to retrieve video stream: ${lastErr?.message || 'Video unavailable'}`);
        }

        const bi = info.basic_info || {};
        const sd = info.streaming_data || {};
        const allCandidates = [...(sd.adaptive_formats || []), ...(sd.formats || [])];
        const audioCandidates = allCandidates.filter((f) => isAudioFormat(f));
        console.log(`[YouTubeResolver] Resolved info for ${videoId}: all formats=${allCandidates.length}, audio candidates=${audioCandidates.length}`);

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

        if (!fmt && audioCandidates.length > 0) {
            fmt =
                audioCandidates.find((f) => f.url && f.mime_type?.includes('mp4')) ||
                audioCandidates.find((f) => f.url) ||
                audioCandidates.find((f) => !f.has_video && f.mime_type?.includes('mp4')) ||
                audioCandidates.find((f) => !f.has_video) ||
                audioCandidates[0];
        }

        if (!fmt) {
            fmt = allCandidates.find((f) => f && f.has_audio) || allCandidates[0];
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
            for (const candidate of audioCandidates) {
                if (candidate) {
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