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
                    try {
                        const fn = new Function(...Object.keys(env || {}), data.output);
                        return fn(...Object.values(env || {}));
                    } catch (e) {
                        const msg = e?.message || String(e);
                        if (msg.includes('unsafe-eval') || msg.includes('CSP') || msg.includes('Refused to evaluate')) {
                            throw new Error(`YouTube signature decipher blocked by CSP: 'unsafe-eval' is required for youtube.js decipher (new Function). Current script-src lacks 'unsafe-eval' — add it to tauri.conf.json security.csp or fall back to TV/ANDROID_VR clients which need no decipher. Original: ${msg}`);
                        }
                        if (msg.includes('Function')) {
                            throw new Error(`YouTube decipher eval failed — signature may have changed or CSP blocked unsafe-eval. Try TV/ANDROID_VR fallback or update youtubei.js. Original: ${msg}`);
                        }
                        throw e;
                    }
                }
            });
        }

        const vd = opts.visitorData || opts.visitor_data || '';
        const key = [opts.cookie || '', opts.poToken || opts.po_token || '', vd].join('|');
        if (!this._clients[key]) {
            const cfg = {
                retrieve_player: false,
            };
            if (opts.cookie) cfg.cookie = opts.cookie;
            // Wire poToken + visitorData for PO (youtubei expects po_token / visitor_data, also accepts camelCase)
            if (opts.poToken || opts.po_token) {
                const tok = opts.poToken || opts.po_token;
                cfg.poToken = tok;
                cfg.po_token = tok;
            }
            if (vd) {
                cfg.visitorData = vd;
                cfg.visitor_data = vd;
            }

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
        if (m.includes('webm') || m.includes('opus')) return 'webm';
        if (m.includes('ogg')) return 'ogg';
        if (m.includes('wav')) return 'wav';
        if (m.includes('flac')) return 'flac';
        if (m.includes('mp4') || m.includes('m4a') || m.includes('aac')) {
            // video/mp4 progressive (itag 18 muxed) must stay mp4, not m4a
            if (m.startsWith('video/')) return 'mp4';
            return 'm4a';
        }
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

        const videoId = this.extractVideoId(url);

        // 2026 PO-token gate: mint po_token for ALL clients (TV/MWEB/WEB included — pot attached unconditionally)
        // Wrapped in dynamic import so resolver never becomes "unavailable" if bgutils missing.
        // Mint happens regardless of winningClient (before actions.execute) and even when caller passed TV preference
        // without token (opts.poToken empty) — visitorData-bound cache via generatePoTokenForVideo (nativeFetchPo for jnn-pa + interpreter_url).
        let client = null;
        if (!opts.poToken && !opts.po_token) {
            try {
                const poMod = await import('./modules/po_token.js').catch(() => import('./po_token.js')).catch(() => null);
                if (poMod) {
                    const { getCachedPoToken, setCachedPoToken, generatePoTokenForVideo } = poMod;
                    // visitorData-bound cache: use session visitorData if available
                    let vdForCache = opts.visitorData || opts.visitor_data || null;
                    try {
                        const tmpForVd = await this._client(opts);
                        vdForCache = tmpForVd?.session?.context?.client?.visitorData || vdForCache;
                    } catch (_) {}
                    const cached = getCachedPoToken ? getCachedPoToken(videoId, vdForCache) : null;
                    if (cached) {
                        opts = { ...opts, poToken: cached.poToken, po_token: cached.poToken, visitorData: cached.visitorData || vdForCache || opts.visitorData, visitor_data: cached.visitorData || vdForCache };
                        console.log(`[YouTubeResolver] Using cached PO token for ${videoId}`);
                    } else if (generatePoTokenForVideo) {
                        const tmpClient = await this._client(opts);
                        const minted = await generatePoTokenForVideo(tmpClient, videoId).catch(() => null);
                        if (minted?.poToken) {
                            opts = { ...opts, poToken: minted.poToken, po_token: minted.poToken, visitorData: minted.visitorData || vdForCache || opts.visitorData, visitor_data: minted.visitorData || vdForCache };
                            if (setCachedPoToken) setCachedPoToken(videoId, minted);
                            console.log(`[YouTubeResolver] Minted PO token for ${videoId}`);
                        } else {
                            console.warn(`[YouTubeResolver] No PO token minted for ${videoId} — will try TV/ANDROID_VR fallback`);
                        }
                    }
                }
            } catch (e) {
                console.warn('[YouTubeResolver] PO token import/mint skipped:', e?.message || e);
            }
        }
        try {
            client = await this._client(opts);
        } catch (e) {
            console.warn('[YouTubeResolver] _client init failed, falling back to TV client:', e?.message || e);
            // Ensure window.AuralisYouTube never becomes unavailable
            try { client = await this._client({}); } catch (_) { throw e; }
        }

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

        const isDecipherable = (f) => Boolean(f && (f.url || f.signature_cipher || f.cipher || typeof f.decipher === 'function'));
        const isAudioOnlyProgressive = (f) => Boolean(f && f.has_audio && !f.has_video);

        // Prefer audio-only progressive (e.g., itag 140 m4a) over muxed video+audio (itag 18) — avoids 360p remux waste
        const pickLegacyProgressive = (fmts) => {
            if (!fmts || !fmts.length) return null;
            const decipherable = fmts.filter(isDecipherable);
            if (!decipherable.length) return null;
            // Keep isAudioFormat filtering but add explicit check for legacy audio-only before video+audio
            const audioOnly = decipherable.filter((f) => isAudioFormat(f) && isAudioOnlyProgressive(f));
            if (audioOnly.length) return [...audioOnly].sort((a, b) => (b.bitrate || 0) - (a.bitrate || 0))[0];
            const audioMimeOnly = decipherable.filter((f) => isAudioFormat(f) && !f.has_video);
            if (audioMimeOnly.length) return [...audioMimeOnly].sort((a, b) => (b.bitrate || 0) - (a.bitrate || 0))[0];
            // No audio-only progressive: fallback to muxed (has_audio) — caller must set ext correctly and log muxed
            const muxed = decipherable.filter((f) => f.has_audio);
            if (muxed.length) return [...muxed].sort((a, b) => (b.bitrate || 0) - (a.bitrate || 0))[0];
            return decipherable[0];
        };

        const hasDirectOrDecipherableAudio = (r) => {
            if (!r || !r.streaming_data) return false;
            const sd = r.streaming_data;
            const all = [...(sd.adaptive_formats || []), ...(sd.formats || [])];
            return all.some((f) => isAudioFormat(f) && isDecipherable(f));
        };

        // F7 SABR-only fallback: 2026 WEB client often returns SABR-only (only
        // serverAbrStreamingUrl, adaptive_formats URLs missing) but legacy
        // progressive formats[18] (360p) remain usable. FreeTube#6977.
        const hasLegacyProgressiveFallback = (r) => {
            if (!r || !r.streaming_data) return false;
            const fmts = r.streaming_data.formats || [];
            if (!fmts.length) return false;
            // Explicit check: prefer audio-only progressive if available (e.g., itag 140 m4a audio) over video+audio 18
            if (fmts.some((f) => isAudioFormat(f) && isAudioOnlyProgressive(f) && isDecipherable(f))) return true;
            if (fmts.some((f) => isAudioFormat(f) && !f.has_video && isDecipherable(f))) return true;
            // Fallback: any decipherable progressive (muxed itag 18) — handles WEB mapping where has_audio may be false (video/mp4)
            return fmts.some(isDecipherable);
        };

        // 1. First attempt: Direct raw player API query.
        // 2026 PO-token enforcement: IOS/ANDROID require poToken for GVS (403 otherwise, empty body),
        // while TV / ANDROID_VR do not (see yt-dlp PO Token Guide). Prefer no-token clients when opts.poToken missing.
        const orderedClients = opts.poToken ? ['IOS','ANDROID','ANDROID_VR','TV','MWEB','WEB'] : ['TV','ANDROID_VR','MWEB','WEB','IOS','ANDROID'];
        // 2026 Jio sn-gwpa-cived gates TV too — caller may exclude TV on 403 retry (ANDROID+pot or WEB_SAFARI).
        // Keep const orderedClients for test regex; apply caller overrides via effective list.
        let effectiveOrderedClients = [...orderedClients];
        // Allow caller (downloads.js 403 auto-retry) to exclude a client or force rotation
        if (opts.excludeClient) {
            const ex = String(opts.excludeClient).toUpperCase();
            effectiveOrderedClients = effectiveOrderedClients.filter((c) => c.toUpperCase() !== ex);
        }
        if (Array.isArray(opts.excludeClients) && opts.excludeClients.length) {
            const set = new Set(opts.excludeClients.map((c) => String(c).toUpperCase()));
            effectiveOrderedClients = effectiveOrderedClients.filter((c) => !set.has(c.toUpperCase()));
        }
        if (opts.forceClient) {
            const fc = String(opts.forceClient).toUpperCase();
            if (effectiveOrderedClients.includes(fc)) effectiveOrderedClients = [fc, ...effectiveOrderedClients.filter((c) => c !== fc)];
            else effectiveOrderedClients = [fc];
        }
        // Support orderedClients override for deterministic retry (downloads.js passes remaining)
        if (Array.isArray(opts.orderedClients) && opts.orderedClients.length) {
            effectiveOrderedClients = [...opts.orderedClients];
        }
        if (client.actions?.execute) {
            for (const cl of effectiveOrderedClients) {
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
                        // SABR-only fallback: allow legacy progressive formats[18] even
                        // when adaptive_formats URLs are missing (FreeTube#6977).
                        // WEB 2026 often returns only serverAbrStreamingUrl for DASH,
                        // but formats still contains 360p progressive with url/cipher.
                        if (hasLegacyProgressiveFallback(parsed)) {
                            console.log(`[YouTubeResolver] actions.execute('${cl}') SABR-only fallback: using legacy progressive formats`);
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
            const clientNames = effectiveOrderedClients;
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
                            // SABR-only fallback for getInfo path too (FreeTube#6977)
                            if (hasLegacyProgressiveFallback(res)) {
                                console.log(`[YouTubeResolver] getInfo('${clientNames[i]}') SABR-only fallback: using legacy progressive formats`);
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
            // Prefer webm/opus (higher bitrate, more efficient) when available
            const attempts = container ? [container, null] : ['webm', 'mp4', null];
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
            // Prefer webm/opus (higher bitrate, more efficient) when available — sorted by bitrate desc
            const byBitrate = (a, b) => (b.bitrate || 0) - (a.bitrate || 0);
            const withUrl = audioCandidates.filter((f) => f.url);
            const webmOpusWithUrl = withUrl.filter((f) => f.mime_type?.includes('opus') || f.mime_type?.includes('webm')).sort(byBitrate);
            const sortedWithUrl = [...withUrl].sort(byBitrate);
            const withCipher = audioCandidates.filter((f) => f.signature_cipher || f.cipher || typeof f.decipher === 'function').sort(byBitrate);
            const webmOpusWithCipher = withCipher.filter((f) => f.mime_type?.includes('opus') || f.mime_type?.includes('webm'));
            const audioOnlyWithCipher = withCipher.filter((f) => !f.has_video);
            const sortedAll = [...audioCandidates].sort(byBitrate);
            fmt = webmOpusWithUrl[0] || sortedWithUrl[0] || webmOpusWithCipher[0] || audioOnlyWithCipher[0] || sortedAll[0];
        }

        if (!fmt) {
            fmt = allCandidates.find((f) => f && f.has_audio) || allCandidates[0];
        }

        // SABR-only final fallback: WEB 2026 may have adaptive_formats with no URLs
        // (only serverAbrStreamingUrl) but legacy progressive formats remain decipherable.
        // Prefer audio-only progressive (e.g., itag 140 m4a) over muxed video+audio (itag 18 360p, wasteful).
        if ((!fmt || (!fmt.url && !fmt.signature_cipher && !fmt.cipher && typeof fmt.decipher !== 'function')) && sd.formats && sd.formats.length) {
            const legacy = pickLegacyProgressive(sd.formats);
            if (legacy && isDecipherable(legacy)) {
                const isMuxed = Boolean(legacy.has_video);
                if (isMuxed) {
                    console.warn(`[YouTubeResolver] SABR-only final fallback: using MUXED progressive itag=${legacy.itag} mime=${legacy.mime_type} (video+audio 360p remux — wasteful, ext will be mp4)`);
                } else {
                    console.log(`[YouTubeResolver] SABR-only final fallback: using legacy progressive itag=${legacy.itag} mime=${legacy.mime_type}`);
                }
                fmt = legacy;
            }
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

        // SABR-only second fallback: if still no URL, try legacy progressive formats
        // directly. Prefer audio-only progressive over muxed video+audio (itag 18).
        if (!streamUrl && sd.formats && sd.formats.length) {
            const orderedLegacy = (() => {
                const best = pickLegacyProgressive(sd.formats);
                if (!best) return [];
                const rest = sd.formats.filter((f) => f !== best && isDecipherable(f));
                const score = (f) => {
                    if (isAudioFormat(f) && isAudioOnlyProgressive(f)) return 0;
                    if (isAudioFormat(f) && !f.has_video) return 1;
                    if (f.has_audio) return 2;
                    return 3;
                };
                rest.sort((a, b) => score(a) - score(b) || (b.bitrate || 0) - (a.bitrate || 0));
                return [best, ...rest];
            })();
            for (const candidate of orderedLegacy) {
                if (!candidate) continue;
                const isMuxed = Boolean(candidate.has_video);
                if (candidate.url) {
                    if (isMuxed) console.warn(`[YouTubeResolver] SABR-only streamUrl fallback: using MUXED progressive url itag=${candidate.itag} mime=${candidate.mime_type} (video+audio, ext mp4 — wasteful)`);
                    else console.log(`[YouTubeResolver] SABR-only streamUrl fallback: using legacy progressive url itag=${candidate.itag}`);
                    streamUrl = candidate.url;
                    fmt = candidate;
                    break;
                }
                if (typeof candidate.decipher === 'function' && client.session?.player) {
                    try {
                        const deciphered = await candidate.decipher(client.session.player);
                        if (deciphered) {
                            if (isMuxed) console.warn(`[YouTubeResolver] SABR-only streamUrl fallback: deciphered MUXED progressive itag=${candidate.itag} (muxed, ext mp4 — wasteful)`);
                            else console.log(`[YouTubeResolver] SABR-only streamUrl fallback: deciphered legacy progressive itag=${candidate.itag}`);
                            streamUrl = deciphered;
                            fmt = candidate;
                            break;
                        }
                    } catch (_) {}
                }
                // signature_cipher case is handled in the next block via fmt.cipher,
                // but we can also try to promote candidate to fmt for that block
                if (candidate.signature_cipher || candidate.cipher) {
                    if (isMuxed) console.warn(`[YouTubeResolver] SABR-only streamUrl fallback: promoting MUXED progressive cipher itag=${candidate.itag} (muxed, ext mp4 — wasteful)`);
                    else console.log(`[YouTubeResolver] SABR-only streamUrl fallback: promoting legacy progressive cipher itag=${candidate.itag}`);
                    fmt = candidate;
                    break;
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

        // Ensure pot is appended as &pot= on googlevideo URL via Player.decipher (youtubei.esm.mjs)
        // Player.decipher already appends pot when session.player.po_token is set; add defensive manual append
        // so Jio IPv6 residential (no datacenter proxy) succeeds even if Player version lags.
        if (streamUrl && (opts.poToken || opts.po_token)) {
            const potVal = opts.poToken || opts.po_token;
            // Player.decipher path already handled pot when po_token bound to session; verify via URL
            if (!streamUrl.includes('pot=')) {
                try {
                    const u = new URL(streamUrl);
                    // Only append for googlevideo hosts (avoid polluting other URLs)
                    if (u.hostname.includes('googlevideo.com') || u.hostname.includes('youtube.com')) {
                        u.searchParams.set('pot', potVal);
                        streamUrl = u.toString();
                        console.log(`[YouTubeResolver] Appended pot to googlevideo URL for ${videoId}`);
                    }
                } catch (_) {
                    if (!streamUrl.includes('pot=')) streamUrl += (streamUrl.includes('?') ? '&' : '?') + 'pot=' + encodeURIComponent(potVal);
                }
            }
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

        // Expose retry metadata so downloads.js/core.js can auto-retry on 403 with next orderedClient
        // Use effectiveOrderedClients (respects excludeClient/forceClient) for retry rotation
        const _ord = (typeof effectiveOrderedClients !== 'undefined' ? effectiveOrderedClients : orderedClients);
        const winIdx = _ord.indexOf(winningClient);
        const retryClients = winIdx >= 0 ? _ord.slice(winIdx + 1) : _ord.filter((c) => c !== winningClient);

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
            winningClient,
            orderedClients: [..._ord],
            retryClients,
            videoId,
            originalUrl: url,
            resolveOpts: { ...opts },
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