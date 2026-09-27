/**
 * Nav Guard — a request-generation counter for everything that writes `#content`.
 *
 * ── Why this module exists ────────────────────────────────────────────────────
 * Reported on-device: tapping **Download** in the sidebar lands on **Home**
 * instead; tapping again works. The first candidate explanation was that
 * `hx-sync="#content:replace"` *queues* the new request behind the abort of the
 * in-flight one, so the stale Home response lands last. That explanation is
 * **wrong**, and this module records why, because the next person will want to
 * re-derive it.
 *
 * `ui/vendor/htmx.min.js` is htmx 1.9.10, a single line, so the citations are
 * character offsets. In `issueAjaxRequest` (= `he`), after the strategy is parsed:
 *
 *   char 38122  `else if(m==="replace"){ce(g,"htmx:abort")}`
 *   char 38160  `if(f.xhr){if(f.abortable){ce(g,"htmx:abort")}else{ …queue… ;ie(o);return l}}`
 *   char 38238  `var b=new XMLHttpRequest;`
 *
 * The `replace` arm fires `htmx:abort` and **falls through** — it does not
 * `return`. The abort is handled by a `body`-level delegate:
 *
 *   char 47369  `e.addEventListener("htmx:abort",function(e){var t=e.target;var r=ae(t);if(r&&r.xhr){r.xhr.abort()}});`
 *
 * `r` is the same `htmx-internal-data` object as `f`, because `f=ae(g)`
 * (char 38061) and `e.target` is `g`. `XMLHttpRequest.abort()` dispatches
 * `abort` synchronously, which runs
 *
 *   char 41084  `b.onabort=function(){ …;ie(s);w()};`   with (char 38237)
 *   char 38237  `var w=function(){f.xhr=null;f.abortable=false; …};`
 *
 * so `f.xhr` is `null` by the time line 38160 is reached. The `else { …queue…;
 * return l }` arm is therefore unreachable for `replace`, and the new XHR is
 * created immediately. An aborted XHR never fires `onload`, so `handleAjaxResponse`
 * never runs and the stale Home response cannot reach `htmx:beforeSwap` at all.
 *
 * (Separately: `hx-sync`'s selector is resolved by `querySelectorExt`/`Z`, which
 * falls through to a document-wide `re().querySelectorAll` — so `#content`
 * resolves to the same element from a nav `<a>` and from `#content` itself. That
 * is a *different* mechanism from the queueing question and does not reopen it.)
 *
 * ── So what does this guard actually buy? ─────────────────────────────────────
 * `hx-sync="#content:replace"` is necessary but not sufficient:
 *  1. It only knows about htmx's own `xhr` slot on `#content`. It cannot see a
 *     `#content` write that never went through htmx — `views.js`'s `goToArtist`
 *     and `goToAlbum` do `content.innerHTML = …` straight after an `await`.
 *  2. The abort is only issued at request-issue time. A response that has
 *     already been received is not withdrawn by a later abort.
 *  3. It is a static attribute. It cannot be reasoned about in a test.
 *
 * This module supplies the missing invariant in a form that is testable: every
 * request that will write `#content` is stamped with a monotonically increasing
 * generation, and a response is only allowed to swap if it still *is* the newest
 * request. Stamping is keyed on the XHR object rather than on the owning
 * element, so it covers both ownerships uniformly — the `#content`-owned boot
 * load (`<main id="content" hx-trigger="load">`) and the `<a>`-owned nav clicks.
 *
 * No browser runtime is available in this environment, so the race itself was
 * never reproduced. What *is* verified here is the guard's decision logic and
 * its htmx wiring, both directly. See `scripts/tests/nav_guard.test.js`.
 */

// ── Pure helpers ───────────────────────────────────────────────────────────────

/**
 * Page identity for each `#content` partial.
 *
 * This is the single vocabulary shared by the guard, `refreshCurrentView()` and
 * the tests, so the two cannot disagree about "which page is on screen" — a
 * disagreement between them would itself be a second cause of the reported bug.
 * Order matters only in that no two selectors overlap, which the partials
 * satisfy (one `.page-*` class per partial).
 */
export const PAGE_SELECTORS = [
    ['library', '.page-library'],
    ['albums', '.page-albums'],
    ['artists', '.page-artists'],
    ['downloads', '.page-downloads'],
    ['search', '.page-search'],
    ['settings', '.page-settings, #settings-view'],
    ['playlists', '.page-playlists'],
    ['sync', '.page-sync'],
    ['home', '.page-home'],
];

/** `/partials/<name>.html` → the page it renders. Anything else → `null`. */
const PARTIAL_PAGES = {
    home: 'home',
    library: 'library',
    albums: 'albums',
    artists: 'artists',
    playlists: 'playlists',
    search: 'search',
    download: 'downloads',
    sync: 'sync',
    settings: 'settings',
};

/**
 * Which page is currently rendered in `contentEl`?
 * Returns `null` for an absent element, an empty `#content`, or a partial this
 * module does not know — `null` means "cannot prove anything", and every caller
 * treats that as permission to proceed.
 */
export function detectPage(contentEl) {
    if (!contentEl || typeof contentEl.querySelector !== 'function') return null;
    for (let i = 0; i < PAGE_SELECTORS.length; i++) {
        const [page, selector] = PAGE_SELECTORS[i];
        try {
            if (contentEl.querySelector(selector)) return page;
        } catch (_) {
            return null;
        }
    }
    return null;
}

/**
 * The page a request's response will render, derived from its path.
 * Strips query/hash first so a cache-buster cannot defeat the match.
 */
export function pageFromPartialPath(path) {
    if (!path) return null;
    const clean = String(path).split('#')[0].split('?')[0];
    const m = /\/partials\/([a-z-]+)\.html$/.exec(clean);
    if (!m) return null;
    return Object.prototype.hasOwnProperty.call(PARTIAL_PAGES, m[1])
        ? PARTIAL_PAGES[m[1]]
        : null;
}

/**
 * The whole policy, as one pure function. This is what the unit tests drive.
 *
 * `permit` describes the response that is asking to swap:
 *   requestGeneration — the generation stamped when it was issued
 *   isBootLoad       — the request was owned by `#content` itself, i.e. the
 *                      `hx-trigger="load"` boot load, not a deliberate nav click
 *   incomingPage     — the page its response will render, or `null` if unknown
 *
 * `state` is the live truth at swap time:
 *   currentGeneration — the newest generation issued so far
 *   displayedPage    — what `#content` shows right now, or `null`
 *
 * Rejects in exactly two cases:
 *
 *   1. `stale-generation` — a newer request has already been issued. This is
 *      the fix for the reported symptom and is ownership-agnostic.
 *
 *   2. `boot-load-over-other-page` — the boot load's response contradicts what
 *      is on screen. This is the v2.6.17 "precise guard" invariant (allow the
 *      boot load into an empty `#content`, refuse it once a *different* page is
 *      displayed) applied to the htmx swap path, where it did not previously
 *      exist at all — `views.js` only guards the separate IPC path.
 *      `isBootLoad` is true only for the `#content`-owned request, so a
 *      deliberate "tap Home while on Library" is unaffected.
 *
 * An unknown generation or an unknown page is *allowed*. A guard that wedges the
 * app on a partial is worse than a guard that misses one clobber, so every
 * uncertain input resolves to "allow".
 */
export function decideContentSwap(permit, state) {
    const p = permit || {};
    const s = state || {};

    const requestGeneration = Number(p.requestGeneration);
    const currentGeneration = Number(s.currentGeneration);

    if (Number.isFinite(requestGeneration) && Number.isFinite(currentGeneration)) {
        if (requestGeneration < currentGeneration) {
            return { allow: false, reason: 'stale-generation' };
        }
    }

    if (p.isBootLoad && p.incomingPage && s.displayedPage && s.displayedPage !== p.incomingPage) {
        return { allow: false, reason: 'boot-load-over-other-page' };
    }

    return { allow: true, reason: 'current' };
}

// ── htmx wiring ───────────────────────────────────────────────────────────────

/** xhr → { generation, isBootLoad }. A WeakMap so nothing leaks onto the XHR. */
const stamps = new WeakMap();

/**
 * @param {object} [opts]
 * @param {Document} [opts.doc]            — defaults to the ambient `document`
 * @param {Element}   [opts.listenTarget]   — defaults to `doc.body`
 * @param {Function}  [opts.getContentEl]   — defaults to `doc.getElementById('content')`
 *
 * Every DOM dependency is injectable so the guard can be driven by synthetic
 * events under `node --test` with no jsdom and no browser.
 */
export function createNavGuard(opts = {}) {
    const doc = opts.doc || (typeof document !== 'undefined' ? document : null);
    const getContentEl =
        opts.getContentEl ||
        (() => (doc && typeof doc.getElementById === 'function' ? doc.getElementById('content') : null));
    const listenTarget = opts.listenTarget || (doc ? doc.body : null);
    const counter = { value: 0 };

    /** Monotonic stamp; a response carrying an older one is stale. */
    function bump() {
        counter.value += 1;
        return counter.value;
    }

    function currentGeneration() {
        return counter.value;
    }

    /**
     * `htmx:beforeRequest` — fired on the request-**owning** element, just
     * before `send()`, with `detail.xhr` and `detail.target` populated.
     * Bumping here rather than on a nav `click` listener is what makes the guard
     * ownership-agnostic: it stamps the `<a>`-owned nav request, the
     * `#content`-owned boot load, and anything else that targets `#content`,
     * including `home.html`'s "quick download" button and any `htmx.ajax` call.
     */
    function onBeforeRequest(evt) {
        const detail = (evt && evt.detail) || {};
        const contentEl = getContentEl();
        // Only `#content` writes participate; `#sidebar` and `#overlay-root`
        // requests must not invalidate an in-flight page swap.
        if (!contentEl || detail.target !== contentEl) return null;

        const gen = bump();
        const isBootLoad = evt.target === contentEl;
        if (detail.xhr) {
            stamps.set(detail.xhr, { generation: gen, isBootLoad });
        }
        return gen;
    }

    /**
     * `htmx:beforeSwap` — fired on the request-**target** element. The swap has
     * not happened yet and the event is cancelable, so refusing here leaves the
     * DOM untouched; htmx's `handleAjaxResponse` does
     * `if(!ce(c,"htmx:beforeSwap",o))return;` and skips the swap entirely.
     */
    function onBeforeSwap(evt) {
        const detail = (evt && evt.detail) || {};
        const contentEl = getContentEl();
        if (!contentEl || detail.target !== contentEl) return null;

        const stamp = (detail.xhr && stamps.get(detail.xhr)) || null;
        const verdict = decideContentSwap(
            {
                requestGeneration: stamp ? stamp.generation : undefined,
                isBootLoad: Boolean(stamp && stamp.isBootLoad),
                incomingPage: pageFromPartialPath(
                    (detail.requestConfig && detail.requestConfig.path) ||
                        (detail.pathInfo && detail.pathInfo.requestPath)
                ),
            },
            {
                currentGeneration: currentGeneration(),
                displayedPage: detectPage(contentEl),
            }
        );

        if (!verdict.allow) {
            // preventDefault makes `dispatchEvent` return false, which is how
            // htmx learns to skip the swap. `shouldSwap = false` is belt and
            // braces: it also short-circuits htmx's own `if(o.shouldSwap)`.
            if (typeof evt.preventDefault === 'function') evt.preventDefault();
            detail.shouldSwap = false;
        }
        return verdict;
    }

    function install() {
        if (!listenTarget || typeof listenTarget.addEventListener !== 'function') return false;
        listenTarget.addEventListener('htmx:beforeRequest', onBeforeRequest);
        listenTarget.addEventListener('htmx:beforeSwap', onBeforeSwap);
        return true;
    }

    return { install, bump, currentGeneration, onBeforeRequest, onBeforeSwap, detectPage };
}

/**
 * The singleton wired into the running app by `views.js`. Exported so tests can
 * drive the real wiring and so a non-htmx writer can order itself against it.
 */
export const navGuard = createNavGuard();

/**
 * Order a non-htmx `#content` write against the htmx swaps: take a stamp now,
 * and refuse to write if a newer request has been issued by the time the write's
 * `await` resolves. Used by `goToArtist` / `goToAlbum`, which do
 * `content.innerHTML = …` and so bypass `hx-sync` altogether.
 */
export function claimContentWrite() {
    navGuard.bump();
    return navGuard.currentGeneration();
}

export function isContentWriteCurrent(generation) {
    return generation === navGuard.currentGeneration();
}
