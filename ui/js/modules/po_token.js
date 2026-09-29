/**
 * po_token.js — BgUtils wrapper for YouTube PO-token generation (2026)
 * Uses bgutils-js 4.0.3 (BotGuardClient/WebPoMinter) to mint per-video WebPO tokens.
 * Falls back gracefully if BotGuard attestation fails (e.g., WebView not passing integrity).
 * - Jio IPv6 residential: uses nativeFetch (Rust http_fetch) to bypass CORS for
 *   jnn-pa.googleapis.com and interpreter_url; no datacenter proxy required.
 * - Cache is visitorData-bound with TTL 6h (key: videoId::visitorData).
 *
 * ── Instrumented 2026-09-27 ────────────────────────────────────────────────
 *
 * Every step below is recorded through `modules/po_diagnostics.js` and travels
 * to the owner on the existing client-report channel (download row → "Copy
 * report"). The reason is not that the logging was thin: it is that **release
 * builds write nothing to logcat**, so the only environment whose answer matters
 * was producing no evidence at all, and a silent failure here is
 * indistinguishable from "the network refused us".
 *
 * The instrumented steps, and where each one can independently die:
 *
 *   page-context-probe      can this WebView `new Function` at all, and which of
 *                           the globals bgutils needs are present
 *   bgutils-import          the vendored ESM under `ui/vendor/bgutils/`
 *   cache-read              visitorData-bound 6h cache
 *   attestation-challenge   innertube.getAttestationChallenge
 *   visitor-data            the id the token will be bound to
 *   interpreter-url         pulled out of bg_challenge
 *   interpreter-fetch       native http_fetch, then the WebView fetch
 *   new-function-eval       `new Function(bgScriptResponse)` — CSP decides
 *   botguard-global         `globalThis[gName].a` — does the eval leave a global?
 *   botguard-load           the VM handshake; its error was swallowed
 *   snapshot                where the minter factory comes from
 *   generate-it             jnn-pa (or the youtube.com fallback)
 *   mint                    WebPoMinter.mintAsWebsafeString
 *   cold-start-fallback     used only when everything above produced nothing
 *   cache-write
 *
 * Two of those deserve a note, because they are the ones that were invisible:
 *
 *  - **`new-function-eval` succeeding does not mean minting can work.** The
 *    interpreter blob is run through `new Function(body)`, and a body that
 *    declares with `var` binds into that function's scope rather than
 *    `globalThis`. The eval can therefore succeed and leave nothing for
 *    `BotGuardClient` to find, which then throws `EGOU: BotGuard unavailable`
 *    — an error this file used to swallow in a bare `catch (_) {}` and whose
 *    only other symptom was a 3-second `VM operation timed out` naming neither
 *    the cause nor the step. Hence the separate `botguard-global` step, which
 *    measures the global rather than inferring it from the eval's success.
 *  - **`botguard-load`'s swallow is now a recorded failure.** Nothing else
 *    about the control flow changed: the run still falls through to `snapshot`
 *    (which times out) and then to the cold-start token, because those are the
 *    paths that decide the outcome, not this catch.
 */

import {
    SNAPSHOT_SHAPES,
    classifySnapshotOutcome,
    createMintReport,
    recordStep,
    recordMintOutcome,
    runPageContextProbe,
    recordPageProbe,
} from './po_diagnostics.js';

/**
 * How long to keep re-reading `webPoSignalOutput` after `snapshot()` resolves
 * before concluding the VM will never push, and how often.
 *
 * This is an *experiment*, and the number is a guess on purpose: it is bounded
 * by the cost of being wrong (600ms on a path that has already failed) rather
 * than by any knowledge of BotGuard's internals, which we do not have — the
 * interpreter is a blob Google serves at runtime. What it buys is a fact:
 * "still empty after 600ms" and "populated at 80ms" are different bugs, and
 * today they are the same line of text.
 */
const SNAPSHOT_SETTLE_MS = 600;
const SNAPSHOT_POLL_MS = 50;

/**
 * Poll the by-reference signal array for a late push.
 *
 * Skipped entirely when the array already holds something, when the snapshot
 * threw, when the client had no snapshot method, or when the budget is 0 — so
 * a run that was going to work never waits, and a run that threw never pays for
 * a second confirmation of something already known.
 *
 * @returns {Promise<{settleGrew: boolean, settleWaitedMs: number}>}
 */
async function settleWebPoSignalOutput(webPoSignalOutput, { snapshotError, unsupported } = {}) {
    const none = { settleGrew: false, settleWaitedMs: 0 };
    if (SNAPSHOT_SETTLE_MS <= 0) return none;
    if (snapshotError || unsupported) return none;
    if (!Array.isArray(webPoSignalOutput) || webPoSignalOutput.length) return none;

    const t0 = Date.now();
    while (Date.now() - t0 < SNAPSHOT_SETTLE_MS) {
        await new Promise((resolve) => setTimeout(resolve, SNAPSHOT_POLL_MS));
        // Re-read the length every turn: the VM holds the same array, so this is
        // the only place a late push can be observed.
        if (webPoSignalOutput.length) {
            return { settleGrew: true, settleWaitedMs: Date.now() - t0 };
        }
    }
    return { settleGrew: false, settleWaitedMs: Date.now() - t0 };
}

let bgUtilsPromise = null;

/**
 * The report from the most recent `generatePoTokenForVideo` / cache read in
 * this module. Exported because `youtube.js` drives the cache lookup itself
 * (it needs the visitorData first) and so has no other way to learn that a
 * cache hit happened rather than a silent no-op.
 */
let lastMintReport = null;

export function getLastMintReport() {
    return lastMintReport;
}

/**
 * Native fetch that delegates to Rust `http_fetch` when inside Tauri,
 * bypassing WebView CORS (required for jnn-pa + google.com interpreter).
 * Mirrors youtube.js nativeFetch but isolated for this module.
 *
 * @param {string|Request} input
 * @param {RequestInit} [init]
 * @param {{transport?: string, nativeError?: string}} [diag] out-param recording
 *   *which transport actually served the request*. The two have genuinely
 *   different failure modes — an allowlist-blocked `http_fetch` degrades
 *   silently to the WebView fetch, which then runs into CORS — and "the
 *   interpreter fetch returned nothing" is not a finding until you know which of
 *   the two was tried.
 */
async function nativeFetchPo(input, init = {}, diag = null) {
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
            if (diag) diag.transport = 'rust-http_fetch';
            return new Response(resp.body, { status: resp.status, statusText: resp.status_text, headers: new Headers(resp.headers) });
        }
        if (diag) diag.transport = 'webview-fetch(no-invoke)';
    } catch (err) {
        if (diag) {
            diag.transport = 'webview-fetch(fallback)';
            diag.nativeError = err?.message || String(err);
        }
        console.warn('[PoToken] native http_fetch failed, falling back to window.fetch:', err?.message || err);
    }
    if (diag && !diag.transport) diag.transport = 'webview-fetch';
    return window.fetch(input, init);
}

async function loadBgUtils(diag = null) {
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
        // Recorded per candidate. "which path did the WebView accept" is a real
        // question here — the first seven are correct only relative to the
        // importing module's depth, and a bundler or a different mount point
        // changes that — and a bare `catch (_) {}` per candidate is exactly how
        // a wrong-path import became indistinguishable from a missing module.
        const perCandidate = [];
        for (const p of candidates) {
            try {
                const mod = await import(p);
                if (mod) {
                    Object.assign(merged, mod);
                    // keep default export if present
                    if (mod.default) Object.assign(merged, mod.default);
                    loaded = true;
                    perCandidate.push(`${p}=ok`);
                } else {
                    perCandidate.push(`${p}=empty`);
                }
            } catch (e) {
                perCandidate.push(`${p}=${e?.name || 'err'}`);
            }
        }
        if (diag) {
            diag.perCandidate = perCandidate;
            diag.exports = {
                BotGuardClient: typeof merged.BotGuardClient,
                WebPoMinter: typeof merged.WebPoMinter,
                buildURL: typeof merged.buildURL,
                getHeaders: typeof merged.getHeaders,
                // NOTE: `createColdStartToken` is a *module-level* function in
                // core/WebPoMinter.js, not a static on the class. Recording the
                // static as `undefined` is what makes the dead
                // `bg.WebPoMinter?.createColdStartToken` branch below visible
                // instead of being an invisible guess about why it never fires.
                WebPoMinterCreateColdStartToken: typeof merged.WebPoMinter?.createColdStartToken,
                createColdStartToken: typeof merged.createColdStartToken,
            };
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
 * Returns { poToken, visitorData, contentBinding, report } or null on failure.
 * Caller should cache per videoId (TTL 6h, visitorData-bound) and pass to Innertube.create({ poToken, visitorData }).
 *
 * @param {object} innertube
 * @param {string} videoId
 * @param {object} [report] a `createMintReport()` from the caller. Accepted
 *   rather than created here so that one report spans the whole resolve —
 *   including the cache lookup and the `applyPoTokenToUrl` decision, which
 *   happen in `youtube.js`, not in here. Also retrievable afterwards through
 *   `getLastMintReport()`.
 */
export async function generatePoTokenForVideo(innertube, videoId, report = null) {
    const diag = report || createMintReport({ videoId });
    lastMintReport = diag;
    const t0 = Date.now();
    // Runs first and is synchronous, so the report always carries the WebView's
    // answer to "can you eval here?" even when the mint dies three steps later.
    // One probe per module load: it cannot change within a page session, and it
    // must not be re-run per resolve.
    if (!diag.page) recordPageProbe(diag, runPageContextProbe());
    try {
        const loadDiag = {};
        const bg = await loadBgUtils(loadDiag);
        const imported = Boolean(bg);
        recordStep(
            diag,
            'bgutils-import',
            imported,
            imported
                ? `vendored bgutils loaded`
                : 'every candidate import failed — no WebPoMinter, no BotGuardClient, so no Web token can be produced',
            { data: loadDiag.exports, ms: Date.now() - t0 }
        );
        if (!bg) {
            console.warn('[PoToken] BgUtils or getAttestationChallenge unavailable — skipping PO token');
            // `attempted`, not "not attempted": we ARE inside
            // generatePoTokenForVideo, so the resolver did try. The
            // `not-attempted` state is reserved for a mint that was never
            // entered at all (module missing, caller brought a token, cache
            // hit) — see `classifyMintOutcome`.
            recordMintOutcome(diag, 'failed');
            return null;
        }
        if (!innertube?.getAttestationChallenge) {
            recordStep(diag, 'attestation-challenge', false, 'innertube.getAttestationChallenge is not a function on the vendored youtubei session');
            console.warn('[PoToken] BgUtils or getAttestationChallenge unavailable — skipping PO token');
            recordMintOutcome(diag, 'failed');
            return null;
        }
        // Wire visitorData from innertube session first
        let visitorData = innertube.session?.context?.client?.visitorData
            || innertube.session?.context?.client?.visitor_data
            || null;

        // 1. Get challenge (ENGAGEMENT_TYPE_UNBOUND is used for GVS)
        const tChallenge = Date.now();
        const challengeResponse = await innertube.getAttestationChallenge('ENGAGEMENT_TYPE_UNBOUND').catch((e) => {
            console.warn('[PoToken] getAttestationChallenge failed:', e?.message || e);
            return null;
        });
        if (!challengeResponse?.bg_challenge) {
            recordStep(
                diag,
                'attestation-challenge',
                false,
                challengeResponse
                    ? 'challenge returned but carried no bg_challenge — the endpoint answered, and the answer had nothing to solve'
                    : 'getAttestationChallenge threw or resolved empty',
                { ms: Date.now() - tChallenge }
            );
            recordMintOutcome(diag, 'failed');
            console.warn('[PoToken] No bg_challenge in response');
            return null;
        }
        recordStep(diag, 'attestation-challenge', true, 'bg_challenge received', { ms: Date.now() - tChallenge });
        // visitorData may also be in challengeResponse (task requirement)
        const crVisitor = challengeResponse.visitorData || challengeResponse.visitor_data
            || challengeResponse.bg_challenge?.visitorData || challengeResponse.bg_challenge?.visitor_data || null;
        if (crVisitor) visitorData = visitorData || crVisitor;
        // Recorded on its own because it is independently failable *and*
        // consequential: a proof minted without the visitorData it is bound to
        // is rejected when it is presented, which looks like a random token
        // failure much later.
        recordStep(
            diag,
            'visitor-data',
            visitorData ? true : false,
            visitorData
                ? 'visitorData available for the token binding'
                : 'NO visitorData anywhere (session or challenge) — a proof minted now cannot be bound to this InnerTube session',
            { fatal: false }
        );
        diag.mint.visitorData = visitorData ? String(visitorData).slice(0, 16) : null;

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
            recordStep(diag, 'interpreter-url', false, 'bg_challenge carried no interpreter_url in any of the 5 wrapped spellings');
            recordMintOutcome(diag, 'failed');
            console.warn('[PoToken] No interpreter_url in bg_challenge');
            return null;
        }
        recordStep(diag, 'interpreter-url', true, `interpreter_url found (globalName=${globalName || 'default'})`);
        let scriptUrl = String(interpreterUrlRaw);
        if (scriptUrl.startsWith('//')) scriptUrl = 'https:' + scriptUrl;
        else if (!scriptUrl.startsWith('https://')) scriptUrl = 'https://' + scriptUrl.replace(/^https?:\/\//, '');

        let bgScriptResponse = null;
        // Transport is recorded because the two paths fail for unrelated
        // reasons and the symptom is identical: an allowlisted-away
        // `http_fetch` degrades to the WebView fetch, which then meets CORS.
        const fetchDiag = {};
        let nativeStatus = null;
        let nativeStatusText = '';
        try {
            const r = await nativeFetchPo(scriptUrl, { method: 'GET' }, fetchDiag);
            nativeStatus = r.status;
            nativeStatusText = r.statusText || '';
            if (!r.ok) {
                console.warn('[PoToken] interpreter fetch failed:', r.status, r.statusText);
            } else {
                bgScriptResponse = await r.text();
            }
        } catch (e) {
            fetchDiag.error = e?.message || String(e);
            console.warn('[PoToken] interpreter fetch error:', e?.message || e);
        }
        // Fallback to window.fetch if nativeFetch returned empty (allowlist blocked)
        let webviewStatus = null;
        if (!bgScriptResponse) {
            try {
                const r2 = await fetch(scriptUrl).then((r) => {
                    webviewStatus = r.status;
                    return r.text();
                }).catch((e) => {
                    fetchDiag.webviewError = e?.message || String(e);
                    return null;
                });
                bgScriptResponse = r2;
            } catch (_) {}
        }
        if (!bgScriptResponse) {
            recordStep(
                diag,
                'interpreter-fetch',
                false,
                `no interpreter bytes from either transport (${fetchDiag.transport || 'unknown'}${fetchDiag.error ? `, error="${fetchDiag.error}"` : ''}${fetchDiag.webviewError ? `, webview="${fetchDiag.webviewError}"` : ''})`,
                { data: { nativeStatus, webviewStatus } }
            );
            recordMintOutcome(diag, 'failed');
            console.warn('[PoToken] No bg script');
            return null;
        }
        recordStep(
            diag,
            'interpreter-fetch',
            true,
            `interpreter downloaded (${bgScriptResponse.length} bytes) via ${fetchDiag.transport || 'unknown'}`,
            { status: nativeStatus ?? webviewStatus, data: { nativeStatus, webviewStatus, bytes: bgScriptResponse.length } }
        );

        let poToken = null;
        // Which KIND of proof came out, if any. Load-bearing and not derivable
        // from the state: a cold-start token is built with no BotGuard involved
        // and only works while `sps` is 2, so a run that "succeeded" through one
        // has not demonstrated that BotGuard works in this WebView at all.
        // Declared here because the direct-mint path below also assigns it.
        let proofKind = 'webpo';

        // Attempt high-level API if present (defensive)
        if (typeof bg.generatePoToken === 'function') {
            try {
                const res = await bg.generatePoToken({ innertube, videoId, bgScript: bgScriptResponse, challenge: bgCh });
                if (res?.poToken) poToken = res.poToken;
                if (res?.visitorData) visitorData = res.visitorData || visitorData;
                recordStep(diag, 'mint', poToken ? true : null, poToken ? 'high-level bg.generatePoToken produced a proof' : 'high-level bg.generatePoToken returned no proof (falling through to the low-level path)');
            } catch (e) {
                recordStep(diag, 'mint', null, `high-level bg.generatePoToken threw (${e?.message || e}) — falling through to the low-level path`, { fatal: false, error: e?.message || String(e) });
                console.warn('[PoToken] generatePoToken high-level failed:', e?.message || e);
            }
        }

        // Low-level: BotGuardClient + WebPoMinter (bgutils 4.x)
        if (!poToken && bg.BotGuardClient && bg.WebPoMinter) {
            try {
                // Evaluate bg script to populate global object (required for BotGuardClient)
                const gName = globalName || 'botguard';
                const gObj = globalThis;
                // Whether the eval is *needed* is a distinct question from whether
                // it *works*, and the old `if (gName && !gObj[gName] && ...)` guard
                // made the two indistinguishable: a global left behind by an
                // earlier resolve looked exactly like a successful eval here.
                const preExisting = gName ? Boolean(gObj[gName]) : false;
                if (gName && !gObj[gName] && bgScriptResponse) {
                    const tEval = Date.now();
                    try {
                        // Execute script in global scope; CSP requires unsafe-eval (allowed in tauri.conf.json)
                        const fn = new Function(bgScriptResponse);
                        fn();
                        recordStep(diag, 'new-function-eval', true, `compiled and ran ${bgScriptResponse.length} bytes of interpreter in ${Date.now() - tEval}ms`, { ms: Date.now() - tEval });
                    } catch (e) {
                        // A WebView whose CSP lacks 'unsafe-eval' lands here and
                        // nowhere else in the entire mint path.
                        recordStep(diag, 'new-function-eval', false, `new Function(interpreter) threw: ${e?.message || e} — if this mentions CSP or unsafe-eval, tauri.conf.json security.csp script-src is the cause`, { error: e?.message || String(e), ms: Date.now() - tEval });
                        console.warn('[PoToken] bg script eval failed (non-fatal):', e?.message || e);
                    }
                } else {
                    recordStep(diag, 'new-function-eval', null, preExisting ? `skipped: globalThis.${gName} already exists from an earlier resolve, so the eval never ran` : 'skipped: no interpreter bytes to evaluate');
                }
                // The decisive measurement the eval's own success does not make.
                // BotGuardClient.js:22 reads globalObject[globalName] and
                // BotGuardClient.js:34-37 throws `EGOU: BotGuard unavailable` on
                // a falsy vm. A `var`-declaring body executed through
                // `new Function(body)` binds into the function's own scope, so
                // the eval can succeed and leave nothing here.
                const vm = gName ? gObj[gName] : undefined;
                recordStep(
                    diag,
                    'botguard-global',
                    vm && typeof vm.a === 'function' ? true : false,
                    vm
                        ? `globalThis.${gName} exists; .a is ${typeof vm?.a}${typeof vm?.a === 'function' ? '' : ' (BotGuardClient needs a function here)'}`
                        : `globalThis.${gName} is ${vm === undefined ? 'undefined' : String(vm)} after eval — BotGuardClient will throw EGOU: BotGuard unavailable`,
                );

                // Create BotGuardClient
                const tLoad = Date.now();
                let clientCreateError = null;
                const botGuard = await bg.BotGuardClient.create({
                    program: program || bgScriptResponse,
                    globalName: gName || 'botguard',
                    globalObject: gObj,
                }).catch((e) => {
                    clientCreateError = e?.message || String(e);
                    console.warn('[PoToken] BotGuardClient.create failed:', clientCreateError);
                    return null;
                });
                // NOTE a behaviour change, and the only one: this used to be
                // `.catch(() => new bg.BotGuardClient({...}))`. The constructor
                // cannot succeed where `create` failed (both need the same
                // globalName/globalObject/program triple, and `create` is just
                // `new` + `load`), so the fallback constructor produced an
                // object whose `load()` was then swallowed by the empty catch
                // below — turning a precise `EGOU` into a 3-second
                // `VM operation timed out` that named neither the step nor the
                // cause. `load()` is still called on the fallback path, so
                // nothing downstream is lost.
                const client = botGuard || (() => {
                    try { return new bg.BotGuardClient({ program: program || bgScriptResponse, globalName: gName || 'botguard', globalObject: gObj }); }
                    catch (e) { clientCreateError = `${clientCreateError || 'create failed'}; constructor failed: ${e?.message || e}`; return null; }
                })();

                let loadError = null;
                if (client && typeof client.load === 'function') {
                    // Swallowed until 2026-09-27. This is the error that says
                    // whether the WebView's global is usable at all, and it was
                    // the single most expensive silence in this file.
                    try { await client.load(); } catch (e) { loadError = e?.message || String(e); }
                } else {
                    loadError = clientCreateError || 'no BotGuardClient instance could be constructed';
                }
                recordStep(
                    diag,
                    'botguard-load',
                    loadError ? false : true,
                    loadError
                        ? `BotGuardClient.load failed: ${loadError}`
                        : 'VM handshake returned a snapshot function',
                    { error: loadError, ms: Date.now() - tLoad }
                );
                if (loadError) {
                    // Deliberately NOT returning. Control flow is unchanged: the
                    // run still goes on to `snapshot` (which will hit its 3s
                    // timeout, BotGuardClient.js:131) and then to the cold-start
                    // token, and those are what decide the outcome. The
                    // difference is that the report now says why.
                    console.warn('[PoToken] BotGuard load failed:', loadError);
                }

                // Snapshot with webPoSignalOutput to obtain minter factory.
                //
                // ── The call order here is the contract's order, and it is
                // deliberately NOT being changed. The obvious repair for "the
                // VM pushed no factory" is to fetch the GenerateIT integrity
                // token first and try again. That is not available: the
                // GenerateIT body is the protobuf pair
                // `[requestKey, botguardResponse]` built just below from
                // `botguardResponse`, and `WebPoMinter.create` hands the token
                // to the factory as its *argument*
                // (ui/vendor/bgutils/core/WebPoMinter.js:21). The token is
                // derived from this response and consumed after the factory
                // exists, so there are no bytes to send before the snapshot.
                const webPoSignalOutput = [];
                let botguardResponse = null;
                let snapshotError = null;
                let snapshotUnsupported = false;
                const tSnap = Date.now();
                try {
                    // Try snapshot with webPoSignalOutput (required for WebPoMinter)
                    if (typeof client?.snapshot === 'function') {
                        botguardResponse = await client.snapshot({ webPoSignalOutput });
                    } else if (typeof client?.snapshotSynchronous === 'function') {
                        botguardResponse = await client.snapshotSynchronous({ webPoSignalOutput });
                    } else {
                        snapshotUnsupported = true;
                        snapshotError = 'client exposes neither snapshot nor snapshotSynchronous';
                    }
                } catch (e) {
                    snapshotError = e?.message || String(e);
                    console.warn('[PoToken] BotGuard snapshot failed:', snapshotError);
                }
                // ── EXPERIMENT, not a fix ──────────────────────────────────────
                //
                // `webPoSignalOutput` is handed to the VM **by reference**
                // (BotGuardClient.js:152-157) while `snapshot()` resolves its
                // own promise from a *callback the VM invokes*
                // (BotGuardClient.js:152, `(response) => resolve(response)`).
                // Nothing in bgutils establishes that the VM's push happens
                // before it calls that callback. Reading `.length` on the very
                // next line therefore races the write, and a race is the one
                // explanation here that is both cheap to rule out and free of
                // speculation: if the factory lands late, this finds it; if it
                // does not, "the VM pushed nothing" is established rather than
                // assumed.
                //
                // Bounded, and only ever entered on the path that has already
                // failed, so it cannot slow a working mint down or change a
                // successful outcome. Set SNAPSHOT_SETTLE_MS to 0 to disable.
                const settle = await settleWebPoSignalOutput(webPoSignalOutput, {
                    snapshotError,
                    unsupported: snapshotUnsupported,
                });
                const snap = classifySnapshotOutcome({
                    webPoSignalOutput,
                    botguardResponse,
                    snapshotError,
                    unsupported: snapshotUnsupported,
                    settle,
                });
                recordStep(
                    diag,
                    'snapshot',
                    snap.ok,
                    snap.detail,
                    // `shape` and the measurements ride into the entry under
                    // `data`, which `stepDetail` renders — so they reach the
                    // copied report, not just this module's internals. The whole
                    // purpose of the taxonomy is that a person reading a phone
                    // screenshot can tell two runs apart.
                    { shape: snap.shape, data: { ...snap.facts }, note: snap.note, error: snapshotError, ms: Date.now() - tSnap }
                );

                // Gated on the classification rather than on
                // `webPoSignalOutput.length && botguardResponse`: length>0 with
                // a non-function at [0] is not a minter factory, and letting it
                // through only moved the failure to the `mint` step where it
                // was reported as the wrong stage.
                if (snap.shape === SNAPSHOT_SHAPES.OK) {
                    // Fetch integrity token via bgutils helpers (BgUtils example) — proper protobuf encoding via buildURL/getHeaders, avoids 400
                    // Example payload is [requestKey, botguardResponse] where requestKey is 'O43z0dpjhgX20SCx4KAo' (same in both examples)
                    const REQUEST_KEY = 'O43z0dpjhgX20SCx4KAo';
                    const payload = [REQUEST_KEY, botguardResponse];
                    const hasHelpers = typeof bg.buildURL === 'function' && typeof bg.getHeaders === 'function';
                    const itUrl = hasHelpers ? bg.buildURL('GenerateIT') : 'https://jnn-pa.googleapis.com/$rpc/google.internal.waa.v1.Waa/GenerateIT';
                    const itHeaders = hasHelpers ? bg.getHeaders() : { 'content-type': 'application/json+protobuf', 'x-goog-api-key': 'AIzaSyDyT5W0Jh49F30Pqqtyfdf7pDLFKLJoAnw', 'x-user-agent': 'grpc-web-javascript/0.1' };
                    const tIt = Date.now();
                    const itDiag = {};
                    let itResp = null;
                    let integrityToken = null;
                    let estimatedTtlSecs = null;
                    let mintRefreshThreshold = null;
                    let websafeFallbackToken = null;
                    try {
                        itResp = await nativeFetchPo(itUrl, {
                            method: 'POST',
                            headers: itHeaders,
                            body: JSON.stringify(payload),
                        }, itDiag);
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
                            integrityToken = Array.isArray(json) ? json[0] : json?.integrityToken || json?.integrity_token;
                            estimatedTtlSecs = Array.isArray(json) ? json[1] : json?.estimatedTtlSecs;
                            mintRefreshThreshold = Array.isArray(json) ? json[2] : json?.mintRefreshThreshold;
                            websafeFallbackToken = Array.isArray(json) ? json[3] : json?.websafeFallbackToken;
                            if (!integrityToken) {
                                recordStep(diag, 'generate-it', false, 'GenerateIT answered ok but carried no integrityToken', { status: itResp?.status, ms: Date.now() - tIt });
                                console.warn('[PoToken] GenerateIT empty integrityToken');
                            } else {
                                recordStep(diag, 'generate-it', true, `integrityToken received (ttl=${estimatedTtlSecs ?? '?'}s, refresh=${mintRefreshThreshold ?? '?'})`, { status: itResp?.status, endpoint: itDiag.transport, fatal: false, ms: Date.now() - tIt });
                            }
                        } else {
                            recordStep(diag, 'generate-it', false, `GenerateIT refused the request (status ${itResp?.status ?? 'no response'} ${itResp?.statusText || ''})`.trim(), { status: itResp?.status, endpoint: itDiag.transport, ms: Date.now() - tIt });
                            console.warn('[PoToken] GenerateIT bad status:', itResp?.status, itResp?.statusText);
                        }
                    } catch (e) {
                        recordStep(diag, 'generate-it', false, `GenerateIT request threw: ${e?.message || e}`, { error: e?.message || String(e), endpoint: itDiag.transport, ms: Date.now() - tIt });
                        console.warn('[PoToken] GenerateIT failed:', e?.message || e);
                    }
                    if (integrityToken) {
                        const tMint = Date.now();
                        try {
                            const integrityTokenData = { integrityToken, estimatedTtlSecs, mintRefreshThreshold, websafeFallbackToken };
                            const minter = await bg.WebPoMinter.create(integrityTokenData, webPoSignalOutput);
                            // WebPoMinter.mint as per BgUtils example (contentBinding = videoId, visitorData-bound)
                            poToken = await minter.mintAsWebsafeString(videoId);
                            recordStep(diag, 'mint', true, `WebPoMinter produced a ${String(poToken).length}-char websafe proof`, { ms: Date.now() - tMint });
                        } catch (e) {
                            // bgutils names its own failure modes here and they
                            // say which stage of BotGuard refused: PMD:Undefined
                            // (no factory), APF:Failed (factory returned a
                            // non-Function), YNJ:Undefined (empty proof),
                            // ODM:Invalid (proof not a Uint8Array).
                            recordStep(diag, 'mint', false, `WebPoMinter failed: ${e?.message || e}`, { error: e?.message || String(e), ms: Date.now() - tMint });
                            console.warn('[PoToken] WebPoMinter mint failed:', e?.message || e);
                        }
                    }
                }

                // Fallback: direct mint if snapshot gave us minter without GenerateIT (some bgutils builds)
                //
                // `typeof === 'function'`, not truthiness. A truthy non-function
                // at [0] is exactly the `non-function` snapshot shape, and
                // calling it produced a `TypeError: getMinter is not a function`
                // that was then recorded as a *mint* failure — naming the wrong
                // stage for a problem the snapshot step had already measured.
                if (!poToken && typeof webPoSignalOutput?.[0] === 'function') {
                    const tDirect = Date.now();
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
                        if (poToken) {
                            // Recorded as its own kind because it is the weakest
                            // proof this file can produce: `getMinter` was called
                            // with an EMPTY integrity token, which
                            // `WebPoMinter.create` explicitly refuses to do
                            // (WebPoMinter.js:19-20). A proof minted without one
                            // is expected to be refused by the edge, and
                            // "outcome=succeeded" alone would hide that.
                            proofKind = 'webpo-direct';
                        }
                        recordStep(diag, 'mint', poToken ? true : null, poToken
                            ? `direct mint callback produced a ${poToken.length}-char proof with NO GenerateIT integrity token — expected to be refused by the edge`
                            : 'direct mint callback ran but returned no Uint8Array', { fatal: false, ms: Date.now() - tDirect });
                    } catch (e) {
                        recordStep(diag, 'mint', false, `direct mint callback threw: ${e?.message || e}`, { error: e?.message || String(e), ms: Date.now() - tDirect, fatal: false });
                    }
                }
            } catch (e) {
                recordStep(diag, 'botguard-load', false, `BotGuard/WebPoMinter stage threw: ${e?.message || e}`, { error: e?.message || String(e) });
                console.warn('[PoToken] BotGuard/WebPoMinter mint failed:', e?.message || e);
            }
        } else if (!poToken) {
            recordStep(
                diag,
                'botguard-load',
                false,
                `bgutils exports do not include both BotGuardClient (${typeof bg.BotGuardClient}) and WebPoMinter (${typeof bg.WebPoMinter}) — the low-level path cannot run`,
            );
        }

        // Cold-start token fallback via bgutils helper (no BotGuard needed, works when sps=2)
        if (!poToken && bg.WebPoMinter?.createColdStartToken) {
            // Unreachable in practice: `createColdStartToken` is a module-level
            // function in core/WebPoMinter.js, not a static on the class, so
            // `WebPoMinter.createColdStartToken` is always undefined. Left as-is
            // (it is a correct guard for a future bgutils that does attach it),
            // and `bgutils-import` records the static's type so the dead branch
            // is visible in the report rather than a guess.
            try {
                poToken = bg.createColdStartToken ? bg.createColdStartToken(videoId) : bg.WebPoMinter.createColdStartToken(videoId);
                proofKind = 'cold-start';
                recordStep(diag, 'cold-start-fallback', true, `cold-start token produced (${String(poToken).length} chars) — no BotGuard attestation was involved`);
                console.log('[PoToken] Using cold-start token for', videoId);
            } catch (e) {
                recordStep(diag, 'cold-start-fallback', false, `cold-start mint failed: ${e?.message || e}`, { error: e?.message || String(e) });
                console.warn('[PoToken] cold-start mint failed:', e?.message || e);
            }
        } else if (!poToken && bg.createColdStartToken) {
            try {
                poToken = bg.createColdStartToken(videoId);
                proofKind = 'cold-start';
                recordStep(diag, 'cold-start-fallback', true, `cold-start token produced (${String(poToken).length} chars) — no BotGuard involved`);
            } catch (e) {
                recordStep(diag, 'cold-start-fallback', false, `cold-start mint failed: ${e?.message || e}`, { error: e?.message || String(e) });
            }
        } else if (!poToken) {
            recordStep(diag, 'cold-start-fallback', false, `no cold-start helper in the bgutils export (createColdStartToken=${typeof bg.createColdStartToken}) — the last fallback before giving up`);
        }

        if (!poToken) {
            console.warn('[PoToken] Failed to mint token for', videoId, '— will fallback to TV/ANDROID_VR');
            // A mint DID run here, so this is `attempted-failed` and must not be
            // downgraded to "not attempted" — the difference decides whether the
            // owner should go looking at the WebView or at their Settings.
            recordMintOutcome(diag, 'failed');
            if (!diag.mint.failedAt) {
                // Every individual step said "skipped", not "failed", so
                // `failedAt` is empty. Name the run rather than leave the owner
                // with a failure that has no step.
                recordStep(diag, 'mint', false, 'no step reported a failure, yet no proof was produced — something returned empty without recording (look for an un-instrumented early return)');
            }
            return null;
        }

        // `proofKind` is load-bearing and is NOT derivable from the state. A
        // cold-start token is produced by bgutils with no BotGuard involved and
        // only works while `sps` is 2, so a run that "succeeded" through it has
        // not demonstrated that BotGuard works at all. Both are Web-family
        // scoped, so both are `tokenSource: minted`; the kind is the difference.
        recordMintOutcome(diag, 'succeeded', { token: poToken, visitorData, source: 'minted' });
        diag.mint.proofKind = proofKind;
        console.log(`[PoToken] Minted for ${videoId}: ${poToken.slice(0, 20)}… (visitorData ${visitorData ? visitorData.slice(0, 12) + '…' : 'none'})`);
        return { poToken, visitorData, contentBinding: videoId, proofKind, report: diag };
    } catch (e) {
        console.warn('[PoToken] generatePoTokenForVideo error:', e?.message || e);
        recordMintOutcome(diag, 'failed');
        recordStep(diag, 'mint', false, `generatePoTokenForVideo threw: ${e?.message || e}`, { error: e?.message || String(e) });
        return null;
    }
}

// Simple in-memory cache (TTL 6h) — visitorData-bound key avoids cross-visitor poisoning
const poCache = new Map(); // key: videoId::visitorData -> { poToken, visitorData, contentBinding, proofKind, expires }

function cacheKey(videoId, visitorData) {
    return visitorData ? `${videoId}::${visitorData}` : videoId;
}

/**
 * Look up a cached proof.
 *
 * Records the `cache-read` step into `lastMintReport`, creating one if this
 * lookup is the first thing to happen. That is why the step lives here and not
 * in `youtube.js`: the caller drives the lookup itself (it needs the
 * visitorData first), and a cache hit is otherwise completely invisible — the
 * resolver logs "Using cached PO token" and moves on, so a run that was served
 * a 6-hour-old proof is indistinguishable in the report from a run that minted
 * one, and from a run that minted nothing at all.
 *
 * @param {string} videoId
 * @param {string|null} [visitorData]
 * @param {object} [report] an existing report to record into
 */
export function getCachedPoToken(videoId, visitorData = null, report = null) {
    const diag = report || lastMintReport || createMintReport({ videoId });
    lastMintReport = diag;
    let hit = null;
    let how = 'miss';

    // Exact match first
    if (visitorData) {
        const k = cacheKey(videoId, visitorData);
        const entry = poCache.get(k);
        if (entry && Date.now() <= entry.expires) { hit = entry; how = 'exact(visitorData-bound)'; }
        else if (entry) poCache.delete(k);
    }
    // Fallback: plain videoId (backward compat) or prefix scan
    if (!hit) {
        const plain = poCache.get(videoId);
        if (plain && Date.now() <= plain.expires) { hit = plain; how = 'plain-videoId'; }
        else if (plain) poCache.delete(videoId);
    }
    // Scan for any visitorData-bound entry for this videoId (when caller didn't provide visitorData)
    if (!hit && !visitorData) {
        for (const [k, v] of poCache.entries()) {
            if (k.startsWith(videoId + '::')) {
                if (Date.now() > v.expires) { poCache.delete(k); continue; }
                hit = v;
                how = 'prefix-scan(any visitorData)';
                break;
            }
        }
    }

    if (hit) {
        recordStep(diag, 'cache-read', true, `hit via ${how}, minted earlier and still within the 6h TTL`, {
            data: { expiresInMs: Math.max(0, hit.expires - Date.now()), proofKind: hit.proofKind || 'unknown' },
            fatal: false,
        });
        diag.mint.visitorData = hit.visitorData ? String(hit.visitorData).slice(0, 16) : diag.mint.visitorData;
        diag.mint.tokenLen = hit.poToken ? String(hit.poToken).length : null;
        diag.mint.tokenPreview = hit.poToken ? String(hit.poToken).slice(0, 16) : null;
        // WHICH KIND of proof is now in play, on a run where nothing was minted.
        //
        // The cache deliberately suppresses the mint for 6h, and that decision
        // is NOT being changed here — a cold-start proof is the only kind we
        // currently produce, cold-start is accepted while `sps` is 2, and
        // forcing a re-mint on every resolve would buy a 63KB interpreter
        // download and three round trips per video in exchange for a WebPO path
        // that has never once succeeded in this WebView. Suppressing retries is
        // exactly why the *observability* had to change instead: a cache hit was
        // rendering identically whether it served a cold-start token or a
        // BotGuard one, so six hours of resolves could not tell the owner that
        // the only token they have is the one that needs no attestation.
        diag.mint.proofKind = hit.proofKind || null;
        // No mint ran this time round, but we ARE holding a Web-bound token —
        // and that is the whole reason `classifyMintOutcome` looks at
        // `tokenSource` before `attempted`.
        diag.mint.tokenSource = 'cache';
    } else {
        recordStep(diag, 'cache-read', null, 'no live entry — a mint will be attempted', { fatal: false });
    }
    return hit;
}

/**
 * Store a proof. Records the `cache-write` step, which is how a report says
 * "this proof is now reusable for 6h, so the next resolve will show a cache
 * hit rather than a fresh mint".
 */
export function setCachedPoToken(videoId, data, report = null) {
    const diag = report || lastMintReport;
    const vd = data?.visitorData || null;
    const k = cacheKey(videoId, vd);
    // `proofKind` rides along with the entry. A token is not self-describing: a
    // cold-start proof and a BotGuard proof are both base64url strings of
    // similar length, and the only thing that distinguishes them is which branch
    // of the mint produced it — which is exactly the fact that a cache hit
    // would otherwise erase. See the note in `getCachedPoToken`.
    const proofKind = data?.proofKind || diag?.mint?.proofKind || null;
    const entry = { ...data, proofKind, expires: Date.now() + 6 * 60 * 60 * 1000 };
    poCache.set(k, entry);
    // Also store under plain key for backward compat callers that don't pass visitorData
    if (vd) poCache.set(videoId, entry);
    if (diag) {
        recordStep(diag, 'cache-write', true, `stored under ${k} (and under the plain videoId key) with a 6h TTL`, { fatal: false });
    }
    return diag || null;
}
