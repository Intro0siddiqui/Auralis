#!/usr/bin/env node
/**
 * pot_scope.test.js — real behavioural tests for ui/js/modules/pot_scope.js
 *
 *   node --test scripts/tests/pot_scope.test.js
 *
 * These import the module and exercise it, rather than asserting on the text of
 * youtube.js. They exist because of a real device failure: a BotGuard/WEB PO
 * token was being stapled onto `ios` and `android_vr` googlevideo URLs, and the
 * edge answered 403 Forbidden at byte 0 with an empty text/plain body. Because
 * every client in the rotation received the same invalid token, rotating clients
 * changed nothing — which is precisely what the on-device client report showed
 * (MWEB/TV/WEB UNPLAYABLE, ANDROID SABR-only, then IOS and ANDROID_VR both
 * "chosen" with real audio URLs and both 403 at byte 0).
 */
import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { applyPoTokenToUrl, potUsableForClient, WEB_FAMILY_CLIENTS } from '../../ui/js/modules/pot_scope.js';

const here = import.meta.dirname ?? path.dirname(new URL(import.meta.url).pathname);
const ytPath = path.resolve(here, '../../ui/js/youtube.js');

const IOS_URL =
    'https://rr5---sn-gwpa-cived.googlevideo.com/videoplayback?expire=1790448328&ei=aL63at-pJNSZvcAPjrfCsAE&ip=2409%3A40c4%3A2%3Acdaa%3A8000%3A%3A&id=o-AKWB9KaPsOMB3&itag=140';
const TOKEN = 'WEB_BOTGUARD_TOKEN_VALUE';

describe('potUsableForClient', () => {
    it('allows a web-bound token only on web-family clients', () => {
        for (const c of WEB_FAMILY_CLIENTS) {
            assert.equal(potUsableForClient(c, true, true), true, `${c} should accept a web token`);
        }
        // These are the clients that produced real audio URLs on device and then 403'd.
        for (const c of ['IOS', 'ANDROID', 'ANDROID_VR', 'TV', 'ANDROID_MUSIC']) {
            assert.equal(potUsableForClient(c, true, true), false, `${c} must reject a web token`);
        }
    });

    it('is case-insensitive about the client name', () => {
        assert.equal(potUsableForClient('mweb', true, true), true);
        assert.equal(potUsableForClient('ios', true, true), false);
    });

    it('never carries a token that does not exist', () => {
        for (const c of ['MWEB', 'IOS', 'ANDROID_VR']) {
            assert.equal(potUsableForClient(c, false, true), false);
        }
    });

    it("passes a user-supplied token through instead of second-guessing it", () => {
        // Settings can hold an iOS/Android token; we did not mint it, so we
        // must not strip it just because the winner is not web-family.
        assert.equal(potUsableForClient('IOS', true, false), true);
        assert.equal(potUsableForClient('ANDROID_VR', true, false), true);
    });
});

describe('applyPoTokenToUrl — the device failure', () => {
    it('does NOT put a web-bound token on an IOS url (regression)', () => {
        const r = applyPoTokenToUrl(IOS_URL, { winningClient: 'IOS', token: TOKEN, tokenIsWebBound: true });
        assert.equal(r.action, 'no-token');
        assert.ok(!r.url.includes('pot='), `web token leaked onto an IOS url: ${r.url}`);
    });

    it('does NOT put a web-bound token on an ANDROID_VR url (regression)', () => {
        const r = applyPoTokenToUrl(IOS_URL, { winningClient: 'ANDROID_VR', token: TOKEN, tokenIsWebBound: true });
        assert.equal(r.action, 'no-token');
        assert.ok(!r.url.includes('pot='), r.url);
    });

    it('strips a token Player.decipher already stapled onto a non-web url', () => {
        // youtubei.esm.mjs appends pot itself when session.player.po_token is
        // set, so the "no pot present" case is not the only one to handle.
        const withPot = `${IOS_URL}&pot=${TOKEN}`;
        const r = applyPoTokenToUrl(withPot, { winningClient: 'IOS', token: TOKEN, tokenIsWebBound: true });
        assert.equal(r.action, 'stripped');
        assert.ok(!r.url.includes('pot='), r.url);
    });

    it('still attaches the token for MWEB, the client it is valid for', () => {
        const r = applyPoTokenToUrl(IOS_URL, { winningClient: 'MWEB', token: TOKEN, tokenIsWebBound: true });
        assert.equal(r.action, 'attached');
        assert.ok(r.url.includes(`pot=${encodeURIComponent(TOKEN)}`), r.url);
    });
});

describe('applyPoTokenToUrl — non-regression', () => {
    it('leaves the rest of the url untouched', () => {
        const r = applyPoTokenToUrl(IOS_URL, { winningClient: 'MWEB', token: TOKEN, tokenIsWebBound: true });
        const u = new URL(r.url);
        assert.equal(u.searchParams.get('itag'), '140');
        assert.equal(u.searchParams.get('ei'), 'aL63at-pJNSZvcAPjrfCsAE');
        assert.equal(u.searchParams.get('ip'), '2409:40c4:2:cdaa:8000::');
        assert.equal(u.hostname, 'rr5---sn-gwpa-cived.googlevideo.com');
    });

    it('is idempotent — applying twice does not double the param', () => {
        const first = applyPoTokenToUrl(IOS_URL, { winningClient: 'MWEB', token: TOKEN, tokenIsWebBound: true });
        const second = applyPoTokenToUrl(first.url, { winningClient: 'MWEB', token: TOKEN, tokenIsWebBound: true });
        assert.equal(second.action, 'already-present');
        assert.equal((second.url.match(/[?&]pot=/g) || []).length, 1);
    });

    it('does not touch a non-media host', () => {
        const other = 'https://example.test/audio.m4a?x=1';
        const r = applyPoTokenToUrl(other, { winningClient: 'MWEB', token: TOKEN, tokenIsWebBound: true });
        assert.equal(r.action, 'not-applicable');
        assert.equal(r.url, other);
    });

    it('reports unparsable input instead of throwing', () => {
        const r = applyPoTokenToUrl('not a url', { winningClient: 'IOS', token: TOKEN, tokenIsWebBound: true });
        assert.equal(r.action, 'unparsable');
        assert.equal(r.url, 'not a url');
    });

    it('handles a missing winner and a null token', () => {
        assert.equal(applyPoTokenToUrl(IOS_URL, {}).action, 'no-token');
        assert.equal(applyPoTokenToUrl(IOS_URL, { winningClient: null, token: null }).action, 'no-token');
        assert.equal(applyPoTokenToUrl('', { winningClient: 'IOS' }).action, 'not-applicable');
    });
});

// ── the durable fix ───────────────────────────────────────────────────────────
// The allowlist itself is only a snapshot. This asserts the invariant that makes
// a stale entry impossible to ship quietly: every client `pot_scope` will carry
// a Web token on must be a client the resolver can actually emit. An entry the
// resolver can never produce is a trap — harmless today, and the day someone
// wires that client up, a Web token silently rides a URL it does not belong on
// and reproduces the byte-0 403 this module exists to prevent.
describe('WEB_FAMILY_CLIENTS cannot name a client the resolver cannot produce', () => {
    const src = fs.readFileSync(ytPath, 'utf8');

    // Pull every client string out of both `orderedClients` literals.
    const produced = new Set();
    for (const m of src.matchAll(/\[\s*'([A-Z_]+)'\s*(?:,\s*'[A-Z_]+'\s*)*\]/g)) {
        for (const c of m[0].matchAll(/'([A-Z_]+)'/g)) produced.add(c[1]);
    }
    assert.ok(produced.has('MWEB') && produced.has('WEB'), 'sanity: the resolver still lists MWEB and WEB');

    for (const c of WEB_FAMILY_CLIENTS) {
        it(`${c} is a client the resolver can produce`, () => {
            assert.ok(produced.has(c), `${c} is in WEB_FAMILY_CLIENTS but not in orderedClients`);
        });
    }

    it('does not name any TV client (tv needs no token; tv_simply needs one we cannot mint)', () => {
        for (const c of WEB_FAMILY_CLIENTS) {
            assert.ok(!/TV/.test(c), `${c} is a TV client and must not be in WEB_FAMILY_CLIENTS`);
        }
        assert.ok(!produced.has('TVHTML5'), 'sanity: TVHTML5 is not a client string in this repo');
    });

    it('covers every web-family client the resolver CAN produce', () => {
        // If a web client is ever added to orderedClients, it must be classified
        // here on purpose rather than defaulting into the "no token" branch.
        for (const c of produced) {
            if (/^(MWEB|WEB|WEB_SAFARI)$/.test(c)) {
                assert.ok(WEB_FAMILY_CLIENTS.includes(c), `${c} is web-family and must be in WEB_FAMILY_CLIENTS`);
            }
        }
    });
});
