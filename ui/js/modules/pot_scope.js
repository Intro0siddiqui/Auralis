/**
 * pot_scope.js — where a PO token is allowed to travel.
 *
 * A PO token is *platform-bound*. The googlevideo edge only honours a token on
 * the client family it was minted for, and rejects the request with a 403 at
 * byte 0 (empty `text/plain` body) when the token does not match the client
 * that produced the URL.
 *
 * The only token this app can mint is a BotGuard/WEB one (`WebPoMinter` in
 * `po_token.js`), so it is valid for web-family clients only. Stapling it onto
 * an `ios` / `android` / `android_vr` URL is worse than sending no token at
 * all: the client-matched `User-Agent` is then sent against a URL carrying a
 * foreign token, and every client in the rotation fails identically.
 *
 * Kept as a separate, dependency-free module so it can be unit-tested directly
 * instead of by asserting on the text of `youtube.js`.
 */

/**
 * Clients whose URLs may carry a BotGuard/WEB-minted PO token.
 *
 * Deliberately short. Per the yt-dlp PO Token Guide's enforcement table:
 * `mweb`/`web`/`web_safari` require a GVS token; `tv`, `android_vr` and
 * `web_embedded` require none, and `tv_simply` needs a platform-specific token
 * we have no way to mint. So `TV` must stay out — a web token on a TV URL is
 * downside with no upside, since TV does not want a token from any source.
 *
 * `WEB_SAFARI` is absent even though the guide lists it as GVS-requiring,
 * because `orderedClients` in youtube.js cannot produce it — an entry the
 * resolver can never emit is a trap that fails silently the day someone wires
 * the client up. `scripts/tests/pot_scope.test.js` asserts every member here
 * appears in `orderedClients`, so this list cannot drift out of sync.
 */
export const WEB_FAMILY_CLIENTS = ['MWEB', 'WEB'];

/**
 * May a token ride along on a URL produced by `winningClient`?
 *
 * @param {string|null|undefined} winningClient  the InnerTube client that won
 * @param {boolean} hasToken        is there a token to carry at all
 * @param {boolean} tokenIsWebBound true when the token came from *our* Web
 *        minter (or our cache of it). False means the user supplied it in
 *        Settings, and it may legitimately be an iOS/Android token — so it is
 *        passed through untouched rather than second-guessed.
 */
export function potUsableForClient(winningClient, hasToken, tokenIsWebBound) {
    if (!hasToken) return false;
    if (!tokenIsWebBound) return true; // user's own token: respect it
    return WEB_FAMILY_CLIENTS.includes(String(winningClient || '').toUpperCase());
}

/**
 * Apply (or remove) `pot` on a media URL according to the winning client.
 *
 * @param {string} streamUrl
 * @param {{winningClient?: string|null, token?: string|null, tokenIsWebBound?: boolean}} opts
 * @returns {{url: string, action: string, detail?: string}}
 *   action is one of: `attached`, `already-present`, `stripped`, `no-token`,
 *   `not-applicable`, `unparsable`.
 */
export function applyPoTokenToUrl(streamUrl, opts = {}) {
    const { winningClient = null, token = null, tokenIsWebBound = false } = opts;
    if (!streamUrl) return { url: streamUrl, action: 'not-applicable' };

    let u;
    try {
        u = new URL(streamUrl);
    } catch (_) {
        return { url: streamUrl, action: 'unparsable' };
    }

    // Only media hosts; never pollute an arbitrary URL.
    if (!u.hostname.includes('googlevideo.com') && !u.hostname.includes('youtube.com')) {
        return { url: streamUrl, action: 'not-applicable' };
    }

    const usable = potUsableForClient(winningClient, Boolean(token), tokenIsWebBound);

    if (usable) {
        if (u.searchParams.has('pot')) {
            return { url: u.toString(), action: 'already-present' };
        }
        u.searchParams.set('pot', token);
        return { url: u.toString(), action: 'attached' };
    }

    if (u.searchParams.has('pot')) {
        u.searchParams.delete('pot');
        return {
            url: u.toString(),
            action: 'stripped',
            detail: `a Web-bound pot is not valid for ${winningClient}`,
        };
    }

    return { url: streamUrl, action: usable ? 'attached' : 'no-token' };
}
