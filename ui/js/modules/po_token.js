/**
 * po_token.js — BgUtils wrapper for YouTube PO-token generation (2026)
 * Uses bgutils-js 4.0.3 (BotGuardClient/WebPoMinter) to mint per-video WebPO tokens.
 * Falls back gracefully if BotGuard attestation fails (e.g., WebView not passing integrity).
 * - Jio IPv6 residential: uses nativeFetch (Rust http_fetch) to bypass CORS for
 *   jnn-pa.googleapis.com and interpreter_url; no datacenter proxy required.
 * - Cache is visitorData-bound with TTL 6h (key: videoId::visitorData).
 */

let bgUtilsPromise = null;

/**
 * Native fetch that delegates to Rust `http_fetch` when inside Tauri,
 * bypassing WebView CORS (required for jnn-pa + google.com interpreter).
 * Mirrors youtube.js nativeFetch but isolated for this module.
 */
async function nativeFetchPo(input, init = {}) {
    let url = typeof input === 'string' ? input : (input?.url || String(input));
    const method = init?.method || input?.method || 'GET';
    const cleanHeaders = {};
    const extract = (hdrs) => {
        if (!hdrs) return;
        if (Array.isArray(hdrs)) {
            for (const [k, v] of hdrs) if (v != null) cleanHeaders[String(k)] = String(v);
        } else if (typeof hdrs.forEach === 'function') {
            hdrs.forEach((v, k) => { if (v != null) cleanHeaders[String(k)] = String(v); });
        } else if (typeof hdrs === 'object') {
            for (const [k, v] of Object.entries(hdrs)) if (v != null) cleanHeaders[String(k)] = String(v);
        }
    };
    if (input?.headers) extract(input.headers);
    if (init?.headers) extract(init.headers);
    let body = init?.body ?? null;
    if (body == null && input && typeof input.clone === 'function') {
        try { body = await input.clone().text(); } catch (_) {}
    }
    try {
        const invoke = window.__TAURI__?.core?.invoke || window.__TAURI__?.invoke || window.__TAURI_INTERNALS__?.invoke || window.Auralis?.bridge?.invoke;
        if (typeof invoke === 'function') {
            const resp = await invoke('http_fetch', { request: { url, method, headers: cleanHeaders, body } });
            return new Response(resp.body, { status: resp.status, statusText: resp.status_text, headers: new Headers(resp.headers) });
        }
    } catch (err) {
        console.warn('[PoToken] native http_fetch failed, falling back to window.fetch:', err?.message || err);
    }
    return window.fetch(input, init);
}

async function loadBgUtils() {
    if (bgUtilsPromise) return bgUtilsPromise;
    bgUtilsPromise = (async () => {
        // Correct depth from ui/js/modules/ to ui/vendor/ is ../../vendor/...
        // Include utils/helpers so buildURL/getHeaders are available for proper protobuf GenerateIT (avoids 400)
        const candidates = [
            '../../vendor/bgutils/exports/webpo.js',
            '../../vendor/bgutils/exports/botguard.js',
            '../../vendor/bgutils/exports/utils.js',
            '../../vendor/bgutils/utils/helpers.js',
            '../../vendor/bgutils/utils/constants.js',
            '../../vendor/bgutils/core/WebPoMinter.js',
            '../../vendor/bgutils/core/BotGuardClient.js',
            '../vendor/bgutils/exports/webpo.js',
            '/vendor/bgutils/exports/webpo.js',
        ];
        let merged = {};
        let loaded = false;
        for (const p of candidates) {
            try {
                const mod = await import(p);
                if (mod) {
                    Object.assign(merged, mod);
                    // keep default export if present
                    if (mod.default) Object.assign(merged, mod.default);
                    loaded = true;
                }
            } catch (_) {
                // WebView may throw on missing ESM path; try next candidate
            }
        }
        if (!loaded) {
            console.warn('[PoToken] BgUtils not available: all candidates failed');
            return null;
        }
        return merged;
    })()
        .catch((e) => {
            console.warn('[PoToken] BgUtils not available:', e?.message || e);
            return null;
        })
        .then((val) => {
            // Retry logic: don't cache null forever — reset so next call retries
            if (val === null) bgUtilsPromise = null;
            return val;
        });
    return bgUtilsPromise;
}

/**
 * Generate a WebPO token for a videoId using Innertube challenge + BgUtils.
 * Returns { poToken, visitorData, contentBinding } or null on failure.
 * Caller should cache per videoId (TTL 6h, visitorData-bound) and pass to Innertube.create({ poToken, visitorData }).
 */
export async function generatePoTokenForVideo(innertube, videoId) {
    try {
        const bg = await loadBgUtils();
        if (!bg || !innertube?.getAttestationChallenge) {
            console.warn('[PoToken] BgUtils or getAttestationChallenge unavailable — skipping PO token');
            return null;
        }
        // Wire visitorData from innertube session first
        let visitorData = innertube.session?.context?.client?.visitorData
            || innertube.session?.context?.client?.visitor_data
            || null;

        // 1. Get challenge (ENGAGEMENT_TYPE_UNBOUND is used for GVS)
        const challengeResponse = await innertube.getAttestationChallenge('ENGAGEMENT_TYPE_UNBOUND').catch((e) => {
            console.warn('[PoToken] getAttestationChallenge failed:', e?.message || e);
            return null;
        });
        if (!challengeResponse?.bg_challenge) {
            console.warn('[PoToken] No bg_challenge in response');
            return null;
        }
        // visitorData may also be in challengeResponse (task requirement)
        const crVisitor = challengeResponse.visitorData || challengeResponse.visitor_data
            || challengeResponse.bg_challenge?.visitorData || challengeResponse.bg_challenge?.visitor_data || null;
        if (crVisitor) visitorData = visitorData || crVisitor;

        // Handle both snake_case and camelCase wrapped values
        const bgCh = challengeResponse.bg_challenge;
        const interpreterUrlRaw = bgCh.interpreter_url?.private_do_not_access_or_else_trusted_resource_url_wrapped_value
            || bgCh.interpreter_url?.privateDoNotAccessOrElseTrustedResourceUrlWrappedValue
            || bgCh.interpreterUrl?.privateDoNotAccessOrElseTrustedResourceUrlWrappedValue
            || bgCh.interpreterUrl?.private_do_not_access_or_else_trusted_resource_url_wrapped_value
            || bgCh.interpreterUrl?.privateDoNotAccessOrElseTrustedResourceUrlWrappedValue
            || null;
        const program = bgCh.program || bgCh.prog || null;
        const globalName = bgCh.global_name || bgCh.globalName || null;
        const interpreterHash = bgCh.interpreter_hash || bgCh.interpreterHash || null;
        void interpreterHash; // retained for logging / future; GenerateIT uses fixed requestKey per BgUtils helper (not hash) to avoid 400

        if (!interpreterUrlRaw) {
            console.warn('[PoToken] No interpreter_url in bg_challenge');
            return null;
        }
        let scriptUrl = String(interpreterUrlRaw);
        if (scriptUrl.startsWith('//')) scriptUrl = 'https:' + scriptUrl;
        else if (!scriptUrl.startsWith('https://')) scriptUrl = 'https://' + scriptUrl.replace(/^https?:\/\//, '');

        let bgScriptResponse = null;
        try {
            const r = await nativeFetchPo(scriptUrl, { method: 'GET' });
            if (!r.ok) {
                console.warn('[PoToken] interpreter fetch failed:', r.status, r.statusText);
            } else {
                bgScriptResponse = await r.text();
            }
        } catch (e) {
            console.warn('[PoToken] interpreter fetch error:', e?.message || e);
        }
        // Fallback to window.fetch if nativeFetch returned empty (allowlist blocked)
        if (!bgScriptResponse) {
            try {
                const r2 = await fetch(scriptUrl).then((r) => r.text()).catch(() => null);
                bgScriptResponse = r2;
            } catch (_) {}
        }
        if (!bgScriptResponse) {
            console.warn('[PoToken] No bg script');
            return null;
        }

        let poToken = null;

        // Attempt high-level API if present (defensive)
        if (typeof bg.generatePoToken === 'function') {
            try {
                const res = await bg.generatePoToken({ innertube, videoId, bgScript: bgScriptResponse, challenge: bgCh });
                if (res?.poToken) poToken = res.poToken;
                if (res?.visitorData) visitorData = res.visitorData || visitorData;
            } catch (e) {
                console.warn('[PoToken] generatePoToken high-level failed:', e?.message || e);
            }
        }

        // Low-level: BotGuardClient + WebPoMinter (bgutils 4.x)
        if (!poToken && bg.BotGuardClient && bg.WebPoMinter) {
            try {
                // Evaluate bg script to populate global object (required for BotGuardClient)
                const gName = globalName || 'botguard';
                const gObj = globalThis;
                if (gName && !gObj[gName] && bgScriptResponse) {
                    try {
                        // Execute script in global scope; CSP requires unsafe-eval (allowed in tauri.conf.json)
                        const fn = new Function(bgScriptResponse);
                        fn();
                    } catch (e) {
                        console.warn('[PoToken] bg script eval failed (non-fatal):', e?.message || e);
                    }
                }
                // Create BotGuardClient
                const botGuard = await bg.BotGuardClient.create({
                    program: program || bgScriptResponse,
                    globalName: gName || 'botguard',
                    globalObject: gObj,
                }).catch(() => new bg.BotGuardClient({ program: program || bgScriptResponse, globalName: gName || 'botguard', globalObject: gObj }));

                if (botGuard && typeof botGuard.load === 'function') {
                    try { await botGuard.load(); } catch (_) {}
                }

                // Snapshot with webPoSignalOutput to obtain minter factory
                const webPoSignalOutput = [];
                let botguardResponse = null;
                try {
                    // Try snapshot with webPoSignalOutput (required for WebPoMinter)
                    if (typeof botGuard.snapshot === 'function') {
                        botguardResponse = await botGuard.snapshot({ webPoSignalOutput });
                    } else if (typeof botGuard.snapshotSynchronous === 'function') {
                        botguardResponse = await botGuard.snapshotSynchronous({ webPoSignalOutput });
                    }
                } catch (e) {
                    console.warn('[PoToken] BotGuard snapshot failed:', e?.message || e);
                }

                if (webPoSignalOutput.length && botguardResponse) {
                    // Fetch integrity token via bgutils helpers (BgUtils example) — proper protobuf encoding via buildURL/getHeaders, avoids 400
                    // Example payload is [requestKey, botguardResponse] where requestKey is 'O43z0dpjhgX20SCx4KAo' (same in both examples)
                    const REQUEST_KEY = 'O43z0dpjhgX20SCx4KAo';
                    const payload = [REQUEST_KEY, botguardResponse];
                    const hasHelpers = typeof bg.buildURL === 'function' && typeof bg.getHeaders === 'function';
                    const itUrl = hasHelpers ? bg.buildURL('GenerateIT') : 'https://jnn-pa.googleapis.com/$rpc/google.internal.waa.v1.Waa/GenerateIT';
                    const itHeaders = hasHelpers ? bg.getHeaders() : { 'content-type': 'application/json+protobuf', 'x-goog-api-key': 'AIzaSyDyT5W0Jh49F30Pqqtyfdf7pDLFKLJoAnw', 'x-user-agent': 'grpc-web-javascript/0.1' };
                    try {
                        let itResp = await nativeFetchPo(itUrl, {
                            method: 'POST',
                            headers: itHeaders,
                            body: JSON.stringify(payload),
                        });
                        // Fallback to youtube.com endpoint if jnn-pa 400s (examples show both; bgutils buildURL('GenerateIT', true) => youtube)
                        if (!itResp?.ok && hasHelpers) {
                            try {
                                const altUrl = bg.buildURL('GenerateIT', true);
                                if (altUrl !== itUrl) {
                                    const altResp = await nativeFetchPo(altUrl, {
                                        method: 'POST',
                                        headers: itHeaders,
                                        body: JSON.stringify(payload),
                                    });
                                    if (altResp?.ok) itResp = altResp;
                                }
                            } catch (_) {}
                        }
                        if (itResp?.ok) {
                            const json = await itResp.json();
                            const integrityToken = Array.isArray(json) ? json[0] : json?.integrityToken || json?.integrity_token;
                            const estimatedTtlSecs = Array.isArray(json) ? json[1] : json?.estimatedTtlSecs;
                            const mintRefreshThreshold = Array.isArray(json) ? json[2] : json?.mintRefreshThreshold;
                            const websafeFallbackToken = Array.isArray(json) ? json[3] : json?.websafeFallbackToken;
                            if (integrityToken) {
                                const integrityTokenData = { integrityToken, estimatedTtlSecs, mintRefreshThreshold, websafeFallbackToken };
                                const minter = await bg.WebPoMinter.create(integrityTokenData, webPoSignalOutput);
                                // WebPoMinter.mint as per BgUtils example (contentBinding = videoId, visitorData-bound)
                                poToken = await minter.mintAsWebsafeString(videoId);
                            } else {
                                console.warn('[PoToken] GenerateIT empty integrityToken');
                            }
                        } else {
                            console.warn('[PoToken] GenerateIT bad status:', itResp?.status, itResp?.statusText);
                        }
                    } catch (e) {
                        console.warn('[PoToken] GenerateIT failed:', e?.message || e);
                    }
                }

                // Fallback: direct mint if snapshot gave us minter without GenerateIT (some bgutils builds)
                if (!poToken && webPoSignalOutput[0]) {
                    try {
                        const getMinter = webPoSignalOutput[0];
                        const mintCb = await getMinter(new Uint8Array(0));
                        if (typeof mintCb === 'function') {
                            const out = await mintCb(new TextEncoder().encode(videoId));
                            if (out instanceof Uint8Array) {
                                // u8ToBase64 websafe
                                const b64 = btoa(String.fromCharCode(...out)).replace(/\+/g, '-').replace(/\//g, '_');
                                poToken = b64;
                            }
                        }
                    } catch (_) {}
                }
            } catch (e) {
                console.warn('[PoToken] BotGuard/WebPoMinter mint failed:', e?.message || e);
            }
        }

        // Cold-start token fallback via bgutils helper (no BotGuard needed, works when sps=2)
        if (!poToken && bg.WebPoMinter?.createColdStartToken) {
            try {
                poToken = bg.createColdStartToken ? bg.createColdStartToken(videoId) : bg.WebPoMinter.createColdStartToken(videoId);
                console.log('[PoToken] Using cold-start token for', videoId);
            } catch (e) {
                console.warn('[PoToken] cold-start mint failed:', e?.message || e);
            }
        } else if (!poToken && bg.createColdStartToken) {
            try { poToken = bg.createColdStartToken(videoId); } catch (_) {}
        }

        if (!poToken) {
            console.warn('[PoToken] Failed to mint token for', videoId, '— will fallback to TV/ANDROID_VR');
            return null;
        }

        console.log(`[PoToken] Minted for ${videoId}: ${poToken.slice(0, 20)}… (visitorData ${visitorData ? visitorData.slice(0, 12) + '…' : 'none'})`);
        return { poToken, visitorData, contentBinding: videoId };
    } catch (e) {
        console.warn('[PoToken] generatePoTokenForVideo error:', e?.message || e);
        return null;
    }
}

// Simple in-memory cache (TTL 6h) — visitorData-bound key avoids cross-visitor poisoning
const poCache = new Map(); // key: videoId::visitorData -> { poToken, visitorData, contentBinding, expires }

function cacheKey(videoId, visitorData) {
    return visitorData ? `${videoId}::${visitorData}` : videoId;
}

export function getCachedPoToken(videoId, visitorData = null) {
    // Exact match first
    if (visitorData) {
        const k = cacheKey(videoId, visitorData);
        const entry = poCache.get(k);
        if (entry && Date.now() <= entry.expires) return entry;
        if (entry) poCache.delete(k);
    }
    // Fallback: plain videoId (backward compat) or prefix scan
    const plain = poCache.get(videoId);
    if (plain && Date.now() <= plain.expires) return plain;
    if (plain) poCache.delete(videoId);
    // Scan for any visitorData-bound entry for this videoId (when caller didn't provide visitorData)
    if (!visitorData) {
        for (const [k, v] of poCache.entries()) {
            if (k.startsWith(videoId + '::')) {
                if (Date.now() > v.expires) { poCache.delete(k); continue; }
                return v;
            }
        }
    }
    return null;
}

export function setCachedPoToken(videoId, data) {
    const vd = data?.visitorData || null;
    const k = cacheKey(videoId, vd);
    poCache.set(k, { ...data, expires: Date.now() + 6 * 60 * 60 * 1000 });
    // Also store under plain key for backward compat callers that don't pass visitorData
    if (vd) poCache.set(videoId, { ...data, expires: Date.now() + 6 * 60 * 60 * 1000 });
}
