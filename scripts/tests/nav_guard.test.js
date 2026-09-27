#!/usr/bin/env node
/**
 * nav_guard.test.js — unit tests for ui/js/modules/nav_guard.js
 *
 *   node --test scripts/tests/nav_guard.test.js
 *
 * SCOPE, STATED PLAINLY
 * ---------------------
 * There is no browser runtime in this environment: no jsdom, no npm, no dev
 * server, no CDP. The reported race (tap Download, land on Home) was therefore
 * NEVER REPRODUCED. Nothing here observes the race.
 *
 * What these tests do cover is the guard itself, end to end: the pure decision
 * function, and the htmx wiring driven by synthetic `htmx:beforeRequest` /
 * `htmx:beforeSwap` events through the real `createNavGuard` listeners. They
 * also assert the static claims that the guard rests on (all `#content` writers
 * carry `hx-sync`, no `transition:true`/`hx-boost` crept back), because those are
 * checkable without a browser and they were checkable before too.
 *
 * What remains unverified is in the handoff report: that the device still
 * exhibits the symptom at all, whether the boot load is genuinely slower than the
 * nav request on a real device, and whether the residual window this guard
 * closes is the one the device was actually hitting. `hx-sync="#content:replace"`
 * appears, per the vendored source, to already abort the boot request — which
 * means the live cause may be something this guard does not address.
 */
import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import {
    PAGE_SELECTORS,
    detectPage,
    pageFromPartialPath,
    decideContentSwap,
    createNavGuard,
} from '../../ui/js/modules/nav_guard.js';

const here = path.dirname(fileURLToPath(import.meta.url));
const uiDir = path.join(here, '..', '..', 'ui');

// ── test doubles ──────────────────────────────────────────────────────────────

/** Minimal stand-in for an element: knows which page classes it "contains". */
function fakeContent(pages = []) {
    const list = pages.slice();
    return {
        id: 'content',
        _pages: list,
        querySelector(selector) {
            // Selector strings in PAGE_SELECTORS are comma lists; match any part.
            const parts = selector.split(',').map((s) => s.trim());
            return parts.some((p) => list.includes(p)) ? {} : null;
        },
    };
}

/** Captures listeners registered on a fake body so events can be dispatched. */
function fakeBody() {
    const listeners = new Map();
    return {
        addEventListener(type, fn) {
            if (!listeners.has(type)) listeners.set(type, []);
            listeners.get(type).push(fn);
        },
        fire(type, event) {
            const evt = {
                defaultPrevented: false,
                preventDefault() {
                    this.defaultPrevented = true;
                },
                ...event,
            };
            for (const fn of listeners.get(type) || []) fn(evt);
            return evt;
        },
        types() {
            return [...listeners.keys()].sort();
        },
    };
}

/**
 * A fake htmx request. `beforeRequest` carries `{xhr, target}` on the OWNING
 * element; `beforeSwap` carries `{xhr, target, requestConfig}` on the TARGET
 * element, and `detail.elt` is the target (htmx's `triggerEvent` overwrites
 * `detail.elt` with the dispatch element).
 */
function makeHarness({ contentEl, doc } = {}) {
    const content = contentEl || fakeContent([]);
    const body = fakeBody();
    const getContentEl = () => content;
    const guard = createNavGuard({ doc: doc || null, listenTarget: body, getContentEl });
    return { guard, body, content };
}

function issueRequest(harness, { owner, path, xhr = {} }) {
    return harness.body.fire('htmx:beforeRequest', {
        target: owner,
        detail: { xhr, target: harness.content, requestConfig: { path, verb: 'get' } },
    });
}

function receiveResponse(harness, { path, xhr }) {
    return harness.body.fire('htmx:beforeSwap', {
        target: harness.content,
        detail: {
            xhr,
            target: harness.content,
            shouldSwap: true,
            requestConfig: { path, verb: 'get' },
        },
    });
}

// ── the pure policy ───────────────────────────────────────────────────────────

describe('decideContentSwap', () => {
    it('rejects a stale generation — the fix for the reported symptom', () => {
        // Boot home load issued (gen 1); the user taps Download (gen 2); the
        // stale Home response arrives. This is the clobber, and it is refused.
        const verdict = decideContentSwap(
            { requestGeneration: 1, isBootLoad: true, incomingPage: 'home' },
            { currentGeneration: 2, displayedPage: null }
        );
        assert.equal(verdict.allow, false);
        assert.equal(verdict.reason, 'stale-generation');
    });

    it('allows the current generation', () => {
        const verdict = decideContentSwap(
            { requestGeneration: 2, isBootLoad: false, incomingPage: 'downloads' },
            { currentGeneration: 2, displayedPage: 'home' }
        );
        assert.equal(verdict.allow, true);
        assert.equal(verdict.reason, 'current');
    });

    it('allows a generation that is ahead of the counter (counter reset / re-entry)', () => {
        const verdict = decideContentSwap(
            { requestGeneration: 9, isBootLoad: false, incomingPage: 'library' },
            { currentGeneration: 3, displayedPage: 'home' }
        );
        assert.equal(verdict.allow, true);
    });

    it('rejects a boot load that lands on a #content already showing another page', () => {
        const verdict = decideContentSwap(
            { requestGeneration: 1, isBootLoad: true, incomingPage: 'home' },
            { currentGeneration: 1, displayedPage: 'downloads' }
        );
        assert.equal(verdict.allow, false);
        assert.equal(verdict.reason, 'boot-load-over-other-page');
    });

    it('allows a boot load into an empty #content (the v2.6.17 "precise" half)', () => {
        const verdict = decideContentSwap(
            { requestGeneration: 1, isBootLoad: true, incomingPage: 'home' },
            { currentGeneration: 1, displayedPage: null }
        );
        assert.equal(verdict.allow, true);
    });

    it('allows a DELIBERATE home nav while another page is displayed', () => {
        // Regression guard on rule 2: "tap Home while on Library" must work. Only
        // the #content-owned boot load is restricted, never a nav click.
        const verdict = decideContentSwap(
            { requestGeneration: 5, isBootLoad: false, incomingPage: 'home' },
            { currentGeneration: 5, displayedPage: 'library' }
        );
        assert.equal(verdict.allow, true);
    });

    it('allows navigation between two non-home pages (rule 2 must not generalise)', () => {
        for (const [from, to] of [
            ['library', 'downloads'],
            ['downloads', 'settings'],
            ['settings', 'sync'],
            ['albums', 'artists'],
        ]) {
            const verdict = decideContentSwap(
                { requestGeneration: 4, isBootLoad: false, incomingPage: to },
                { currentGeneration: 4, displayedPage: from }
            );
            assert.equal(verdict.allow, true, `${from} → ${to} must be allowed`);
        }
    });

    it('allows rather than wedges when the inputs are unknown', () => {
        // A guard that blanks the app on a partial is worse than one that misses
        // a clobber, so every uncertain input resolves to "allow".
        const cases = [
            [{}, {}],
            [{ requestGeneration: undefined }, { currentGeneration: 3 }],
            [{ requestGeneration: 3 }, { currentGeneration: undefined }],
            [{ requestGeneration: 3, isBootLoad: true }, { currentGeneration: 3, displayedPage: 'library' }],
        ];
        for (const [permit, state] of cases) {
            assert.equal(
                decideContentSwap(permit, state).allow,
                true,
                `unexpectedly refused: ${JSON.stringify({ permit, state })}`
            );
        }
    });

    it('tolerates null arguments', () => {
        assert.equal(decideContentSwap(null, null).allow, true);
    });
});

// ── the page vocabulary ───────────────────────────────────────────────────────

describe('detectPage', () => {
    it('maps every page partial to its page name', () => {
        for (const [page, selector] of PAGE_SELECTORS) {
            const el = fakeContent([selector.split(',')[0].trim()]);
            assert.equal(detectPage(el), page, `${selector} should be ${page}`);
        }
    });

    it('returns null for an absent, empty or unrecognised #content', () => {
        assert.equal(detectPage(null), null);
        assert.equal(detectPage(undefined), null);
        assert.equal(detectPage(fakeContent([])), null);
        assert.equal(detectPage(fakeContent(['.page-nope'])), null);
    });

    it('resolves the settings page by either of its two selectors', () => {
        assert.equal(detectPage(fakeContent(['.page-settings'])), 'settings');
        assert.equal(detectPage(fakeContent(['#settings-view'])), 'settings');
    });

    it('does not throw on an element with no querySelector', () => {
        assert.equal(detectPage({}), null);
    });
});

describe('pageFromPartialPath', () => {
    it('maps each nav target to the page it renders', () => {
        const expected = {
            '/partials/home.html': 'home',
            '/partials/library.html': 'library',
            '/partials/albums.html': 'albums',
            '/partials/artists.html': 'artists',
            '/partials/playlists.html': 'playlists',
            '/partials/search.html': 'search',
            '/partials/download.html': 'downloads',
            '/partials/sync.html': 'sync',
            '/partials/settings.html': 'settings',
        };
        for (const [p, page] of Object.entries(expected)) {
            assert.equal(pageFromPartialPath(p), page, p);
        }
    });

    it('is unmappable for partials that do not render a #content page', () => {
        // These are the ones that must NOT be mistaken for a page, and they are
        // also the ones whose requests must not bump the generation.
        for (const p of [
            '/partials/nav.html',
            '/partials/player-full.html',
            '/partials/modal-tag-editor.html',
        ]) {
            assert.equal(pageFromPartialPath(p), null, p);
        }
    });

    it('survives query strings, fragments, cache busters and junk', () => {
        assert.equal(pageFromPartialPath('/partials/home.html?org.htmx.cache-buster=1'), 'home');
        assert.equal(pageFromPartialPath('/partials/download.html#anchor'), 'downloads');
        // Deliberately strict: a path outside /partials/ is not a nav target, and
        // "cannot identify the page" must resolve to allow, not to a guess.
        assert.equal(pageFromPartialPath('/other/home.html'), null);
        assert.equal(pageFromPartialPath('/partials/home.htm'), null);
        assert.equal(pageFromPartialPath('home.html'), null);
        assert.equal(pageFromPartialPath(''), null);
        assert.equal(pageFromPartialPath(null), null);
        assert.equal(pageFromPartialPath(undefined), null);
    });
});

// ── the htmx wiring, driven by synthetic events ───────────────────────────────

describe('createNavGuard htmx wiring', () => {
    it('installs exactly the two listeners the policy needs', () => {
        const { guard, body } = makeHarness();
        assert.equal(guard.install(), true);
        assert.deepEqual(body.types(), ['htmx:beforeRequest', 'htmx:beforeSwap']);
    });

    it('is a no-op rather than a throw when there is no event target', () => {
        const guard = createNavGuard({ doc: null, listenTarget: null, getContentEl: () => null });
        assert.equal(guard.install(), false);
    });

    it('STALE: the boot Home response is refused after a Download click', () => {
        // Reproduces the reported ordering, minus the network: the boot load is
        // issued first, the nav click second, and Home's response lands last.
        const home = fakeContent([]);
        const h = makeHarness({ contentEl: home });
        h.guard.install();

        const homeXhr = {};
        const downloadXhr = {};
        issueRequest(h, { owner: home, path: '/partials/home.html', xhr: homeXhr });
        // #content now shows the Download page, as it would after a real swap.
        home._pages = ['.page-downloads'];
        issueRequest(h, { owner: { tagName: 'A' }, path: '/partials/download.html', xhr: downloadXhr });

        const stale = receiveResponse(h, { path: '/partials/home.html', xhr: homeXhr });
        assert.equal(stale.defaultPrevented, true, 'the stale Home swap must be cancelled');
        assert.equal(stale.detail.shouldSwap, false, 'htmx must be told not to swap');

        const fresh = receiveResponse(h, { path: '/partials/download.html', xhr: downloadXhr });
        assert.equal(fresh.defaultPrevented, false, 'the current Download swap must be allowed');
        assert.equal(fresh.detail.shouldSwap, true);
    });

    it('ALLOWS the boot load to land in an empty #content', () => {
        const home = fakeContent([]);
        const h = makeHarness({ contentEl: home });
        h.guard.install();
        const homeXhr = {};
        issueRequest(h, { owner: home, path: '/partials/home.html', xhr: homeXhr });
        const evt = receiveResponse(h, { path: '/partials/home.html', xhr: homeXhr });
        assert.equal(evt.defaultPrevented, false);
    });

    it('REFUSES the boot load when #content already shows a different page', () => {
        // This is the invariant stated without reference to generations, so it
        // holds even if the counter bookkeeping is wrong.
        const home = fakeContent(['.page-library']);
        const h = makeHarness({ contentEl: home });
        h.guard.install();
        const homeXhr = {};
        issueRequest(h, { owner: home, path: '/partials/home.html', xhr: homeXhr });
        const evt = receiveResponse(h, { path: '/partials/home.html', xhr: homeXhr });
        assert.equal(evt.defaultPrevented, true, 'boot load must not overwrite a displayed page');
    });

    it('ALLOWS a deliberate Home nav over a displayed Library page', () => {
        const content = fakeContent(['.page-library']);
        const h = makeHarness({ contentEl: content });
        h.guard.install();
        const homeXhr = {};
        // Owned by the nav <a>, NOT by #content → not a boot load.
        issueRequest(h, { owner: { tagName: 'A' }, path: '/partials/home.html', xhr: homeXhr });
        const evt = receiveResponse(h, { path: '/partials/home.html', xhr: homeXhr });
        assert.equal(evt.defaultPrevented, false);
    });

    it('covers BOTH ownerships: <a>-owned nav and #content-owned boot load', () => {
        // The two owners htmx keys its per-element queue by. Each must be stamped.
        const content = fakeContent([]);
        const h = makeHarness({ contentEl: content });
        h.guard.install();
        assert.equal(h.guard.currentGeneration(), 0);
        issueRequest(h, { owner: content, path: '/partials/home.html', xhr: {} });
        assert.equal(h.guard.currentGeneration(), 1, 'the #content-owned boot load must be stamped');
        issueRequest(h, { owner: { tagName: 'A' }, path: '/partials/library.html', xhr: {} });
        assert.equal(h.guard.currentGeneration(), 2, 'the <a>-owned nav request must be stamped');
    });

    it('ignores requests that do not target #content', () => {
        // #sidebar and #overlay-root requests must not invalidate an in-flight
        // page swap, or the guard would cancel Home for no reason.
        const content = fakeContent([]);
        const h = makeHarness({ contentEl: content });
        h.guard.install();
        const sidebar = fakeContent([]);
        h.body.fire('htmx:beforeRequest', {
            target: sidebar,
            detail: { xhr: {}, target: sidebar },
        });
        h.body.fire('htmx:beforeRequest', {
            target: { tagName: 'DIV' },
            detail: { xhr: {}, target: fakeContent([]) },
        });
        assert.equal(h.guard.currentGeneration(), 0, 'non-#content requests must not bump');
    });

    it('judges #content swaps and leaves every other container alone', () => {
        // The target filter is load-bearing, not defensive tidiness: without it a
        // stale generation would let the guard refuse the player bar's
        // `#overlay-root` swap or the sidebar's `#sidebar` swap.
        //
        // The first version of this test asserted only "not prevented", which a
        // handler that judged the swap and then allowed it also satisfies — it
        // passed with the filter deleted (mutation M5). So this asserts both the
        // observable outcome and the handler's return value (`null` = not mine).
        const content = fakeContent([]);
        const h = makeHarness({ contentEl: content });
        h.guard.install();

        // gen 1: boot home. gen 2: a nav request, which makes gen 1 stale.
        const homeXhr = {};
        issueRequest(h, { owner: content, path: '/partials/home.html', xhr: homeXhr });
        issueRequest(h, { owner: { tagName: 'A' }, path: '/partials/download.html', xhr: {} });

        // Same response, same stamp, but aimed at a different container.
        const elsewhere = fakeContent(['.page-library']);
        const ignored = h.body.fire('htmx:beforeSwap', {
            target: elsewhere,
            detail: {
                xhr: homeXhr,
                target: elsewhere,
                shouldSwap: true,
                requestConfig: { path: '/partials/home.html' },
            },
        });
        assert.equal(ignored.defaultPrevented, false, 'a non-#content swap must be left alone');
        assert.equal(
            h.guard.onBeforeSwap({
                detail: { xhr: homeXhr, target: elsewhere, requestConfig: { path: '/partials/home.html' } },
                preventDefault() {},
            }),
            null,
            'a non-#content target must not be judged at all'
        );

        // The identical response aimed at #content is refused.
        const refused = h.body.fire('htmx:beforeSwap', {
            target: content,
            detail: {
                xhr: homeXhr,
                target: content,
                shouldSwap: true,
                requestConfig: { path: '/partials/home.html' },
            },
        });
        assert.equal(refused.defaultPrevented, true, 'the same stale response IS refused for #content');
    });

    it('allows an unstamped #content response rather than wedging the view', () => {
        // If the guard was not installed when the request was issued there is no
        // stamp to compare. Refusing here would blank the app.
        const content = fakeContent(['.page-library']);
        const h = makeHarness({ contentEl: content });
        h.guard.install();
        const evt = receiveResponse(h, { path: '/partials/library.html', xhr: {} });
        assert.equal(evt.defaultPrevented, false);
    });

    it('tolerates a missing #content and a detail-less event', () => {
        const h = makeHarness({ contentEl: null });
        h.guard.install();
        assert.doesNotThrow(() => h.body.fire('htmx:beforeRequest', { target: null, detail: null }));
        assert.doesNotThrow(() => h.body.fire('htmx:beforeSwap', { target: null, detail: null }));
        assert.doesNotThrow(() => h.body.fire('htmx:beforeSwap', {}));
    });

    it('returns the verdict so a caller can log it', () => {
        const content = fakeContent([]);
        const h = makeHarness({ contentEl: content });
        h.guard.install();
        const xhr = {};
        issueRequest(h, { owner: content, path: '/partials/home.html', xhr });
        const verdict = h.guard.onBeforeSwap({
            detail: { xhr, target: content, requestConfig: { path: '/partials/home.html' } },
            preventDefault() {},
        });
        assert.deepEqual(verdict, { allow: true, reason: 'current' });
    });

    it('falls back to pathInfo.requestPath when requestConfig is absent', () => {
        const content = fakeContent(['.page-downloads']);
        const h = makeHarness({ contentEl: content });
        h.guard.install();
        const xhr = {};
        issueRequest(h, { owner: content, path: '/partials/home.html', xhr });
        const verdict = h.guard.onBeforeSwap({
            detail: { xhr, target: content, pathInfo: { requestPath: '/partials/home.html' } },
            preventDefault() {},
        });
        assert.equal(verdict.allow, false);
        assert.equal(verdict.reason, 'boot-load-over-other-page');
    });
});

// ── static claims the guard rests on ──────────────────────────────────────────

/** Opening tags with their attribute text, walked per tag (not per line). */
function extractTags(html) {
    const tags = [];
    const re = /<([a-zA-Z][\w-]*)((?:[^>"']|"[^"]*"|'[^']*')*)>/g;
    let m;
    let line = 1;
    let last = 0;
    while ((m = re.exec(html))) {
        line += (html.slice(last, m.index).match(/\n/g) || []).length;
        last = m.index;
        tags.push({ name: m[1], attrs: m[2], line });
    }
    return tags;
}

function uiHtmlFiles() {
    const out = [];
    const walk = (dir) => {
        for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
            const full = path.join(dir, entry.name);
            if (entry.isDirectory()) {
                if (entry.name === 'vendor' || entry.name === 'styles' || entry.name === 'js') continue;
                walk(full);
            } else if (entry.name.endsWith('.html')) {
                out.push(full);
            }
        }
    };
    walk(uiDir);
    return out;
}

describe('the static claims nav_guard.js is layered on top of', () => {
    const TARGET_RE = /hx-target\s*=\s*["']\s*#content\s*["']/;
    const SYNC_RE = /hx-sync\s*=\s*["']\s*#content\s*:\s*replace\s*["']/;
    const files = uiHtmlFiles();

    it('every element targeting #content also carries hx-sync="#content:replace"', () => {
        const problems = [];
        let checked = 0;
        for (const file of files) {
            const rel = path.relative(uiDir, file);
            for (const tag of extractTags(fs.readFileSync(file, 'utf8'))) {
                if (!TARGET_RE.test(tag.attrs)) continue;
                checked += 1;
                if (!SYNC_RE.test(tag.attrs)) {
                    problems.push(`${rel}:${tag.line} <${tag.name}>`);
                }
            }
        }
        assert.ok(checked > 0, 'the scan found no hx-target="#content" elements — the parser is broken');
        assert.equal(problems.length, 0, `missing hx-sync on:\n  - ${problems.join('\n  - ')}`);
    });

    it('the #content-owned boot load is itself hx-sync-serialized', () => {
        const tags = extractTags(fs.readFileSync(path.join(uiDir, 'index.html'), 'utf8'));
        const main = tags.find((t) => /id\s*=\s*["']\s*content\s*["']/.test(t.attrs));
        assert.ok(main, 'index.html must still contain <main id="content">');
        assert.match(main.attrs, /hx-get\s*=\s*["']\s*\/partials\/home\.html\s*["']/);
        assert.ok(SYNC_RE.test(main.attrs), `<main id="content"> lost hx-sync (index.html:${main.line})`);
    });

    it('does not reintroduce the race via view transitions or boosting', () => {
        const problems = [];
        for (const file of files) {
            const rel = path.relative(uiDir, file);
            for (const tag of extractTags(fs.readFileSync(file, 'utf8'))) {
                if (!TARGET_RE.test(tag.attrs)) continue;
                if (/(?:^|\s)transition\s*:\s*true/.test(tag.attrs)) {
                    problems.push(`${rel}:${tag.line} transition:true`);
                }
                if (/(?:^|\s)hx-boost/.test(tag.attrs)) {
                    problems.push(`${rel}:${tag.line} hx-boost`);
                }
            }
        }
        assert.equal(problems.length, 0, problems.join('\n  - '));
    });

    it('views.js wires the guard and imports it from nav_guard.js', () => {
        const src = fs.readFileSync(path.join(uiDir, 'js', 'modules', 'views.js'), 'utf8');
        assert.match(src, /from\s+['"]\.\/nav_guard\.js['"]/, 'views.js must import the guard');
        assert.match(src, /navGuard\.install\(\)/, 'views.js must install the guard');
        // The two non-htmx #content writers must claim a generation, or a
        // goToArtist/goToAlbum IPC result can clobber the view the user just
        // navigated to — hx-sync cannot see these writes at all.
        assert.match(src, /claimContentWrite\(\)/);
        assert.match(src, /isContentWriteCurrent\(gen\)/);
    });

    it('the vendored htmx is the version this module was traced against', () => {
        // The char offsets quoted in nav_guard.js only mean anything for 1.9.10.
        // If the bundle is upgraded, those citations must be re-derived.
        const src = fs.readFileSync(path.join(uiDir, 'vendor', 'htmx.min.js'), 'utf8');
        assert.ok(src.includes('version:"1.9.10"'), 'unexpected vendored htmx version; re-derive the offsets');
        assert.ok(src.includes('else if(m==="replace"){ce(g,"htmx:abort")}'), 'the replace arm is gone; re-derive');
        assert.ok(src.includes('r.xhr.abort()'), 'the htmx:abort delegate is gone; re-derive');
        assert.ok(src.includes('var b=new XMLHttpRequest;'), 'the XHR creation point moved; re-derive');
    });
});
