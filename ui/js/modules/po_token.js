/**
 * po_token.js — BgUtils wrapper for YouTube PO-token generation (2026)
 * Uses bgutils-js (LuanRT/BgUtils) to mint WebPO tokens bound to videoId.
 * Falls back gracefully if BotGuard attestation fails (e.g., WebView not passing integrity).
 */

let bgUtilsPromise = null;

async function loadBgUtils() {
    if (bgUtilsPromise) return bgUtilsPromise;
    bgUtilsPromise = (async () => {
        try {
            // bgutils-js 4.x dist: ui/vendor/bgutils/exports/{webpo,botguard}.js
            const mod = await import('../../vendor/bgutils/exports/webpo.js')
                .catch(() => import('../vendor/bgutils/exports/webpo.js'))
                .catch(() => import('../../vendor/bgutils/exports/botguard.js'))
                .catch(() => import('/vendor/bgutils/exports/webpo.js'));
            return mod;
        } catch (e) {
            console.warn('[PoToken] BgUtils not available:', e?.message || e);
            return null;
        }
    })();
    return bgUtilsPromise;
}

/**
 * Generate a WebPO token for a videoId using Innertube challenge + BgUtils.
 * Returns { poToken, visitorData, contentBinding } or null on failure.
 * Caller should cache per videoId (TTL 6h) and pass to Innertube.create({ poToken, visitorData }).
 */
export async function generatePoTokenForVideo(innertube, videoId) {
    try {
        const bg = await loadBgUtils();
        if (!bg || !innertube?.getAttestationChallenge) {
            console.warn('[PoToken] BgUtils or getAttestationChallenge unavailable — skipping PO token');
            return null;
        }
        // 1. Get challenge (ENGAGEMENT_TYPE_UNBOUND is used for GVS)
        const challengeResponse = await innertube.getAttestationChallenge('ENGAGEMENT_TYPE_UNBOUND').catch(() => null);
        if (!challengeResponse?.bg_challenge) {
            console.warn('[PoToken] No bg_challenge in response');
            return null;
        }
        const interpreterUrl = challengeResponse.bg_challenge.interpreter_url?.private_do_not_access_or_else_trusted_resource_url_wrapped_value;
        if (!interpreterUrl) {
            console.warn('[PoToken] No interpreter_url');
            return null;
        }
        const bgScriptResponse = await fetch(`https:${interpreterUrl}`).then(r => r.text()).catch(() => null);
        if (!bgScriptResponse) return null;

        // 2. Use BgUtils to run BotGuard and get integrity token
        // BgUtils 4.x exports: BgChallenge, WebPoMinter etc. Try to use high-level helper if available.
        // Fallback to low-level: new BgChallenge(...).execute() -> getPoToken
        let poToken = null;
        let visitorData = innertube.session?.context?.client?.visitorData || null;

        // Attempt high-level API (if bg object has generatePoToken)
        if (typeof bg.generatePoToken === 'function') {
            const res = await bg.generatePoToken({ innertube, videoId, bgScript: bgScriptResponse, challenge: challengeResponse.bg_challenge }).catch(() => null);
            if (res?.poToken) poToken = res.poToken;
            if (res?.visitorData) visitorData = res.visitorData;
        }

        // Low-level fallback: use BgUtils core classes
        if (!poToken && bg.BotGuard) {
            try {
                const botGuard = new bg.BotGuard(bgScriptResponse);
                await botGuard.initialize();
                const webPoMinter = await botGuard.createWebPoMinter(challengeResponse.bg_challenge);
                poToken = await webPoMinter.mint(videoId);
                visitorData = visitorData || challengeResponse.bg_challenge.visitorData || null;
            } catch (e) {
                console.warn('[PoToken] BotGuard mint failed:', e?.message || e);
            }
        }

        if (!poToken) {
            console.warn('[PoToken] Failed to mint token for', videoId);
            return null;
        }

        console.log(`[PoToken] Minted for ${videoId}: ${poToken.slice(0, 20)}… (visitorData ${visitorData ? visitorData.slice(0, 12) + '…' : 'none'})`);
        return { poToken, visitorData, contentBinding: videoId };
    } catch (e) {
        console.warn('[PoToken] generatePoTokenForVideo error:', e?.message || e);
        return null;
    }
}

// Simple in-memory cache (TTL 6h) — avoids re-minting same video within session
const poCache = new Map(); // videoId -> { poToken, visitorData, expires }

export function getCachedPoToken(videoId) {
    const entry = poCache.get(videoId);
    if (!entry) return null;
    if (Date.now() > entry.expires) {
        poCache.delete(videoId);
        return null;
    }
    return entry;
}

export function setCachedPoToken(videoId, data) {
    poCache.set(videoId, { ...data, expires: Date.now() + 6 * 60 * 60 * 1000 });
}
