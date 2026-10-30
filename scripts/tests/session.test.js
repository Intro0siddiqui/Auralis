import { describe, it, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import {
    SessionManager,
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
} from '../../ui/js/modules/session.js';

describe('SessionManager Unit Tests', () => {
    beforeEach(() => {
        clearSession();
        // Clear mock localStorage if simulated
        if (typeof globalThis.localStorage !== 'undefined') {
            globalThis.localStorage.clear();
        }
    });

    describe('Cookie String Formatting', () => {
        it('formats single and multiple Set-Cookie headers properly', () => {
            const raw = [
                'VISITOR_INFO1_LIVE=abc12345; path=/; domain=.youtube.com; Secure; HttpOnly',
                'YSC=xyz789; path=/; domain=.youtube.com; HttpOnly',
                'GPS=1; path=/; domain=.youtube.com',
            ];
            const result = formatCookieString(raw);
            assert.equal(result, 'VISITOR_INFO1_LIVE=abc12345; YSC=xyz789; GPS=1');
        });

        it('handles duplicate cookie keys by keeping the latest value', () => {
            const raw = [
                'GPS=0; path=/',
                'GPS=1; path=/',
            ];
            const result = formatCookieString(raw);
            assert.equal(result, 'GPS=1');
        });

        it('gracefully handles empty, null, and non-array inputs', () => {
            assert.equal(formatCookieString(null), '');
            assert.equal(formatCookieString(undefined), '');
            assert.equal(formatCookieString([]), '');
            assert.equal(formatCookieString('PREF=f6=400; path=/'), 'PREF=f6=400');
        });
    });

    describe('visitorData HTML Extraction', () => {
        it('extracts VISITOR_DATA from ytcfg.set script payload', () => {
            const html = `<html><script>ytcfg.set({"VISITOR_DATA": "CgtPZ2ZWTEZ3TDNWVSjo_ZHXBjIKCg", "INNERTUBE_CONTEXT": {}});</script></html>`;
            const vd = extractVisitorDataFromHtml(html);
            assert.equal(vd, 'CgtPZ2ZWTEZ3TDNWVSjo_ZHXBjIKCg');
        });

        it('extracts escaped visitorData from embedded JSON strings', () => {
            const html = `<div data-player="{\\"VISITOR_DATA\\": \\"CgtEscapedVd123\\"}"></div>`;
            const vd = extractVisitorDataFromHtml(html);
            assert.equal(vd, 'CgtEscapedVd123');
        });

        it('returns null when visitorData is not in HTML', () => {
            assert.equal(extractVisitorDataFromHtml('<html><body>No data</body></html>'), null);
            assert.equal(extractVisitorDataFromHtml(''), null);
            assert.equal(extractVisitorDataFromHtml(null), null);
        });
    });

    describe('Session Validity and Expiration', () => {
        it('marks valid session with VISITOR_INFO1_LIVE as valid', () => {
            const sess = {
                cookie: 'VISITOR_INFO1_LIVE=test_token; YSC=sec_123',
                visitorData: 'vd_123',
                createdAt: Date.now() - 1000,
            };
            assert.ok(isSessionValid(sess));
        });

        it('marks session without core cookies as invalid', () => {
            assert.equal(isSessionValid({ cookie: 'random=123' }), false);
            assert.equal(isSessionValid({ cookie: '' }), false);
            assert.equal(isSessionValid(null), false);
        });

        it('marks expired session as invalid', () => {
            const sess = {
                cookie: 'VISITOR_INFO1_LIVE=old_token',
                createdAt: Date.now() - (100 * 24 * 60 * 60 * 1000), // 100 days old
            };
            assert.equal(isSessionValid(sess, 90 * 24 * 60 * 60 * 1000), false);
        });
    });

    describe('Session Persistence & In-Memory Fallback', () => {
        it('saves and retrieves session properly', () => {
            const sess = {
                cookie: 'VISITOR_INFO1_LIVE=persisted_val; YSC=1',
                visitorData: 'vd_persisted',
                createdAt: Date.now(),
            };
            saveSession(sess);

            const retrieved = getStoredSession();
            assert.ok(retrieved);
            assert.equal(retrieved.cookie, sess.cookie);
            assert.equal(retrieved.visitorData, 'vd_persisted');
        });

        it('updates visitorData on existing session', () => {
            saveSession({
                cookie: 'VISITOR_INFO1_LIVE=val1',
                visitorData: 'old_vd',
            });
            updateSessionVisitorData('new_vd_updated');

            const retrieved = getStoredSession();
            assert.equal(retrieved.visitorData, 'new_vd_updated');
            assert.equal(retrieved.cookie, 'VISITOR_INFO1_LIVE=val1');
        });

        it('clears session from storage', () => {
            saveSession({
                cookie: 'VISITOR_INFO1_LIVE=to_delete',
            });
            clearSession();
            assert.equal(getStoredSession(), null);
        });
    });

    describe('Guest Session Harvesting (Mocked)', () => {
        it('harvests cookies and extracts visitorData from fetch response', async () => {
            const mockFetch = async () => ({
                headers: {
                    getSetCookie: () => [
                        'VISITOR_INFO1_LIVE=harvested_cookie_123; path=/; domain=.youtube.com',
                        'YSC=ysc_val; path=/',
                    ],
                },
                text: async () => `ytcfg.set({"VISITOR_DATA": "vd_from_html_456"});`,
            });

            const session = await harvestGuestSession(mockFetch);
            assert.ok(session);
            assert.equal(session.cookie, 'VISITOR_INFO1_LIVE=harvested_cookie_123; YSC=ysc_val');
            assert.equal(session.visitorData, 'vd_from_html_456');

            // Verify it was stored
            const stored = getStoredSession();
            assert.equal(stored.cookie, session.cookie);
            assert.equal(stored.visitorData, session.visitorData);
        });

        it('getOrInitSession reuses existing stored session if valid', async () => {
            saveSession({
                cookie: 'VISITOR_INFO1_LIVE=existing_token',
                visitorData: 'existing_vd',
                createdAt: Date.now(),
            });

            let fetchCalled = false;
            const mockFetch = async () => {
                fetchCalled = true;
                return {};
            };

            const session = await getOrInitSession(mockFetch);
            assert.equal(fetchCalled, false, 'Should not fetch when existing valid session is present');
            assert.equal(session.cookie, 'VISITOR_INFO1_LIVE=existing_token');
        });
    });

    describe('Header Injection', () => {
        it('injects Cookie header when not present', () => {
            const session = { cookie: 'VISITOR_INFO1_LIVE=token_xyz' };
            const headers = { 'User-Agent': 'TestUA' };
            const injected = injectSessionHeaders(headers, session);
            assert.equal(injected['User-Agent'], 'TestUA');
            assert.equal(injected['Cookie'], 'VISITOR_INFO1_LIVE=token_xyz');
        });

        it('does not overwrite existing explicit Cookie header', () => {
            const session = { cookie: 'VISITOR_INFO1_LIVE=default' };
            const headers = { 'Cookie': 'CUSTOM_COOKIE=123' };
            const injected = injectSessionHeaders(headers, session);
            assert.equal(injected['Cookie'], 'CUSTOM_COOKIE=123');
        });
    });
});
