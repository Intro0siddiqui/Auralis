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
        if (Array.isArray(hdrs)) {
            for (const [k, v] of hdrs) {
                if (v !== undefined && v !== null) cleanHeaders[String(k)] = String(v);
            }
        } else if (typeof hdrs.forEach === 'function') {
            hdrs.forEach((v, k) => {
                if (v !== undefined && v !== null) cleanHeaders[String(k)] = String(v);
            });
        } else if (typeof hdrs === 'object') {
            for (const [k, v] of Object.entries(hdrs)) {
                if (v !== undefined && v !== null) cleanHeaders[String(k)] = String(v);
            }
        }
    };

    if (input?.headers) extractHeaders(input.headers);
    if (init?.headers) extractHeaders(init.headers);

    let body = null;
    if (init?.body !== undefined && init?.body !== null) {
        const rawBody = init.body;
        if (typeof rawBody === 'string') {
            body = rawBody;
        } else if (rawBody instanceof Uint8Array || rawBody instanceof ArrayBuffer) {
            body = new TextDecoder().decode(rawBody);
        } else if (typeof rawBody === 'object' && typeof rawBody.text === 'function') {
            try { body = await rawBody.text(); } catch (_) { body = String(rawBody); }
        } else if (typeof rawBody === 'object') {
            try { body = JSON.stringify(rawBody); } catch (_) { body = String(rawBody); }
        } else {
            body = String(rawBody);
        }
    } else if (input && typeof input.clone === 'function') {
        try {
            body = await input.clone().text();
        } catch (_) {}
    } else if (input?.body) {
        const rawBody = input.body;
        if (typeof rawBody === 'string') {
            body = rawBody;
        } else if (rawBody instanceof Uint8Array || rawBody instanceof ArrayBuffer) {
            body = new TextDecoder().decode(rawBody);
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
        let winningClient = null;
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

        // 1. First attempt: Direct raw player API query.
        // 2026 PO-token enforcement: IOS/ANDROID require poToken for GVS (403 otherwise, empty body),
        // while TV / ANDROID_VR do not (see yt-dlp PO Token Guide). Prefer no-token clients when opts.poToken missing.
        const orderedClients = opts.poToken ? ['IOS','ANDROID','ANDROID_VR','TV','MWEB','WEB'] : ['TV','ANDROID_VR','MWEB','WEB','IOS','ANDROID'];
        if (client.actions?.execute) {
            for (const cl of orderedClients) {
                try {
                    const raw = await client.actions.execute('/player', { videoId, client: cl });
                    const st = raw?.data?.playabilityStatus?.status;
                    const sd = raw?.data?.streamingData;
                    const totalFormats = (sd?.adaptiveFormats?.length || 0) + (sd?.formats?.length || 0);
                    console.log(`[YouTubeResolver] actions.execute('${cl}') -> status: ${st}, formats: ${totalFormats}`);
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
                        const vd = raw?.data?.videoDetails || {};
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
                            winningClient = cl;
                            break;
                        }
                    }
                } catch (e) {
                    console.error(`[YouTubeResolver] actions.execute('${cl}') error:`, e.message);
                    lastErr = e;
                }
            }
        }

        // 2. Second attempt: High-level Innertube getInfo fallback (same PO-token-aware order)
        if (!info) {
            const clientNames = orderedClients;
            const clientAttempts = clientNames.map((cl) => async () => client.getInfo(videoId, { client: cl }));

            let fallbackInfo = null;
            let fallbackClient = null;
            for (let i = 0; i < clientAttempts.length; i++) {
                const attempt = clientAttempts[i];
                try {
                    const res = await attempt();
                    if (res && res.streaming_data) {
                        const sd = res.streaming_data;
                        const candidates = [...(sd.adaptive_formats || []), ...(sd.formats || [])];
                        if (candidates.length > 0) {
                            if (!fallbackInfo) { fallbackInfo = res; fallbackClient = clientNames[i]; }
                            if (hasDirectOrDecipherableAudio(res)) {
                                info = res;
                                winningClient = clientNames[i];
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
                if (!winningClient) winningClient = fallbackClient;
            }
        }

        if (!info) {
            try {
                const bi = await client.getBasicInfo(videoId);
                if (bi && bi.streaming_data) info = bi;
            } catch (e) {
                console.warn('[YouTubeResolver] getBasicInfo fallback failed:', e?.message || e);
            }
        }

        if (!info) {
            const msg = `Failed to retrieve video stream: ${lastErr?.message || 'Video unavailable'} (videoId=${videoId}, tried 6 InnerTube clients; last status was checked via actions.execute/getInfo — check log above for per-client status, and ensure device has network + valid YouTube cookie/PO token if age-restricted)`;
            console.error(`DIAGNOSTIC youtube_resolve_failed videoId=${videoId} error=${msg} lastErr=${lastErr?.message || lastErr}`);
            console.error(lastErr);
            throw new Error(msg);
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

        if (!fmt) {
            const diag = `No audio stream found for ${videoId}: all=${allCandidates.length} audioCandidates=${audioCandidates.length} streaming_data keys=${Object.keys(sd||{}).join(',')} (winningClient=${winningClient || 'none'}). This usually means YouTube returned no adaptive_formats — video may be private/age-restricted/region-blocked or Innertube throttling LOGIN_REQUIRED. Try another client or set youtube_cookie/po_token in Settings.`;
            console.error(`DIAGNOSTIC no_audio_stream ${diag}`);
            throw new Error(diag);
        }

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

        if (!streamUrl && (fmt.signature_cipher || fmt.cipher)) {
            // actions.execute path produced a plain object with cipher but no decipher()
            // method — decode and ask the player to decipher `s`/`n`.
            const cipherStr = fmt.signature_cipher || fmt.cipher;
            try {
                const params = new URLSearchParams(cipherStr);
                let url = params.get('url');
                const s = params.get('s');
                const sp = params.get('sp') || 'sig';
                const n = params.get('n') ? null : null; // n lives in url, handled below
                if (url) {
                    url = decodeURIComponent(url);
                    if (s && client.session?.player) {
                        // Try youtubei's n/s decipher via player
                        let deciphered = s;
                        if (typeof client.session.player.decipher === 'function') {
                            try { deciphered = await client.session.player.decipher(s); } catch (_) {}
                        } else if (typeof client.session.player.ncode === 'function') {
                            try { deciphered = client.session.player.ncode(s); } catch (_) {}
                        }
                        const u = new URL(url);
                        u.searchParams.set(sp, deciphered);
                        streamUrl = u.toString();
                    } else if (url) {
                        streamUrl = url;
                    }
                }
            } catch (e) {
                console.warn('[YouTubeResolver] cipher decode failed:', e?.message || e);
            }
        }

        // n-parameter throttling: if URL contains &n=..., ask player to decipher n
        if (streamUrl && streamUrl.includes('&n=') && client.session?.player) {
            try {
                const u = new URL(streamUrl);
                const nVal = u.searchParams.get('n');
                if (nVal) {
                    let nDec = nVal;
                    const p = client.session.player;
                    if (typeof p.decipher === 'function') {
                        // Some players expose decipher for n as well
                        try { nDec = await p.decipher(nVal); } catch (_) {}
                    }
                    if (typeof p.ncode === 'function') {
                        try { nDec = p.ncode(nVal); } catch (_) {}
                    }
                    // youtubei's player often has `n` transform on `player.n`
                    if (nDec !== nVal) {
                        u.searchParams.set('n', nDec);
                        streamUrl = u.toString();
                    }
                }
            } catch (_) {}
        }

        if (!streamUrl) {
            const diag = `Unable to extract playable audio URL for ${videoId}: fmt keys=${fmt ? Object.keys(fmt).join(',') : 'no fmt'} url=${fmt?.url?'has url':''} cipher=${fmt?.signature_cipher||fmt?.cipher?'has cipher':''} decipher=${typeof fmt?.decipher} (winningClient=${winningClient}) — check that headers/UA match and n/s decipher succeeded; see logs above.`;
            console.error(`DIAGNOSTIC no_stream_url ${diag}`);
            throw new Error(diag);
        }

        const ext = this.extFromMime(fmt.mime_type);
        const title = String(bi.title || 'YouTube Audio').trim() || 'YouTube Audio';
        const thumb = this.pickThumb(bi.thumbnail);
        const totalRaw = fmt.content_length ? Number(fmt.content_length) : NaN;
        const total = isNaN(totalRaw) ? null : totalRaw;

        // Build headers matched to the InnerTube client that produced the URL
        // — googlevideo validates UA/Referer/Origin against the client context.
        const uaMap = {
            'IOS': 'Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Safari/604.1',
            'ANDROID': 'com.google.android.youtube/20.10.38 (Linux; U; Android 14; en_US; Pixel 8 Build/UD1A.230803.041)',
            'ANDROID_VR': 'com.google.android.apps.youtube.vr/1.56.42 (Linux; U; Android 14; en_US; Pixel 8 Build/UD1A.230803.041)',
            'TV': 'Mozilla/5.0 (ChromiumStylePlatform) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36',
            'MWEB': 'Mozilla/5.0 (Linux; Android 14; Mobile) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Mobile Safari/537.36',
            'WEB': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36',
        };
        const headers = {
            'User-Agent': uaMap[winningClient] || uaMap['ANDROID'],
            'Referer': 'https://www.youtube.com/',
            'Origin': 'https://www.youtube.com',
            'Accept': '*/*',
            'Accept-Language': 'en-US,en;q=0.9',
        };

        return {
            kind: 'track',
            stream_url: streamUrl,
            title,
            ext,
            total_bytes: total,
            thumbnail: thumb,
            platform: 'youtube',
            headers,
            client: winningClient,
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