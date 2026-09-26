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

/**
 * Format preference scoring — prefers itag 140 (m4a) which rodio can decode.
 * rodio 0.22.2 has no opus feature: webm/opus would cause DecodeError, so
 * we do NOT prefer webm/opus. Only m4a/mp4 audio is prioritized.
 */
function scoreFormat(fmt) {
    const mime = (fmt.mimeType || fmt.mime_type || '').toLowerCase();
    // itag 140 = standard M4A 128kbps, usually FastStart from YouTube CDN
    if (fmt.itag === 140) return 3;
    if (mime.includes('mp4') && !mime.includes('video')) return 2;
    if (mime.includes('m4a')) return 2;
    return 1;
}

/**
 * Compact one-line-per-client summary of how each InnerTube client reacted.
 *
 * Release builds emit nothing to logcat, so this string is the only
 * device-visible evidence of what each client returned — it is embedded in
 * resolve failure messages, rendered in the download row and copied by the
 * "Copy report" action.
 *
 * Example: `MWEB ok/audio-url a3/3 aurl 2 sabr0 431ms* | ANDROID 403 55ms | TV sabr-only a6/0 aurl 0 sabr1`
 * (`*` marks the client whose formats were actually used)
 */
function formatClientReport(report) {
    if (!Array.isArray(report) || !report.length) return 'no client attempts recorded';
    return report
        .map((e) => {
            const mark = e.chosen ? '*' : ' ';
            const head = e.chosen ? 'ok' : e.status || e.reason || 'fail';
            const nums = `a${e.adaptive ?? 0}/p${e.progressive ?? 0} aurl${e.adaptiveWithUrl ?? 0} audio${e.audioWithUrl ?? 0}${e.sabrStreamingUrl ? ' sabr1' : ''}`;
            const extra = [];
            if (e.legacyProgressive) extra.push('legacy');
            if (e.ms) extra.push(`${e.ms}ms`);
            if (e.error) extra.push(String(e.error).slice(0, 60));
            return `${mark}${e.client} ${head} ${nums}${extra.length ? ' ' + extra.join(' ') : ''}`;
        })
        .join(' | ');
}

class YouTubeResolver {
    constructor() {
        this._modulePromise = null;
        this._clients = {};
    }

    /** Exposed so the downloads UI can render/copy the same report. */
    static formatClientReport(report) {
        return formatClientReport(report);
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
        // True when the token in play is one WE minted (or read from our own
        // cache). Those come from WebPoMinter, i.e. a BotGuard/WEB token, and a PO
        // token is platform-bound: only web-family clients honour it. A token the
        // user typed into Settings is left alone — that one may legitimately be an
        // iOS/Android token, so we must not assume anything about it.
        let tokenIsWebBound = false;
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
                        tokenIsWebBound = true;
                        console.log(`[YouTubeResolver] Using cached PO token for ${videoId}`);
                    } else if (generatePoTokenForVideo) {
                        const tmpClient = await this._client(opts);
                        const minted = await generatePoTokenForVideo(tmpClient, videoId).catch(() => null);
                        if (minted?.poToken) {
                            opts = { ...opts, poToken: minted.poToken, po_token: minted.poToken, visitorData: minted.visitorData || vdForCache || opts.visitorData, visitor_data: minted.visitorData || vdForCache };
                            if (setCachedPoToken) setCachedPoToken(videoId, minted);
                            tokenIsWebBound = true;
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
        // Per-client reaction log. Every attempted InnerTube client records what
        // it answered with (playability status, format counts, whether adaptive
        // formats carried real urls, whether a SABR `serverAbrStreamingUrl` was
        // present, and how long it took). Release builds write nothing to
        // logcat, so this is the only way to see how SABR/403 gating behaves
        // per client on the user's own network — it is surfaced in the download
        // row and copied by the "Copy report" action.
        const clientReport = [];

        const isAudioFormat = (f) => {
            if (!f) return false;
            if (f.has_audio && !f.has_video) return true;
            if (typeof f.mime_type === 'string' && f.mime_type.startsWith('audio/')) return true;
            if (f.has_audio) return true;
            return false;
        };

        const isDecipherable = (f) => Boolean(f && (f.url || f.signature_cipher || f.cipher || typeof f.decipher === 'function'));
        const isAudioOnlyProgressive = (f) => Boolean(f && f.has_audio && !f.has_video);

        const hasValidAudioContainer = (f) => {
            if (!f || !f.mime_type) return false;
            const mime = String(f.mime_type).toLowerCase();
            return (
                mime.startsWith('audio/mp4') ||
                mime.startsWith('audio/webm') ||
                mime.startsWith('audio/m4a') ||
                mime.startsWith('audio/ogg') ||
                mime.startsWith('audio/opus') ||
                mime.startsWith('video/mp4') ||
                mime.startsWith('video/webm')
            );
        };

        const hasValidAudioCodec = (f) => {
            if (!f) return false;
            const mime = String(f.mime_type || '').toLowerCase();
            return (
                mime.includes('opus') ||
                mime.includes('mp4a') ||
                mime.includes('aac') ||
                mime.includes('vorbis') ||
                mime.includes('flac') ||
                mime.startsWith('audio/mp4') ||
                mime.startsWith('audio/webm') ||
                mime.startsWith('audio/ogg')
            );
        };

        const selectBestAudioFormat = (candidates, quality = 'best', targetContainer = null) => {
            if (!candidates || !candidates.length) return null;
            const valid = candidates.filter((f) => f && isDecipherable(f));
            if (!valid.length) return candidates[0] || null;

            // Use global scoreFormat (itag 140 > mp4/m4a > other) — rodio 0.22.2 lacks opus, so never prefer webm/opus by default
            const scoreFn = (f) => {
                let base = scoreFormat(f);
                const mimeL = String(f.mime_type || f.mimeType || '').toLowerCase();
                if (targetContainer === 'mp4' && (mimeL.includes('mp4') || mimeL.includes('m4a'))) {
                    base += 0.5;
                } else if (targetContainer === 'webm' && (mimeL.includes('webm') || mimeL.includes('opus'))) {
                    base += 0.5;
                }
                return base;
            };

            const sorted = [...valid].sort((a, b) => {
                const diffScore = scoreFn(b) - scoreFn(a);
                if (diffScore !== 0) return diffScore;
                // Secondary tie-breakers: audio-only, direct URL, content-length, bitrate
                const aAudioOnly = (isAudioOnlyProgressive(a) || !a.has_video) ? 1 : 0;
                const bAudioOnly = (isAudioOnlyProgressive(b) || !b.has_video) ? 1 : 0;
                if (bAudioOnly !== aAudioOnly) return bAudioOnly - aAudioOnly;
                const aHasUrl = a.url ? 1 : 0;
                const bHasUrl = b.url ? 1 : 0;
                if (bHasUrl !== aHasUrl) return bHasUrl - aHasUrl;
                const aHasLen = ((a.content_length && a.content_length > 0) || (a.contentLength && parseInt(a.contentLength, 10) > 0)) ? 1 : 0;
                const bHasLen = ((b.content_length && b.content_length > 0) || (b.contentLength && parseInt(b.contentLength, 10) > 0)) ? 1 : 0;
                if (bHasLen !== aHasLen) return bHasLen - aHasLen;
                return (b.bitrate || 0) - (a.bitrate || 0);
            });

            return sorted[0] || valid[0];
        };

        const pickAudioFormat = selectBestAudioFormat;

        // Prefer audio-only progressive (e.g., itag 140 m4a) over muxed video+audio (itag 18) — avoids 360p remux waste
        const pickLegacyProgressive = (fmts) => {
            if (!fmts || !fmts.length) return null;
            // Ensure SABR fallback only selects formats with valid audio containers (audio/mp4, audio/webm)
            const valid = fmts.filter((f) => isDecipherable(f) && hasValidAudioContainer(f));
            if (!valid.length) return null;
            // Keep isAudioFormat filtering but add explicit check for legacy audio-only before video+audio
            const audioOnly = valid.filter((f) => isAudioFormat(f) && isAudioOnlyProgressive(f));
            if (audioOnly.length) return [...audioOnly].sort((a, b) => (b.bitrate || 0) - (a.bitrate || 0))[0];
            const audioMimeOnly = valid.filter((f) => isAudioFormat(f) && !f.has_video);
            if (audioMimeOnly.length) return [...audioMimeOnly].sort((a, b) => (b.bitrate || 0) - (a.bitrate || 0))[0];
            // No audio-only progressive: fallback to muxed (has_audio) — caller must set ext correctly and log muxed
            const muxed = valid.filter((f) => f.has_audio);
            if (muxed.length) return [...muxed].sort((a, b) => (b.bitrate || 0) - (a.bitrate || 0))[0];
            return valid[0];
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
            // Ensure SABR fallback only selects formats with valid audio containers (audio/mp4, audio/webm)
            const valid = fmts.filter((f) => isDecipherable(f) && hasValidAudioContainer(f));
            if (!valid.length) return false;
            // Explicit check: prefer audio-only progressive if available (e.g., itag 140 m4a audio) over video+audio 18
            if (valid.some((f) => isAudioFormat(f) && isAudioOnlyProgressive(f))) return true;
            if (valid.some((f) => isAudioFormat(f) && !f.has_video)) return true;
            // Fallback: any decipherable progressive (muxed itag 18) — handles WEB mapping where has_audio may be false (video/mp4)
            return valid.some(isDecipherable);
        };

        // 1. First attempt: Direct raw player API query.
        //
        // Clients fall into three groups, and the order is those groups:
        //
        //   SERVABLE  — web-family. These are the only clients a BotGuard/WEB
        //               token is valid on, so while we hold one they lead.
        //   TOKEN_FREE — need no PO token at all. They lead when we hold none.
        //   UNMINTABLE — need a DroidGuard/iOSGuard token we cannot produce, so
        //               they are tried last. Kept, not deleted: `ios` does
        //               resolve and does return real audio urls, so this is a
        //               prediction of GVS failure rather than an observation of
        //               one, and the client report will settle it.
        //
        // `tv` is in TOKEN_FREE but must not lead: the guide's cell reads "All
        // formats DRM'd if cookies (logged-in or active guest) aren't passed",
        // and we send no account cookies, whereas `android_vr` carries no such
        // caveat and was the client a device report showed returning real audio
        // (22 adaptive / 4 with urls). So ANDROID_VR is the better first bet.
        //
        // `web_safari` is here because it replaced `tv_simply` in yt-dlp's
        // defaults and IS web-family, so our token serves it — and it is the one
        // web client we had never actually tried, on a network where mweb and
        // web both came back UNPLAYABLE. An experiment, not a claim.
        const SERVABLE_CLIENTS = ['MWEB', 'WEB', 'WEB_SAFARI'];
        const TOKEN_FREE_CLIENTS = ['ANDROID_VR', 'TV'];
        const UNMINTABLE_CLIENTS = ['IOS', 'ANDROID'];
        const orderedClients = opts.poToken
            ? [...SERVABLE_CLIENTS, ...TOKEN_FREE_CLIENTS, ...UNMINTABLE_CLIENTS]
            : [...TOKEN_FREE_CLIENTS, ...SERVABLE_CLIENTS, ...UNMINTABLE_CLIENTS];
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
        // A client that can only offer the legacy muxed progressive (itag 18) is a
        // last resort: the muxed 360p rendition is what the SABR window truncates,
        // and the bytes it reports as `clen` are the whole window. Because all
        // clients are raced with `Promise.any`, simply answering *first* used to
        // win - so a SABR-only client beat a slower client that had a real
        // audio-only url. Hold such a result back briefly to give the genuine
        // audio urls a head start, and fall back to it if none arrives.
        const LEGACY_RESULT_DELAY_MS = 1200;
        // Support orderedClients override for deterministic retry (downloads.js passes remaining)
        if (Array.isArray(opts.orderedClients) && opts.orderedClients.length) {
            effectiveOrderedClients = [...opts.orderedClients];
        }
        if (client.actions?.execute && effectiveOrderedClients.length > 0) {
            try {
                const executeClient = async (cl) => {
                    const t0 = Date.now();
                    const entry = {
                        client: cl,
                        phase: 'actions.execute',
                        chosen: false,
                        ok: false,
                        legacyProgressive: false,
                        status: null,
                        reason: null,
                        adaptive: 0,
                        progressive: 0,
                        adaptiveWithUrl: 0,
                        audioWithUrl: 0,
                        sabrStreamingUrl: false,
                        ms: 0,
                    };
                    clientReport.push(entry);
                    try {
                        const raw = await client.actions.execute('/player', { videoId, client: cl });
                        const st = raw?.data?.playabilityStatus?.status;
                        const sd = raw?.data?.streamingData;
                        const totalFormats = (sd?.adaptiveFormats?.length || 0) + (sd?.formats?.length || 0);
                        const rawAdapt = sd?.adaptiveFormats || [];
                        const urlOf = (f) => Boolean(f && (f.url || f.signatureCipher || f.cipher));
                        entry.status = st ?? null;
                        entry.adaptive = rawAdapt.length;
                        entry.progressive = (sd?.formats?.length || 0);
                        entry.adaptiveWithUrl = rawAdapt.filter(urlOf).length;
                        entry.audioWithUrl = rawAdapt.filter((f) => String(f.mimeType || '').startsWith('audio/') && urlOf(f)).length;
                        entry.sabrStreamingUrl = Boolean(sd?.serverAbrStreamingUrl);
                        // A client that answered but exposed only SABR metadata is
                        // materially different from one that returned real CDN urls.
                        if (entry.sabrStreamingUrl && entry.audioWithUrl === 0) {
                            entry.reason = 'sabr-only';
                        } else if (entry.adaptive > 0 && entry.adaptiveWithUrl === 0) {
                            entry.reason = 'adaptive-urls-missing';
                        }
                        console.log(`[YouTubeResolver] actions.execute('${cl}') -> status: ${st}, formats: ${totalFormats}, adaptiveWithUrl: ${entry.adaptiveWithUrl}, audioWithUrl: ${entry.audioWithUrl}, sabr: ${entry.sabrStreamingUrl}`);
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
                                entry.ok = true;
                                entry.reason = entry.reason || 'audio-url';
                                return { info: parsed, winningClient: cl, report: entry };
                            }
                            // SABR-only fallback: allow legacy progressive formats[18] even
                            // when adaptive_formats URLs are missing (FreeTube#6977).
                            // WEB 2026 often returns only serverAbrStreamingUrl for DASH,
                            // but formats still contains 360p progressive with url/cipher.
                            if (hasLegacyProgressiveFallback(parsed)) {
                                entry.ok = true;
                                entry.legacyProgressive = true;
                                entry.reason = 'legacy-progressive';
                                console.log(`[YouTubeResolver] actions.execute('${cl}') SABR-only fallback: using legacy progressive formats`);
                                await new Promise((resolve) => setTimeout(resolve, LEGACY_RESULT_DELAY_MS));
                                return { info: parsed, winningClient: cl, report: entry };
                            }
                            entry.reason = entry.reason || 'no-usable-audio';
                        }
                    } catch (e) {
                        entry.error = e?.message || String(e);
                        entry.reason = entry.reason || 'error';
                        console.error(`[YouTubeResolver] actions.execute('${cl}') error:`, e.message);
                        lastErr = e;
                    } finally {
                        entry.ms = Date.now() - t0;
                    }
                    throw new Error(`Client '${cl}' produced no usable stream`);
                };

                const res = await Promise.any(effectiveOrderedClients.map((cl) => executeClient(cl)));
                info = res.info;
                winningClient = res.winningClient;
                if (res.report) res.report.chosen = true;
            } catch (e) {
                lastErr = e;
            }
        }

        // 2. Second attempt: High-level Innertube getInfo fallback (same PO-token-aware order)
        if (!info && effectiveOrderedClients.length > 0) {
            const clientNames = effectiveOrderedClients;
            try {
                const getInfoClient = async (cl) => {
                    const t0 = Date.now();
                    const entry = {
                        client: cl,
                        phase: 'getInfo',
                        chosen: false,
                        ok: false,
                        legacyProgressive: false,
                        status: null,
                        reason: null,
                        adaptive: 0,
                        progressive: 0,
                        adaptiveWithUrl: 0,
                        audioWithUrl: 0,
                        sabrStreamingUrl: false,
                        ms: 0,
                    };
                    clientReport.push(entry);
                    try {
                        const res = await client.getInfo(videoId, { client: cl });
                        if (res && res.streaming_data) {
                            const sd = res.streaming_data;
                            const adapt = sd.adaptive_formats || [];
                            const candidates = [...adapt, ...(sd.formats || [])];
                            const urlOf = (f) => Boolean(f && (f.url || f.signature_cipher || f.cipher));
                            entry.adaptive = adapt.length;
                            entry.progressive = (sd.formats || []).length;
                            entry.adaptiveWithUrl = adapt.filter(urlOf).length;
                            entry.audioWithUrl = adapt.filter((f) => isAudioFormat(f) && urlOf(f)).length;
                            if (candidates.length > 0) {
                                if (hasDirectOrDecipherableAudio(res)) {
                                    entry.ok = true;
                                    entry.reason = 'audio-url';
                                    return { info: res, winningClient: cl, report: entry };
                                }
                                // SABR-only fallback for getInfo path too (FreeTube#6977)
                                if (hasLegacyProgressiveFallback(res)) {
                                    entry.ok = true;
                                    entry.legacyProgressive = true;
                                    entry.reason = 'legacy-progressive';
                                    console.log(`[YouTubeResolver] getInfo('${cl}') SABR-only fallback: using legacy progressive formats`);
                                    await new Promise((resolve) => setTimeout(resolve, LEGACY_RESULT_DELAY_MS));
                                    return { info: res, winningClient: cl, report: entry };
                                }
                                entry.reason = 'no-usable-audio';
                            } else {
                                entry.reason = 'no-formats';
                            }
                        } else {
                            entry.reason = 'no-streaming-data';
                        }
                    } catch (e) {
                        entry.error = e?.message || String(e);
                        entry.reason = entry.reason || 'error';
                        lastErr = e;
                    } finally {
                        entry.ms = Date.now() - t0;
                    }
                    throw new Error(`getInfo('${cl}') produced no usable stream`);
                };

                const res = await Promise.any(clientNames.map((cl) => getInfoClient(cl)));
                info = res.info;
                winningClient = res.winningClient;
                if (res.report) res.report.chosen = true;
            } catch (e) {
                lastErr = e;
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
            const msg = `Failed to retrieve video stream: ${lastErr?.message || 'Video unavailable'} (videoId=${videoId}, tried 6 InnerTube clients; last status was checked via actions.execute/getInfo — client report: ${formatClientReport(clientReport)}; ensure device has network + valid YouTube cookie/PO token if age-restricted)`;
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
        // True when we had to fall back to the SABR-only legacy progressive
        // (muxed 360p itag 18). Those URLs are served by the SABR endpoint,
        // which routinely returns a *partial* media resource whose container
        // header still advertises the full track length — the download then
        // completes at 100% of the advertised bytes while holding only part of
        // the audio. Callers retry truncated downloads with
        // `avoidLegacyProgressive` so a different client must produce a real
        // audio-only stream.
        let used_legacy_progressive = false;
        // A retry that already got a short legacy stream refuses the fallback
        // entirely: better to fail loudly than to re-download the same 99s.
        // Refuse a legacy-progressive fallback ONLY when this attempt is the
        // same client that already produced a short stream. The truncation we
        // are recovering from is a SABR *window* — a property of the response,
        // not of the muxed container — so the same itag 18 served by a
        // different client can be complete. Refusing the format outright
        // (the old behaviour) meant that once any attempt truncated, no client
        // could ever deliver a file, which is strictly worse for the user than a
        // complete 360p track that plays to the end.
        const truncated_client = String(opts.truncatedClient || opts.truncated_client || '').toUpperCase();
        const refuse_legacy_for_this_client =
            Boolean(opts.avoidLegacyProgressive) && (!truncated_client || truncated_client === String(winningClient || '').toUpperCase());
        const allow_legacy_progressive = !refuse_legacy_for_this_client;

        if (typeof info.chooseFormat === 'function') {
            // rodio 0.22.2 lacks opus — prefer m4a/mp4 (itag 140) over webm/opus to avoid DecodeError
            const attempts = container ? [container, null] : ['mp4', null, 'webm'];
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
            fmt = selectBestAudioFormat(audioCandidates, quality, container);
        }

        if (!fmt) {
            const validAll = allCandidates.filter((f) => isAudioFormat(f) || f.has_audio);
            fmt = selectBestAudioFormat(validAll, quality, container) || allCandidates[0];
        }

        // SABR-only final fallback: WEB 2026 may have adaptive_formats with no URLs
        // (only serverAbrStreamingUrl) but legacy progressive formats remain decipherable.
        // Prefer audio-only progressive (e.g., itag 140 m4a) over muxed video+audio (itag 18 360p, wasteful).
        if ((!fmt || (!fmt.url && !fmt.signature_cipher && !fmt.cipher && typeof fmt.decipher !== 'function')) && sd.formats && sd.formats.length) {
            const legacy = pickLegacyProgressive(sd.formats);
            if (legacy && isDecipherable(legacy)) {
                if (!allow_legacy_progressive) {
                    const diag = `Refusing SABR-only legacy progressive (itag=${legacy.itag}) for ${videoId}: a previous attempt already produced a short stream from it. Retry with another Innertube client or set youtube_po_token/cookie in Settings. Client report: ${formatClientReport(clientReport)}`;
                    console.error(`DIAGNOSTIC legacy_progressive_refused videoId=${videoId} itag=${legacy.itag}`);
                    const err = new Error(diag);
                    err.client_report = clientReport;
                    err.client_report_text = formatClientReport(clientReport);
                    throw err;
                }
                const isMuxed = Boolean(legacy.has_video);
                used_legacy_progressive = true;
                if (isMuxed) {
                    console.warn(`[YouTubeResolver] SABR-only final fallback: using MUXED progressive itag=${legacy.itag} mime=${legacy.mime_type} (video+audio 360p remux — wasteful, ext will be mp4). SABR streams are often PARTIAL: the file can finish at 100% of the advertised bytes while holding only part of the audio.`);
                } else {
                    console.log(`[YouTubeResolver] SABR-only final fallback: using legacy progressive itag=${legacy.itag} mime=${legacy.mime_type}`);
                }
                fmt = legacy;
            }
        }

        if (!fmt) {
            const diag = `No audio stream found for ${videoId}: all=${allCandidates.length} audioCandidates=${audioCandidates.length} streaming_data keys=${Object.keys(sd||{}).join(',')} (winningClient=${winningClient || 'none'}). This usually means YouTube returned no adaptive_formats — video may be private/age-restricted/region-blocked or Innertube throttling LOGIN_REQUIRED. Try another client or set youtube_cookie/po_token in Settings. Client report: ${formatClientReport(clientReport)}`;
            console.error(`DIAGNOSTIC no_audio_stream ${diag}`);
            const err = new Error(diag);
            err.client_report = clientReport;
            err.client_report_text = formatClientReport(clientReport);
            throw err;
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
                const rest = sd.formats.filter((f) => f !== best && isDecipherable(f) && hasValidAudioContainer(f));
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
                if (!allow_legacy_progressive) {
                    const diag = `Refusing SABR-only legacy progressive (itag=${candidate.itag}) for ${videoId}: a previous attempt already produced a short stream from it. Retry with another Innertube client or set youtube_po_token/cookie in Settings.`;
                    console.error(`DIAGNOSTIC legacy_progressive_refused videoId=${videoId} itag=${candidate.itag}`);
                    throw new Error(diag);
                }
                used_legacy_progressive = true;
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

        // A PO token is *platform-bound* — see modules/pot_scope.js. Attaching
        // the BotGuard/WEB token we mint to an `ios`/`android`/`android_vr` URL
        // makes googlevideo answer 403 at byte 0, because the client-matched UA
        // (uaMap below) is then sent against a foreign token. The decision lives
        // in a dependency-free module so it is unit-tested, not just asserted.
        {
            const { applyPoTokenToUrl } = await import('./modules/pot_scope.js');
            const applied = applyPoTokenToUrl(streamUrl, {
                winningClient: winningClient,
                token: opts.poToken || opts.po_token || null,
                tokenIsWebBound: tokenIsWebBound,
            });
            streamUrl = applied.url;
            if (applied.action === 'attached') {
                console.log(`[YouTubeResolver] Appended pot to ${winningClient} googlevideo URL for ${videoId}`);
            } else if (applied.action === 'stripped') {
                console.warn(`[YouTubeResolver] Stripped pot from a ${winningClient} URL for ${videoId}: ${applied.detail}`);
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
        const totalRaw = (fmt.content_length ?? fmt.contentLength) ? Number(fmt.content_length || fmt.contentLength) : NaN;
        let total = isNaN(totalRaw) ? null : totalRaw;
        if (!total && streamUrl) {
            try {
                const u = new URL(streamUrl);
                const clen = u.searchParams.get('clen');
                if (clen && parseInt(clen, 10) > 0) total = parseInt(clen, 10);
            } catch (_) {}
        }
        let durationSecs = Number(bi.duration || bi.lengthSeconds || 0);
        if (!durationSecs && fmt?.approx_duration_ms) {
            durationSecs = Math.round(fmt.approx_duration_ms / 1000);
        }
        if (!durationSecs && streamUrl) {
            try {
                const u = new URL(streamUrl);
                const dur = u.searchParams.get('dur');
                if (dur && parseFloat(dur) > 0) durationSecs = Math.round(parseFloat(dur));
            } catch (_) {}
        }

        // Build headers matched to the InnerTube client that produced the URL
        // — googlevideo validates UA/Referer/Origin against the client context.
        const uaMap = {
            'IOS': 'Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Safari/604.1',
            'ANDROID': 'com.google.android.youtube/20.10.38 (Linux; U; Android 14; en_US; Pixel 8 Build/UD1A.230803.041)',
            'ANDROID_VR': 'com.google.android.apps.youtube.vr/1.56.42 (Linux; U; Android 14; en_US; Pixel 8 Build/UD1A.230803.041)',
            'TV': 'Mozilla/5.0 (ChromiumStylePlatform) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36',
            'MWEB': 'Mozilla/5.0 (Linux; Android 14; Mobile) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Mobile Safari/537.36',
            'WEB': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36',
            // MUST exist. `uaMap[winningClient] || uaMap['ANDROID']` would otherwise
            // put an Android app UA on a web_safari URL — the same UA/token
            // mismatch class that produced the byte-0 403 fixed in v2.6.50.
            'WEB_SAFARI': 'Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Safari/604.1',
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
            duration: durationSecs,
            duration_secs: durationSecs,
            expected_duration_secs: durationSecs,
            thumbnail: thumb,
            platform: 'youtube',
            headers,
            client: winningClient,
            winningClient,
            orderedClients: [..._ord],
            retryClients,
            // True when the URL came from the SABR-only legacy progressive
            // path — such streams are routinely partial even though the
            // container header advertises the full length.
            sabrFallback: used_legacy_progressive,
            // What we actually picked, and how every client reacted.
            selection: {
                itag: fmt?.itag ?? null,
                mime: fmt?.mime_type ?? fmt?.mimeType ?? null,
                ext,
                hasVideo: Boolean(fmt?.has_video),
                audioOnly: Boolean(isAudioFormat(fmt) && !fmt?.has_video),
                legacyProgressive: used_legacy_progressive,
            },
            client_report: clientReport,
            client_report_text: formatClientReport(clientReport),
            videoId,
            originalUrl: url,
            resolveOpts: { ...opts },
        };
    }

    async getPlaylist(playlistIdOrUrl, opts = {}) {
        const str = (playlistIdOrUrl || '').trim();
        const match = str.match(/[?&]list=([^&]+)/);
        const playlistId = match ? match[1] : str;
        const client = await this._client(opts);

        let playlist = null;
        try {
            playlist = await client.getPlaylist(playlistId);
        } catch (_) {
            if (client.music) {
                try { playlist = await client.music.getPlaylist(playlistId); } catch (_) {}
            }
        }
        if (!playlist) throw new Error('Playlist not found');

        const title = (playlist.info?.title || playlist.title || playlist.header?.title?.text) || 'YouTube Playlist';
        const author = (playlist.info?.author?.name || playlist.author || playlist.header?.author?.name) || '';
        const source = playlist.videos || playlist.contents || [];

        const items = [];
        for (const v of source) {
            let id = v.id || v.videoId;
            if (!id && v.url) {
                const m = v.url.match(/[?&]v=([^&]+)/);
                if (m) id = m[1];
            }
            if (!id) continue;
            const itemTitle = typeof v.title === 'string' ? v.title : (v.title?.text || String(v.title || 'Unknown Track'));
            const itemAuthor = typeof v.author === 'string' ? v.author : (v.author?.name || (v.artists && v.artists[0]?.name) || author || '');
            const itemDuration = v.duration?.seconds || v.duration_seconds || (typeof v.duration === 'number' ? v.duration : 0);
            const thumb = this.pickThumb(v.thumbnails || v.thumbnail);
            items.push({
                id: typeof id === 'string' ? id : id[1],
                url: `https://www.youtube.com/watch?v=${typeof id === 'string' ? id : id[1]}`,
                title: String(itemTitle).trim() || 'Unknown Track',
                channel: String(itemAuthor).trim(),
                duration: itemDuration,
                thumbnail: thumb,
            });
        }
        if (items.length === 0) throw new Error('Playlist contained no videos');
        return {
            id: playlistId,
            title: String(title).trim() || 'YouTube Playlist',
            author: String(author).trim(),
            items,
        };
    }

    async resolvePlaylist(rawUrl, opts = {}) {
        const pl = await this.getPlaylist(rawUrl, opts);
        return pl.items;
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

        let raw = [];
        if (Array.isArray(res.videos)) {
            raw = res.videos;
        } else if (Array.isArray(res.results)) {
            raw = res.results;
        } else if (Array.isArray(res.contents)) {
            raw = res.contents;
        } else if (Array.isArray(res)) {
            raw = res;
        }

        const out = [];
        for (const r of raw) {
            if (!r) continue;
            const id = r.id || r.videoId || r.video_id;
            if (!id || typeof id !== 'string') continue;

            let title = 'Unknown';
            if (typeof r.title === 'string' && r.title.trim()) {
                title = r.title.trim();
            } else if (typeof r.title?.text === 'string' && r.title.text.trim()) {
                title = r.title.text.trim();
            } else if (Array.isArray(r.title?.runs) && r.title.runs[0]?.text) {
                title = r.title.runs.map(run => run.text || '').join('').trim() || 'Unknown';
            } else if (r.name && typeof r.name === 'string') {
                title = r.name.trim();
            }

            let author = 'YouTube';
            if (typeof r.author === 'string' && r.author.trim()) {
                author = r.author.trim();
            } else if (typeof r.author?.name === 'string' && r.author.name.trim()) {
                author = r.author.name.trim();
            } else if (typeof r.author?.text === 'string' && r.author.text.trim()) {
                author = r.author.text.trim();
            } else if (Array.isArray(r.artists) && r.artists[0]?.name) {
                author = r.artists.map(a => a.name || '').filter(Boolean).join(', ').trim() || 'YouTube';
            } else if (r.channel && typeof r.channel === 'string' && r.channel.trim()) {
                author = r.channel.trim();
            } else if (r.channel && typeof r.channel?.name === 'string' && r.channel.name.trim()) {
                author = r.channel.name.trim();
            } else if (r.short_byline_text?.text) {
                author = String(r.short_byline_text.text).trim();
            } else if (Array.isArray(r.short_byline_text?.runs) && r.short_byline_text.runs[0]?.text) {
                author = r.short_byline_text.runs.map(rn => rn.text || '').join('').trim() || 'YouTube';
            }

            let durationSecs = 0;
            if (typeof r.duration?.seconds === 'number') {
                durationSecs = r.duration.seconds;
            } else if (typeof r.duration_seconds === 'number') {
                durationSecs = r.duration_seconds;
            } else if (typeof r.duration === 'number') {
                durationSecs = r.duration;
            } else if (typeof r.length_seconds === 'number' || typeof r.length_seconds === 'string') {
                durationSecs = parseInt(r.length_seconds, 10) || 0;
            }

            let durationText = '';
            if (typeof r.duration?.text === 'string' && r.duration.text.trim()) {
                durationText = r.duration.text.trim();
            } else if (typeof r.duration === 'string' && r.duration.trim()) {
                durationText = r.duration.trim();
            }

            if (!durationText && durationSecs > 0) {
                const m = Math.floor(durationSecs / 60);
                const s = Math.floor(durationSecs % 60);
                durationText = `${m}:${s < 10 ? '0' : ''}${s}`;
            } else if (durationText && (!durationSecs || durationSecs === 0)) {
                const parts = String(durationText).split(':').map(p => parseInt(p, 10));
                if (parts.every(p => !isNaN(p))) {
                    if (parts.length === 2) durationSecs = parts[0] * 60 + parts[1];
                    else if (parts.length === 3) durationSecs = parts[0] * 3600 + parts[1] * 60 + parts[2];
                }
            }

            const thumb = this.pickThumb(r.thumbnails || r.thumbnail || r.best_thumbnail);

            out.push({
                id: String(id),
                title: String(title).trim() || 'Unknown',
                channel: String(author).trim() || 'YouTube',
                duration: Number(durationSecs) || 0,
                duration_text: String(durationText || '').trim(),
                thumbnail: thumb || null,
                url: `https://www.youtube.com/watch?v=${id}`,
            });
            if (out.length >= 15) break;
        }
        return out;
    }

    hasValidAudioContainer(f) {
        if (!f || !f.mime_type) return false;
        const mime = String(f.mime_type).toLowerCase();
        return (
            mime.startsWith('audio/mp4') ||
            mime.startsWith('audio/webm') ||
            mime.startsWith('audio/m4a') ||
            mime.startsWith('audio/ogg') ||
            mime.startsWith('audio/opus') ||
            mime.startsWith('video/mp4') ||
            mime.startsWith('video/webm')
        );
    }

    hasValidAudioCodec(f) {
        if (!f) return false;
        const mime = String(f.mime_type || '').toLowerCase();
        return (
            mime.includes('opus') ||
            mime.includes('mp4a') ||
            mime.includes('aac') ||
            mime.includes('vorbis') ||
            mime.includes('flac') ||
            mime.startsWith('audio/mp4') ||
            mime.startsWith('audio/webm') ||
            mime.startsWith('audio/ogg')
        );
    }

    scoreFormat(fmt) {
        return scoreFormat(fmt);
    }

    selectBestAudioFormat(candidates, quality = 'best', targetContainer = null) {
        if (!candidates || !candidates.length) return null;
        const isDecipherable = (f) => Boolean(f && (f.url || f.signature_cipher || f.cipher || typeof f.decipher === 'function'));
        const valid = candidates.filter((f) => f && isDecipherable(f));
        if (!valid.length) return candidates[0] || null;

        const scoreFn = (f) => {
            let base = scoreFormat(f);
            const mimeL = String(f.mime_type || f.mimeType || '').toLowerCase();
            if (targetContainer === 'mp4' && (mimeL.includes('mp4') || mimeL.includes('m4a'))) {
                base += 0.5;
            } else if (targetContainer === 'webm' && (mimeL.includes('webm') || mimeL.includes('opus'))) {
                base += 0.5;
            }
            return base;
        };

        const sorted = [...valid].sort((a, b) => {
            const diffScore = scoreFn(b) - scoreFn(a);
            if (diffScore !== 0) return diffScore;
            const aAudioOnly = ((a.has_audio && !a.has_video) || !a.has_video) ? 1 : 0;
            const bAudioOnly = ((b.has_audio && !b.has_video) || !b.has_video) ? 1 : 0;
            if (bAudioOnly !== aAudioOnly) return bAudioOnly - aAudioOnly;
            const aHasUrl = a.url ? 1 : 0;
            const bHasUrl = b.url ? 1 : 0;
            if (bHasUrl !== aHasUrl) return bHasUrl - aHasUrl;
            const aHasLen = ((a.content_length && a.content_length > 0) || (a.contentLength && parseInt(a.contentLength, 10) > 0)) ? 1 : 0;
            const bHasLen = ((b.content_length && b.content_length > 0) || (b.contentLength && parseInt(b.contentLength, 10) > 0)) ? 1 : 0;
            if (bHasLen !== aHasLen) return bHasLen - aHasLen;
            return (b.bitrate || 0) - (a.bitrate || 0);
        });

        return sorted[0] || valid[0];
    }

    pickAudioFormat(candidates, quality = 'best', targetContainer = null) {
        return this.selectBestAudioFormat(candidates, quality, targetContainer);
    }
}

window.AuralisYouTube = new YouTubeResolver();
// Expose scoreFormat globally for testing / debugging (rodio no opus — m4a preference)
if (typeof window !== 'undefined') {
    window.scoreFormat = scoreFormat;
    try { window.AuralisYouTube.scoreFormat = scoreFormat.bind(window.AuralisYouTube); } catch (_) {}
}
try { if (typeof globalThis !== 'undefined') globalThis.scoreFormat = scoreFormat; } catch (_) {}
try { if (typeof module !== 'undefined' && module.exports) module.exports.scoreFormat = scoreFormat; } catch (_) {}