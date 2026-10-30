/**
 * session.js — Persistent YouTube Guest Session & Cookie Jar
 * -----------------------------------------------------------
 * Emulates a real web browser profile by harvesting, persisting, and reusing
 * authentic YouTube guest cookies (`VISITOR_INFO1_LIVE`, `YSC`, `GPS`, `PREF`)
 * and matching `visitorData` across application restarts.
 *
 * Why this matters:
 * 1. Long-lived browser visitor IDs establish trust with YouTube's anti-bot system,
 *    substantially reducing HTTP 403 Forbidden errors and CAPTCHA rate limits.
 * 2. Binds PO tokens and InnerTube sessions to an aged, stable identity.
 * 3. Outgoing stream and download requests carry matching `Cookie` headers to
 *    `googlevideo.com`, mirroring a real browser's media player.
 */

export const SESSION_STORAGE_KEY = 'auralis_youtube_session_v1';
export const DEFAULT_SESSION_MAX_AGE_MS = 90 * 24 * 60 * 60 * 1000; // 90 days

// In-memory fallback if localStorage is unavailable
let _memSession = null;

/**
 * Standard browser user agent used for harvesting guest cookies.
 */
export const HARVEST_USER_AGENT =
    'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36';

/**
 * Parse raw Set-Cookie header strings into a clean `name=value; name2=value2` cookie string.
 *
 * @param {string[]|string|null|undefined} rawCookies
 * @returns {string}
 */
export function formatCookieString(rawCookies) {
    if (!rawCookies) return '';
    const cookieList = Array.isArray(rawCookies) ? rawCookies : [String(rawCookies)];
    const map = new Map();

    for (const item of cookieList) {
        if (!item || typeof item !== 'string') continue;
        // Set-Cookie can be comma-separated or semicolon-separated directives
        const parts = item.split(';');
        const first = parts[0]?.trim();
        if (!first) continue;
        const eqIdx = first.indexOf('=');
        if (eqIdx > 0) {
            const name = first.slice(0, eqIdx).trim();
            const val = first.slice(eqIdx + 1).trim();
            if (name && val) {
                map.set(name, val);
            }
        }
    }

    return Array.from(map.entries())
        .map(([k, v]) => `${k}=${v}`)
        .join('; ');
}

/**
 * Extract visitorData string from YouTube HTML or response text if present.
 *
 * @param {string} html
 * @returns {string|null}
 */
export function extractVisitorDataFromHtml(html) {
    if (!html || typeof html !== 'string') return null;
    const match =
        html.match(/"VISITOR_DATA"\s*:\s*"([^"]+)"/) ||
        html.match(/"visitorData"\s*:\s*"([^"]+)"/) ||
        html.match(/\\\"VISITOR_DATA\\\"\s*:\s*\\\"([^\\\"]+)\\\"/);
    return match ? match[1] : null;
}

/**
 * Inspect whether a stored session is valid and not expired.
 *
 * @param {object|null} session
 * @param {number} [maxAgeMs=DEFAULT_SESSION_MAX_AGE_MS]
 * @returns {boolean}
 */
export function isSessionValid(session, maxAgeMs = DEFAULT_SESSION_MAX_AGE_MS) {
    if (!session || typeof session !== 'object') return false;
    if (!session.cookie || typeof session.cookie !== 'string' || session.cookie.trim().length === 0) {
        return false;
    }
    // Must contain authentic YouTube visitor identification or session tokens
    const hasCoreCookie =
        session.cookie.includes('VISITOR_INFO1_LIVE') ||
        session.cookie.includes('YSC') ||
        session.cookie.includes('__Secure-');
    if (!hasCoreCookie) return false;

    if (session.createdAt && typeof session.createdAt === 'number') {
        const age = Date.now() - session.createdAt;
        if (age > maxAgeMs) return false;
    }
    return true;
}

/**
 * Read the persistent session from localStorage or memory.
 *
 * @returns {object|null}
 */
export function getStoredSession() {
    try {
        if (typeof localStorage !== 'undefined') {
            const raw = localStorage.getItem(SESSION_STORAGE_KEY);
            if (raw) {
                const parsed = JSON.parse(raw);
                if (isSessionValid(parsed)) {
                    _memSession = parsed;
                    return parsed;
                }
            }
        }
    } catch (_) {}
    return isSessionValid(_memSession) ? _memSession : null;
}

/**
 * Save the session object to localStorage and memory.
 *
 * @param {object} session
 */
export function saveSession(session) {
    if (!session || typeof session !== 'object') return;
    const toStore = {
        cookie: session.cookie || '',
        visitorData: session.visitorData || null,
        createdAt: session.createdAt || Date.now(),
        updatedAt: Date.now(),
    };
    _memSession = toStore;
    try {
        if (typeof localStorage !== 'undefined') {
            localStorage.setItem(SESSION_STORAGE_KEY, JSON.stringify(toStore));
        }
    } catch (_) {}
}

/**
 * Clear the persistent session (e.g. for session rotation upon unexpected 403).
 */
export function clearSession() {
    _memSession = null;
    try {
        if (typeof localStorage !== 'undefined') {
            localStorage.removeItem(SESSION_STORAGE_KEY);
        }
    } catch (_) {}
}

/**
 * Harvest fresh guest cookies directly from YouTube.
 *
 * @param {Function} [fetchImpl=globalThis.fetch]
 * @returns {Promise<object>}
 */
export async function harvestGuestSession(fetchImpl) {
    const fn = fetchImpl || (typeof globalThis.fetch === 'function' ? globalThis.fetch : null);
    if (!fn) throw new Error('No fetch implementation available for session harvest');

    const headers = {
        'User-Agent': HARVEST_USER_AGENT,
        'Accept': 'text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8',
        'Accept-Language': 'en-US,en;q=0.9',
        'Sec-Fetch-Mode': 'navigate',
    };

    let rawCookies = [];
    let visitorData = null;

    try {
        const res = await fn('https://www.youtube.com/', {
            headers,
            redirect: 'manual',
        });

        if (res.headers) {
            if (typeof res.headers.getSetCookie === 'function') {
                rawCookies = res.headers.getSetCookie();
            } else if (typeof res.headers.get === 'function') {
                const sc = res.headers.get('set-cookie');
                if (sc) rawCookies = [sc];
            }
        }

        try {
            const html = await res.text();
            visitorData = extractVisitorDataFromHtml(html);
        } catch (_) {}
    } catch (e) {
        console.warn('[SessionManager] Guest cookie harvest fetch failed:', e);
    }

    const cookieStr = formatCookieString(rawCookies);
    const session = {
        cookie: cookieStr,
        visitorData: visitorData || null,
        createdAt: Date.now(),
        updatedAt: Date.now(),
    };

    if (isSessionValid(session)) {
        saveSession(session);
    }
    return session;
}

/**
 * Get the active session, or automatically harvest and persist one if missing or expired.
 *
 * @param {Function} [fetchImpl]
 * @returns {Promise<object>}
 */
export async function getOrInitSession(fetchImpl) {
    const existing = getStoredSession();
    if (existing) {
        return existing;
    }
    return await harvestGuestSession(fetchImpl);
}

/**
 * Update the stored session with updated visitorData.
 *
 * @param {string} visitorData
 */
export function updateSessionVisitorData(visitorData) {
    if (!visitorData || typeof visitorData !== 'string') return;
    const session = getStoredSession() || { cookie: '', createdAt: Date.now() };
    session.visitorData = visitorData;
    saveSession(session);
}

/**
 * Inject the persistent session cookies into an outgoing headers object.
 *
 * @param {Record<string, string>|null|undefined} headers
 * @param {object|null} [session]
 * @returns {Record<string, string>}
 */
export function injectSessionHeaders(headers = {}, session = null) {
    const sess = session || getStoredSession();
    const result = { ...(headers || {}) };
    if (sess && sess.cookie && !result['Cookie'] && !result['cookie']) {
        result['Cookie'] = sess.cookie;
    }
    return result;
}

export const SessionManager = {
    SESSION_STORAGE_KEY,
    DEFAULT_SESSION_MAX_AGE_MS,
    formatCookieString,
    extractVisitorDataFromHtml,
    isSessionValid,
    getStoredSession,
    saveSession,
    clearSession,
    harvestGuestSession,
    getOrInitSession,
    updateSessionVisitorData,
    injectSessionHeaders,
};

export default SessionManager;
