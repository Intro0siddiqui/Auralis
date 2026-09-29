/**
 * po_diagnostics.js — *what happened* when we tried to mint a PO token.
 *
 * ── Why this file exists ────────────────────────────────────────────────────
 *
 * Nobody knows whether the WebView can mint a YouTube PO token, and if it
 * cannot, which of the seven-ish independent things in `po_token.js` is the one
 * that fails. That matters a great deal right now: the web client family
 * (`MWEB`/`WEB`) is the only family in the rotation that is *not* SABR-flagged,
 * and on the owner's residential Jio line it returns zero formats — which is
 * what yt-dlp's PO-Token Guide predicts for a client that requires a GVS token
 * we have never successfully minted. If the WebView can mint, the web family may
 * start resolving, and that is the only route to non-SABR urls we have.
 *
 * Two things made the answer unreachable before:
 *
 *  1. **Release builds write nothing to logcat.** Every failure in the mint path
 *     was a `console.warn`, so the one environment that matters produced no
 *     evidence at all. This output is designed to ride the channel the project
 *     already solved that problem with: the per-client `clientReport` that
 *     `youtube.js` builds, `downloads.js:_recordClientReport` archives in
 *     `window.__auralisClientReports`, the download row renders as a
 *     `clients: …` line, and the **Copy report** button hands over.
 *  2. **The steps are not independent, so "it failed" is not a finding.** See
 *     the honest-step list below. Notably `new Function` *succeeding* does not
 *     imply minting works, and the two lines that decide that (`po_token.js`
 *     `new Function` and `globalThis[gName]`) were silent in both directions.
 *
 * ── The four states this must never confuse ─────────────────────────────────
 *
 *     not-attempted      no mint ran (caller supplied a token, or the mint
 *                        module could not be imported, or a cache hit served us)
 *     attempted-failed   a mint ran and died at a named step
 *     minted-attached   we minted a Web/BotGuard token and it rode the URL
 *     minted-stripped   we minted one and deliberately did NOT put it on the URL
 *
 * The last is the one that gets misread as "minting is broken". It is not: a PO
 * token is platform-bound, and the only token we can mint is a Web/BotGuard one,
 * so `pot_scope.js` withholds it whenever the winning client is not web-family.
 * On the 2026-09-27 device run the winner was `IOS`, so a perfectly good token
 * would have been withheld — and "no token on the URL" is exactly what a failed
 * mint also looks like. Hence a first-class state.
 *
 * ── Deliberately dependency-free ─────────────────────────────────────────────
 *
 * No bgutils import, no `window` access at module scope, no side effects. It is
 * imported by `po_token.js` (which may itself be the thing that fails) and by
 * `youtube.js` (which must be able to report "the mint module would not load"),
 * so it has to be the one piece of this system that cannot fail to load. That
 * also makes it directly unit-testable under node — see
 * `scripts/tests/po_diagnostics.test.js`.
 */

// ── the step list ────────────────────────────────────────────────────────────
//
// Derived by reading the code, not imagined. Every entry names the place it
// comes from so a reader can check it. The order is the order the code runs
// them in, which is what lets `formatMintReport` print `- not reached` for
// everything downstream of the first failure — the single most useful thing in
// the report, because it names the step that actually matters instead of the
// pile of steps that merely inherited the failure.

export const MINT_STEPS = [
    // Cheap, synchronous, runs before any network. Recorded first so that a
    // report always carries the WebView's answer to "can you even eval here?",
    // even when the mint dies three steps later.
    { id: 'page-context-probe', label: 'page context: new Function + globals' },
    // po_token.js:49 loadBgUtils — dynamic import of the vendored bgutils ESM.
    { id: 'bgutils-import', label: 'load vendored bgutils modules' },
    // po_token.js:352 getCachedPoToken — visitorData-bound, 6h TTL.
    { id: 'cache-read', label: 'visitorData-bound cache lookup' },
    // po_token.js:116 innertube.getAttestationChallenge('ENGAGEMENT_TYPE_UNBOUND').
    { id: 'attestation-challenge', label: 'InnerTube attestation challenge' },
    // po_token.js:111-127. Recorded separately because it is independently
    // failable AND consequential: a token minted without the visitorData it was
    // bound to is rejected by InnerTube, so "minted" here is not enough.
    { id: 'visitor-data', label: 'visitorData for the token binding' },
    // po_token.js:131-148 — pull interpreter_url out of bg_challenge and
    // normalise the scheme-protocol-relative form.
    { id: 'interpreter-url', label: 'extract interpreter_url' },
    // po_token.js:150-171 — nativeFetchPo (Rust http_fetch) then window.fetch.
    // `transport` is recorded because the two have different failure modes: a
    // blocked/allowlisted `http_fetch` silently degrades to the WebView fetch,
    // which then runs into CORS.
    { id: 'interpreter-fetch', label: 'fetch the BotGuard interpreter' },
    // po_token.js:195 — `new Function(bgScriptResponse); fn()`. The decisive
    // line for the CSP question. A WebView with `unsafe-eval` stripped fails
    // here and nowhere else, and this used to be a bare `console.warn`.
    { id: 'new-function-eval', label: 'new Function(interpreter) + call' },
    // po_token.js:191-192 + ui/vendor/bgutils/core/BotGuardClient.js:22,34-37.
    //
    // THIS IS THE STEP `new-function-eval` DOES NOT COVER, and it is the reason
    // "the eval worked" is not an answer. `BotGuardClient` reads
    // `options.globalObject[options.globalName]` and throws
    // `EGOU: BotGuard unavailable` if it is falsy. A `var`-declaring script run
    // through `new Function(body)` declares into the *function's* scope, not
    // globalThis — so the eval can return cleanly and leave nothing behind for
    // BotGuardClient to find. Whether Google's interpreter blob assigns or
    // declares is a property of a script we cannot read until we download it,
    // so it is measured here rather than argued about.
    { id: 'botguard-global', label: 'globalThis[globalName].a reachable' },
    // po_token.js:202-210. `BotGuardClient.create` → `load()` → `vm.a(program,…)`.
    // The `load()` failure was swallowed by a bare `catch (_) {}`, and its only
    // other consequence was a 3s `VM operation timed out` race in `snapshot`
    // (BotGuardClient.js:131) whose message names neither the cause nor the
    // step. That is the failure this step exists to name.
    { id: 'botguard-load', label: 'BotGuardClient.load (VM handshake)' },
    // po_token.js:212-224 — `snapshot({ webPoSignalOutput })`, which is where
    // the minter factory is obtained. This is the step the 2026-09-29 device run
    // died on, and it used to report every way of failing as the same sentence.
    // `SNAPSHOT_SHAPES` is now the taxonomy; the entry carries `shape` plus the
    // array's measured length and per-element types, so a reader can tell
    // "pushed nothing" from "pushed a non-function" from "never answered".
    { id: 'snapshot', label: 'BotGuard snapshot (minter factory)' },
    // po_token.js:226-270 — POST GenerateIT to jnn-pa (or www.youtube.com
    // fallback) for the integrity token. `endpoint` records which one answered.
    { id: 'generate-it', label: 'GenerateIT integrity token (jnn-pa)' },
    // po_token.js:262-264 — `WebPoMinter.create` + `mintAsWebsafeString(videoId)`.
    // bgutils surfaces its own coded failures here: PMD:Undefined (no factory),
    // APF:Failed (factory returned a non-Function), YNJ:Undefined (empty proof),
    // ODM:Invalid (proof not a Uint8Array). They are recorded verbatim because
    // the code alone says which stage of BotGuard refused.
    { id: 'mint', label: 'WebPoMinter.mintAsWebsafeString' },
    // po_token.js:296-306 — cold-start token, used only when the BotGuard path
    // produced nothing. A different kind of token for the same purpose.
    { id: 'cold-start-fallback', label: 'cold-start token fallback' },
    // po_token.js:352 setCachedPoToken / 351 getCachedPoToken.
    { id: 'cache-write', label: 'write visitorData-bound cache' },
    // ui/js/modules/pot_scope.js applyPoTokenToUrl, called from youtube.js after
    // a format is chosen. This is where `minted-attached` vs `minted-stripped`
    // is decided, and it runs in a different function from the mint, so it is a
    // separate step with its own entry.
    //
    // `fatal: false` because a withhold here is NOT a mint failure. If this
    // were allowed to set `failedAt`, a perfectly good token deliberately kept
    // off an `ios` url would be reported as "minting failed at pot-apply" —
    // the exact misreading this file exists to prevent. `consequence: true`
    // for the same reason: the report renders it `held`, not `FAIL`.
    { id: 'pot-apply', label: 'applyPoTokenToUrl on the stream url', fatal: false, consequence: true },
];

/** Steps whose failure means the mint itself failed (see `pot-apply`). */
const FATAL_BY_DEFAULT = MINT_STEPS.filter((s) => s.fatal !== false).map((s) => s.id);

/**
 * Steps where a `false` is the *intended outcome* rather than a defect. Only
 * `pot-apply` qualifies: withholding a Web token from a non-web client is the
 * whole point of `pot_scope.js`. It is marked so the report can say `held`
 * instead of `FAIL`, because a red `FAIL` next to `outcome=succeeded` is a
 * contradiction, and a reader who has to work that out will believe one of the
 * two fields is broken.
 */
const CONSEQUENCE_STEPS = new Set(MINT_STEPS.filter((s) => s.consequence === true).map((s) => s.id));

const STEP_IDS = new Set(MINT_STEPS.map((s) => s.id));
const STEP_INDEX = new Map(MINT_STEPS.map((s, i) => [s.id, i]));

/** The step ids, in execution order. */
export const MINT_STEP_IDS = MINT_STEPS.map((s) => s.id);

export const MINT_STATES = {
    /** No mint was attempted and none of our tokens is in play. */
    NOT_ATTEMPTED: 'not-attempted',
    /** A mint ran and died; `report.mint.failedAt` names the step. */
    ATTEMPTED_FAILED: 'attempted-failed',
    /**
     * We hold a Web-bound token and it is on the URL.
     */
    MINTED_ATTACHED: 'minted-attached',
    /**
     * We hold a Web-bound token and `applyPoTokenToUrl` deliberately did not put
     * it on the URL. Almost always correct behaviour: the winning client was not
     * web-family, so `pot_scope.js` withheld the token (a Web token on an `ios`
     * URL is what produced the byte-0 403 in v2.6.50). Never read this as
     * "minting failed" — check `report.mint.outcome`, which is `succeeded` here
     * by definition.
     */
    MINTED_STRIPPED: 'minted-stripped',
    /**
     * We hold a Web-bound token and the URL step never ran, so whether it would
     * have travelled is undetermined.
     *
     * This state exists because folding it into `minted-stripped` is a lie with
     * consequences: `minted-stripped` reads as "we chose to withhold", which
     * points the owner at `pot_scope.js` when the real cause is that the resolve
     * died earlier (no format, no usable stream, every client UNPLAYABLE).
     */
    MINTED_NOT_EVALUATED: 'minted-not-evaluated',
    /** The caller (Settings) supplied the token. We minted nothing and strip nothing. */
    USER_TOKEN: 'user-token',
    /** The report is malformed; we cannot say. Recorded rather than guessed. */
    UNKNOWN: 'unknown',
};

/**
 * How a `applyPoTokenToUrl` action maps onto "did the token travel".
 *
 * Deliberately two-valued. The question the owner needs answered is binary —
 * is there a `pot=` on the url or is there not — and any third bucket would
 * have to distinguish "we don't know" from "we know and it was withheld", which
 * is a distinction the *raw action* in the report already carries. Getting this
 * wrong in the permissive direction is what would produce the dangerous lie: a
 * resolve that failed before the url step ever ran reporting
 * `minted-stripped`, which reads as a deliberate withhold.
 */
export const POT_PLACEMENT = {
    ATTACHED: 'attached',
    NOT_ATTACHED: 'not-attached',
};

/** `applyPoTokenToUrl` actions that mean "there is a pot on the url". */
const ATTACHED_ACTIONS = new Set(['attached', 'already-present']);

/**
 * @param {string|null|undefined} action an `applyPoTokenToUrl` action
 * @returns {'attached'|'not-attached'}
 */
export function placementFromPotAction(action) {
    return ATTACHED_ACTIONS.has(String(action || '')) ? POT_PLACEMENT.ATTACHED : POT_PLACEMENT.NOT_ATTACHED;
}

// ── report construction / recording ───────────────────────────────────────────

/**
 * A fresh, empty mint report.
 *
 * @param {{videoId?: string|null, tokenSource?: 'minted'|'cache'|'user'|'none',
 *          notAttemptedReason?: string}} [init]
 */
export function createMintReport(init = {}) {
    return {
        at: new Date().toISOString(),
        videoId: init.videoId || null,
        steps: [],
        /** @type {{attempted: boolean|null, outcome: 'succeeded'|'failed'|null,
         *          failedAt: string|null, failedReason: string|null,
         *          tokenSource: 'minted'|'cache'|'user'|'none',
         *          proofKind: 'webpo'|'cold-start'|null,
         *          tokenLen: number|null, tokenPreview: string|null,
         *          visitorData: string|null, notAttemptedReason: string|null}} */
        mint: {
            attempted: null,
            outcome: null,
            failedAt: null,
            failedReason: null,
            tokenSource: init.tokenSource || 'none',
            proofKind: null,
            tokenLen: null,
            tokenPreview: null,
            visitorData: null,
            notAttemptedReason: init.notAttemptedReason || null,
        },
        /** `applyPoTokenToUrl`'s verdict: `{action, winningClient, detail}`. */
        pot: null,
        /** `runPageContextProbe` output. */
        page: null,
    };
}

function short(s, n = 120) {
    const str = s == null ? '' : String(s);
    return str.length > n ? str.slice(0, n - 1) + '…' : str;
}

/**
 * Append one step outcome.
 *
 * `ok` is three-valued on purpose:
 *   `true`  the step ran and succeeded
 *   `false` the step ran and failed — this is what `failedAt` points at
 *   `null`  the step was deliberately not run (e.g. the interpreter global was
 *           already present, so the eval at `po_token.js:192` is skipped)
 *
 * A step may be recorded more than once — `po_token.js` really does try the
 * native transport and then the WebView transport for the same fetch, and
 * collapsing those would hide which one is broken. When it is, record the
 * attempt with `extra.fatal === false` (an informational probe that did not end
 * the mint) and then one aggregate entry whose `ok` reflects what actually
 * happened. Otherwise the first failed transport probe would be reported as
 * the step the mint died at, which would be a lie whenever the second transport
 * succeeded.
 *
 * @returns {object} the recorded entry
 */
export function recordStep(report, step, ok, detail, extra = {}) {
    const entry = {
        step,
        ok: ok === undefined ? null : ok,
        detail: detail == null ? '' : short(detail),
        ...(extra && typeof extra === 'object' ? extra : {}),
        t: Date.now(),
    };
    if (!report || typeof report !== 'object') return entry;
    let fatal = false;
    if (!STEP_IDS.has(step)) {
        // A typo in a step id would silently produce a report whose steps are
        // all "not reached". Refuse loudly in the report itself instead.
        report.unknownSteps = [...(report.unknownSteps || []), step];
    } else {
        fatal = entry.fatal === undefined ? FATAL_BY_DEFAULT.includes(step) : entry.fatal === true;
        if (entry.ok === false && fatal && !report.mint.failedAt) {
            report.mint.failedAt = step;
            report.mint.failedReason = entry.detail;
        }
    }
    // Resolved, and stored: the renderer needs to tell a withhold from a
    // failure, and it cannot know the answer without it.
    entry.fatal = fatal;
    report.steps.push(entry);
    return entry;
}

/**
 * Mark the mint as never having run, with a reason.
 * Safe to call after a step failure — it does not clear `failedAt`.
 */
export function recordNotAttempted(report, reason) {
    if (!report || !report.mint) return report;
    report.mint.attempted = false;
    report.mint.notAttemptedReason = short(reason, 200);
    return report;
}

/**
 * Mark the mint as having run.
 * @param {'succeeded'|'failed'} outcome
 */
export function recordMintOutcome(report, outcome, data = {}) {
    if (!report || !report.mint) return report;
    report.mint.attempted = true;
    report.mint.outcome = outcome === 'succeeded' ? 'succeeded' : 'failed';
    if (data.token) {
        report.mint.tokenLen = String(data.token).length;
        report.mint.tokenPreview = short(String(data.token), 16);
    }
    if (data.visitorData) report.mint.visitorData = short(String(data.visitorData), 16);
    if (data.source) report.mint.tokenSource = data.source;
    return report;
}

/** Record the `applyPoTokenToUrl` verdict (the `pot-apply` step's other half). */
export function recordPotApply(report, applied, winningClient) {
    if (!report || !report.mint) return report;
    report.pot = {
        action: applied?.action || null,
        winningClient: winningClient || null,
        detail: applied?.detail || null,
    };
    recordStep(report, 'pot-apply', applied?.action === 'attached' || applied?.action === 'already-present' ? true : applied?.action ? false : null,
        describePotAction(applied?.action, winningClient));
    return report;
}

function describePotAction(action, winner) {
    const w = winner ? ` winner=${winner}` : '';
    switch (action) {
        case 'attached':
            return `pot added to the ${winner} url${w}`;
        case 'already-present':
            return `pot was already on the ${winner} url${w}`;
        case 'stripped':
            return `pot REMOVED from the ${winner} url — a Web/BotGuard token is platform-bound and the edge 403s at byte 0 with a foreign one${w}`;
        case 'no-token':
            return `token withheld: ${winner} is not web-family, so a Web/BotGuard token is not valid for it${w}`;
        case 'not-applicable':
            return 'no media url to carry a pot (direct audio, or the host is not googlevideo)';
        case 'unparsable':
            return 'stream url could not be parsed, so no pot was applied';
        default:
            return 'the url step never ran (resolve failed before it)';
    }
}

/**
 * The first step recorded with `ok === false`, or `null`.
 */
export function firstFailure(report) {
    const steps = report?.steps;
    if (!Array.isArray(steps)) return null;
    return steps.find((s) => s && s.ok === false) || null;
}

/**
 * The first failure that is not merely the intended consequence of a decision
 * (see `CONSEQUENCE_STEPS`). This is what a one-line summary falls back to when
 * the run *succeeded* — a proof obtained via the cold-start fallback after
 * `botguard-load` died is a real success, and reporting it as a bare
 * "succeeded" would hide a 3-second `VM operation timed out` and a dead WebPO
 * path, which is the single most important thing this report exists to show.
 */
export function firstRealProblem(report) {
    const steps = report?.steps;
    if (!Array.isArray(steps)) return null;
    return steps.find((s) => s && s.ok === false && !CONSEQUENCE_STEPS.has(s.step)) || null;
}

/**
 * The step entry `failedAt` points at, or `null`.
 *
 * Not the same as `firstFailure`: a non-fatal probe (a transport that was
 * retried, a deliberate withhold) is `ok === false` too, and pairing
 * `failedAt` with the *first* failing entry would attach the right step's name
 * to the wrong step's message.
 */
export function failedStepEntry(report) {
    const step = report?.mint?.failedAt;
    if (!step) return null;
    const steps = report?.steps;
    if (!Array.isArray(steps)) return null;
    return steps.find((s) => s && s.step === step && s.ok === false) || null;
}

// ── state classification ─────────────────────────────────────────────────────

/**
 * Reduce a report to exactly one of `MINT_STATES`.
 *
 * The ordering is the whole point, and it is deliberately not
 * "did a token exist". A token existing says nothing about whether minting
 * worked — that conflation is the bug this file exists to make impossible. The
 * questions, in order:
 *
 *   1. did the caller bring the token?      → `user-token` (we minted nothing)
 *   2. did a mint run and fail?             → `attempted-failed`
 *   3. is no token of ours in play at all?  → `not-attempted` / `unknown`
 *   4. do we hold one?                       → attached or not
 *
 * @returns {string} a `MINT_STATES` value
 */
export function classifyMintOutcome(report) {
    if (!report || typeof report !== 'object' || !report.mint) return MINT_STATES.UNKNOWN;
    const m = report.mint;
    const source = m.tokenSource || 'none';

    if (source === 'user') return MINT_STATES.USER_TOKEN;

    // `outcome !== 'succeeded'` would fold "ran, and we never wrote down how it
    // ended" into "failed". Only an explicit `failed` is a failure; a missing
    // outcome is `unknown`, because the difference decides whether the owner
    // should be looking at the WebView or at the app's own bookkeeping.
    if (m.attempted === true && m.outcome === 'failed') return MINT_STATES.ATTEMPTED_FAILED;

    const holdsWebToken = source === 'minted' || source === 'cache';
    if (!holdsWebToken) {
        if (m.attempted === false) return MINT_STATES.NOT_ATTEMPTED;
        return MINT_STATES.UNKNOWN;
    }

    // We hold a Web/BotGuard token. Minting worked (or a previous run's mint
    // did). Everything below is about where the token travelled, which is a
    // different question with a different owner-facing consequence.
    const placement = placementFromPotAction(report.pot?.action);
    if (placement === POT_PLACEMENT.ATTACHED) return MINT_STATES.MINTED_ATTACHED;
    // No `pot` record at all means `applyPoTokenToUrl` never ran. Reporting that
    // as `minted-stripped` would assert a decision nobody made.
    if (!report.pot) return MINT_STATES.MINTED_NOT_EVALUATED;
    return MINT_STATES.MINTED_STRIPPED;
}

// ── the snapshot failure-shape taxonomy ──────────────────────────────────────

/**
 * The ways `client.snapshot({ webPoSignalOutput })` can fail to leave us holding
 * a minter factory, each named separately.
 *
 * ── Why this taxonomy exists ─────────────────────────────────────────────────
 *
 * On the 2026-09-29 device run (v2.6.61) the whole mint succeeded up to
 * `snapshot`, and then the report said, in full:
 *
 *     FAIL snapshot  snapshot returned a response and no minter factory
 *                     — WebPoMinter.create would throw PMD:Undefined · 123ms
 *
 * which is one line standing for at least four independent facts:
 *
 *   1. the VM pushed **nothing** (`webPoSignalOutput.length === 0`);
 *   2. the VM pushed **something that is not a function** — a different bug with
 *      a different fix, and one that does *not* produce `PMD:Undefined` (a truthy
 *      non-function passes bgutils' `!getMinter` guard and then throws a
 *      TypeError when it is called, WebPoMinter.js:16-21);
 *   3. `snapshot()` returned **nothing at all** (no throw, no response — which
 *      means the VM never called its completion callback inside the 3s race at
 *      BotGuardClient.js:131);
 *   4. `snapshot()` **threw** (3s VM timeout, `EGOU`, `EGLIU`).
 *
 * The old message conflated 1 and 2 into "no minter factory" and asserted 3 in
 * the same breath by printing "returned a response", which is a claim about a
 * value the report never measured. Worse, the old `ok` flag was
 * `botguardResponse && webPoSignalOutput.length` — so case 2 was recorded as
 * **`ok: true`, "and a minter factory"**, a positive lie, and the run then died
 * two steps later with a `mint` error that named the wrong stage.
 *
 * `SNAPSHOT_SHAPES` exists so the next device run can answer *which* of these it
 * was. That is the whole point: we cannot iterate on a phone, so one run has to
 * eliminate four hypotheses rather than restate one.
 */
export const SNAPSHOT_SHAPES = {
    /** `webPoSignalOutput[0]` is a function. GenerateIT and `WebPoMinter.create` can run. */
    OK: 'ok',
    /** The client object exposes neither `snapshot` nor `snapshotSynchronous`. */
    UNSUPPORTED: 'unsupported',
    /** `snapshot()` threw — 3s VM timeout, `EGOU`, or a bad `program`. */
    THREW: 'threw',
    /** No throw, no response: the VM never invoked its completion callback. */
    NO_RESPONSE: 'no-response',
    /** A response came back and the array is still empty. The measured device case. */
    EMPTY_ARRAY: 'empty-array',
    /** Something was pushed, but `[0]` is not a function. */
    NON_FUNCTION: 'non-function',
};

/**
 * Appended to every shape that leaves us without a usable factory.
 *
 * This is here to kill a specific, attractive, wrong theory before it is spent a
 * device run on: *"mint the GenerateIT integrity token first, then snapshot."*
 * The GenerateIT request body is the protobuf pair `[requestKey, botguardResponse]`
 * (`po_token.js` builds it from the snapshot's return value), and
 * `WebPoMinter.create` passes the token **into** the factory as its argument
 * (`WebPoMinter.js:21`, `getMinter(base64ToU8(integrityTokenResponse.integrityToken))`).
 * The token is therefore *derived from* the snapshot response and *consumed
 * after* the factory exists — snapshot-then-GenerateIT is the contract's order,
 * and it is the order this file already runs in. Reordering is not "an
 * experiment"; it is a request with no bytes to send.
 *
 * Confidence: read-from-source (both citations above are in `ui/vendor/bgutils/`).
 * What remains genuinely unknown is *why* the VM pushed nothing — the token's
 * position in the protocol does not prove the VM's push is unconditional, only
 * that the token cannot be the missing prerequisite.
 */
const NO_FACTORY_NOTE =
    'GenerateIT could not be run first to supply it: the GenerateIT payload IS [requestKey, botguardResponse], ' +
    'so the integrity token is derived from this snapshot response, not the other way round';

/**
 * How much room the reasoning half of a snapshot message gets.
 *
 * `recordStep` caps `detail` at 120 chars so a phone-width line stays readable,
 * and `NO_FACTORY_NOTE` does not fit under that — left in `detail` it was
 * truncated away, and it is the single sentence most worth reading on the next
 * run. So the measurement goes in `detail` ("what happened") and the reasoning
 * goes in `note` ("why this is not fixable by reordering"), and `stepDetail`
 * renders the second with its own budget.
 */
const NOTE_BUDGET = 220;

/** Cap on how many element types we will name, so a huge array cannot flood a report. */
const SIGNAL_TYPE_SAMPLE = 8;

/**
 * Measure the by-reference `webPoSignalOutput` array without assuming anything
 * about it. `typeof` per element is the only safe read: the VM wrote it, so the
 * elements are not ours to trust, and a getter on index 0 could throw.
 */
function describeWebPoSignalOutput(arr) {
    const facts = { signalIsArray: Array.isArray(arr), signalLength: null, signalTypes: [] };
    if (!Array.isArray(arr)) return facts;
    let len = null;
    try { len = arr.length; } catch (_) { /* a hostile `length` getter */ }
    facts.signalLength = len;
    if (typeof len !== 'number') return facts;
    const n = Math.min(len, SIGNAL_TYPE_SAMPLE);
    for (let i = 0; i < n; i++) {
        let t = 'threw';
        try { t = typeof arr[i]; } catch (_) { /* keep 'threw' */ }
        facts.signalTypes.push(`${i}:${t}`);
    }
    if (len > n) facts.signalTruncated = len - n;
    return facts;
}

/** Measure what `snapshot()` actually resolved to — the old report assumed "a string". */
function describeSnapshotResponse(res) {
    const facts = { responseType: 'undefined', responseLen: null };
    try {
        facts.responseType = res === null ? 'null' : typeof res;
        if (typeof res === 'string' || Array.isArray(res)) facts.responseLen = res.length;
    } catch (_) {
        facts.responseType = 'threw';
    }
    return facts;
}

/**
 * Reduce a snapshot attempt to exactly one `SNAPSHOT_SHAPES` value, with the
 * measurements attached.
 *
 * Pure: no globals, no timers, no bgutils import — it takes the four things the
 * caller observed and returns a verdict plus a `facts` object to hand to
 * `recordStep`. That is what makes it unit-testable under node against the real
 * module rather than a paraphrase of it.
 *
 * Precedence is deliberate and is the order the questions are actually asked in:
 * we did not call it (unsupported) → it threw (threw) → it said nothing
 * (no-response) → we called it and got a response, so now the only question left
 * is what it wrote into the array we passed by reference.
 *
 * @param {{webPoSignalOutput?: any, botguardResponse?: any, snapshotError?: string|null,
 *          unsupported?: boolean, settle?: {settleGrew?: boolean, settleWaitedMs?: number}|null}} input
 * @returns {{shape: string, ok: boolean, detail: string, note?: string, facts: object}}
 */
export function classifySnapshotOutcome(input = {}) {
    const { webPoSignalOutput = null, botguardResponse = null, snapshotError = null, unsupported = false, settle = null } = input || {};
    const facts = { ...describeWebPoSignalOutput(webPoSignalOutput), ...describeSnapshotResponse(botguardResponse) };

    if (settle) {
        facts.settleWaitedMs = settle.settleWaitedMs ?? null;
        facts.settleGrew = settle.settleGrew === true;
    }

    const finish = (shape, ok, detail, note) => ({ shape, ok, detail, note: note || undefined, facts });

    if (unsupported) {
        return finish(
            SNAPSHOT_SHAPES.UNSUPPORTED,
            false,
            'this BotGuardClient exposes neither snapshot() nor snapshotSynchronous(), so the minter factory has no route at all'
        );
    }
    if (snapshotError) {
        return finish(SNAPSHOT_SHAPES.THREW, false, `snapshot threw: ${snapshotError}`);
    }

    const hasResponse = botguardResponse !== null && botguardResponse !== undefined;
    if (!hasResponse) {
        return finish(
            SNAPSHOT_SHAPES.NO_RESPONSE,
            false,
            'snapshot neither threw nor resolved — the VM never called its completion callback within the 3s race in BotGuardClient.js:131',
            'no response means no GenerateIT payload either, so nothing downstream can run'
        );
    }

    // A response came back. Everything from here is a statement about the array,
    // which the VM holds by reference (BotGuardClient.js:152-157).
    const len = facts.signalLength;
    if (len === 0) {
        const grew = settle && settle.settleGrew === true;
        return finish(
            SNAPSHOT_SHAPES.EMPTY_ARRAY,
            false,
            `snapshot returned ${facts.responseType}(len=${facts.responseLen ?? '?'}) but pushed NOTHING into webPoSignalOutput` +
            ` (length=0${settle && !grew ? `, still 0 after waiting ${settle.settleWaitedMs ?? 0}ms for a late push` : ''})` +
            ` — the VM ran and answered but never handed over a minter factory`,
            NO_FACTORY_NOTE
        );
    }
    if (typeof webPoSignalOutput[0] !== 'function') {
        return finish(
            SNAPSHOT_SHAPES.NON_FUNCTION,
            false,
            `webPoSignalOutput has ${len} element(s) but [0] is ${facts.signalTypes[0] || 'unknown'}, not a function — the VM pushed a value WebPoMinter.create cannot call`,
            'this is NOT the PMD:Undefined case: bgutils\' `!getMinter` guard (WebPoMinter.js:17) passes for a truthy non-function, which then throws when called on line 21. ' + NO_FACTORY_NOTE
        );
    }
    return finish(
        SNAPSHOT_SHAPES.OK,
        true,
        `snapshot returned ${facts.responseType}(len=${facts.responseLen ?? '?'}) and webPoSignalOutput[0] is a function (${len} element(s) total)`
    );
}

// ── the page-context experiment ───────────────────────────────────────────────

/**
 * The source string the `new Function` probe compiles.
 *
 * It is built so that a stubbed or mangled `Function` cannot produce a passing
 * result by accident: `n` is arithmetic that has to actually execute, and `echo`
 * is a concatenation, so a constructor that returns a canned object instead of
 * compiling the body yields `null` for both. Nothing here asserts — the caller
 * reads the values.
 */
const PROBE_SOURCE =
    'var a = 6, b = 7; return { n: a * b, echo: "auralis" + "-" + "probe", t: typeof globalThis };';

/**
 * Globals the mint path may need, and where each requirement is known from.
 *
 * `cited: false` marks a requirement that is **not** established by a source we
 * can quote — BotGuard's interpreter is a script downloaded at runtime, so its
 * real global usage is unknown to us until we run it (which is what step
 * `botguard-global` measures). They are listed anyway because their absence
 * would be a cheap, actionable finding, and because reporting "we checked N
 * globals and M were missing" is only meaningful if the checked set is stated.
 *
 * An absent global is reported as data (`type: "undefined"`, and a name in
 * `missing`) and is never on its own a failure verdict: the list is a
 * hypothesis, not a contract.
 */
export const PROBE_GLOBALS = [
    // ui/vendor/bgutils/utils/helpers.js:93-104 — `isBrowser()` requires all of
    // these before `getHeaders()` will omit the `user-agent` header.
    { name: 'window', cited: true, source: 'bgutils/utils/helpers.js:94-102 isBrowser()' },
    { name: 'document', cited: true, source: 'bgutils/utils/helpers.js:95-96 isBrowser()' },
    { name: 'HTMLElement', cited: true, source: 'bgutils/utils/helpers.js:97 isBrowser()' },
    { name: 'navigator', cited: true, source: 'bgutils/utils/helpers.js:98 isBrowser()' },
    { name: 'getComputedStyle', cited: true, source: 'bgutils/utils/helpers.js:99 isBrowser()' },
    { name: 'requestAnimationFrame', cited: true, source: 'bgutils/utils/helpers.js:100 isBrowser()' },
    { name: 'matchMedia', cited: true, source: 'bgutils/utils/helpers.js:101 isBrowser()' },
    // ui/vendor/bgutils/core/WebPoMinter.js:38,102 and
    // ui/vendor/bgutils/utils/helpers.js:35-44.
    { name: 'TextEncoder', cited: true, source: 'bgutils/core/WebPoMinter.js:38 new TextEncoder()' },
    { name: 'TextDecoder', cited: true, source: 'bgutils/core/WebPoMinter.js:102 new TextDecoder()' },
    { name: 'atob', cited: true, source: 'bgutils/utils/helpers.js:35 base64ToU8' },
    { name: 'btoa', cited: true, source: 'bgutils/utils/helpers.js:42 u8ToBase64' },
    { name: 'Uint8Array', cited: true, source: 'bgutils/core/WebPoMinter.js:41-43 instanceof' },
    { name: 'Function', cited: true, source: 'bgutils/core/WebPoMinter.js:22 instanceof Function' },
    // ui/js/modules/po_token.js:37-46 nativeFetchPo needs a usable fetch when
    // the Rust http_fetch bridge is unavailable.
    { name: 'fetch', cited: true, source: 'modules/po_token.js:46 window.fetch fallback' },
    // The BotGuard interpreter is a runtime-downloaded script; these are the
    // usual suspects in an anti-automation VM and are listed as a hypothesis
    // only. See the comment above: uncited on purpose.
    { name: 'crypto', cited: false, source: 'assumed — BotGuard interpreter is a runtime blob we cannot read' },
    { name: 'performance', cited: false, source: 'assumed — same' },
    { name: 'screen', cited: false, source: 'assumed — same' },
    { name: 'Intl', cited: false, source: 'assumed — same' },
    { name: 'WebAssembly', cited: false, source: 'assumed — same' },
    { name: 'PointerEvent', cited: false, source: 'assumed — BotGuardClient telemetry watches mouse/keyboard' },
    { name: 'MouseEvent', cited: false, source: 'assumed — same' },
    { name: 'KeyboardEvent', cited: false, source: 'assumed — same' },
    { name: 'Worker', cited: false, source: 'assumed — same' },
    { name: 'WebSocket', cited: false, source: 'assumed — same' },
];

function looksLikeAndroidWebView(ua) {
    return /;\s*wv\)/.test(ua) || /\bwv\b/.test(ua);
}

/**
 * Run the page-context experiment and return its raw findings.
 *
 * Deliberately synchronous, allocation-light, and side-effect free: it compiles
 * one small string, calls it, and reads `typeof` off a fixed list. It runs
 * inside the page's own realm — this module is imported by the page, so
 * `globalThis` here *is* the WebView's global object, which is the only place
 * the question "can this WebView eval?" can be answered honestly. A desktop
 * browser answer would be worthless for a phone WebView.
 *
 * Nothing here returns a boolean verdict. The caller gets the numbers and makes
 * its own judgement, because the two sub-questions have different failure
 * shapes: `new Function` may be refused outright (CSP), or accepted and quietly
 * mangled — and only the returned values distinguish those.
 *
 * @param {object} [scope] the global object to probe; injectable for tests
 */
export function runPageContextProbe(scope) {
    const g = scope || (typeof globalThis !== 'undefined' ? globalThis : {});
    const out = {
        at: new Date().toISOString(),
        ua: null,
        looksLikeAndroidWebView: null,
        newFunction: { ctor: 'unavailable', compiled: false, evaluated: false, result: null, n: null, echo: null, error: null, errorName: null },
        globals: {},
        missing: [],
        checked: PROBE_GLOBALS.length,
    };

    try {
        out.ua = typeof g.navigator === 'object' && g.navigator ? String(g.navigator.userAgent || '') : null;
        out.looksLikeAndroidWebView = out.ua ? looksLikeAndroidWebView(out.ua) : null;
    } catch (_) { /* a WebView that throws on navigator is itself the finding */ }

    // ── the eval experiment ──
    try {
        out.newFunction.ctor = typeof g.Function;
    } catch (_) {
        out.newFunction.ctor = 'threw';
    }
    if (out.newFunction.ctor === 'function') {
        let fn = null;
        try {
            fn = new g.Function(PROBE_SOURCE);
            out.newFunction.compiled = true;
        } catch (e) {
            out.newFunction.error = short(e?.message || e, 200);
            out.newFunction.errorName = e?.name || null;
        }
        if (fn) {
            try {
                const r = fn();
                out.newFunction.evaluated = true;
                out.newFunction.result = r && typeof r === 'object' ? { t: r.t ?? null } : typeof r;
                // Only real if the body actually ran. A `Function` that compiles
                // to a stub returns undefined here, not 42.
                out.newFunction.n = typeof r?.n === 'number' ? r.n : null;
                out.newFunction.echo = typeof r?.echo === 'string' ? r.echo : null;
            } catch (e) {
                out.newFunction.error = short(e?.message || e, 200);
                out.newFunction.errorName = e?.name || null;
            }
        }
    }

    // ── the globals inventory ──
    for (const entry of PROBE_GLOBALS) {
        let t = 'threw';
        try {
            t = typeof g[entry.name];
        } catch (_) { /* keep 'threw' */ }
        out.globals[entry.name] = { type: t, cited: entry.cited };
        if (t === 'undefined' || t === 'threw') out.missing.push(entry.name);
    }
    return out;
}

/** One-line human summary of a probe result. */
export function describeProbe(probe) {
    if (!probe || typeof probe !== 'object') return 'page probe: not run';
    const nf = probe.newFunction || {};
    const verdict = nf.compiled && nf.n === 42 ? 'eval works' : nf.compiled ? `eval compiled but returned n=${nf.n}` : nf.error ? 'eval REFUSED' : 'eval unavailable';
    return (
        `new Function: ${verdict}` +
        ` (n=${nf.n === null || nf.n === undefined ? 'null' : nf.n}, echo=${nf.echo ?? 'null'}` +
        `${nf.error ? `, error="${nf.error}"` : ''}); globals: ${(probe.checked || 0) - (probe.missing || []).length}/${probe.checked || 0} present` +
        `${(probe.missing || []).length ? `, missing: ${probe.missing.join(',')}` : ''}` +
        `${probe.looksLikeAndroidWebView === true ? ' [android webview ua]' : ''}`
    );
}

/**
 * Record the page-context experiment as step `page-context-probe`.
 *
 * `ok` is intentionally `null` (skipped) rather than a boolean when globals are
 * missing: a missing global is a fact about an uncited hypothesis list, and
 * calling it a failure would be the same "assertion that passes vacuously"
 * failure mode the caller is being warned about. Only a refused or mangled
 * `new Function` is recorded as a failure, because that requirement *is* cited
 * (`po_token.js:195` needs it).
 */
export function recordPageProbe(report, probe) {
    if (!report) return report;
    report.page = probe || null;
    const nf = (probe || {}).newFunction || {};
    const worked = nf.compiled === true && nf.n === 42 && nf.echo === 'auralis-probe';
    // Three outcomes, and the middle one matters: a `Function` that accepted the
    // source and returned a function that did not actually run it is a *mangled*
    // eval, which is a failure of the cited requirement — not a skip. Only a
    // scope with no usable `Function` at all is a skip.
    const ok = worked ? true : nf.error || nf.compiled ? false : null;
    recordStep(report, 'page-context-probe', ok, describeProbe(probe), {
        newFunctionCtor: nf.ctor ?? null,
        missingGlobals: Array.isArray(probe?.missing) ? probe.missing.slice() : [],
    });
    return report;
}

// ── rendering ────────────────────────────────────────────────────────────────

function stepMarker(entry) {
    if (entry.ok === true) return 'ok  ';
    if (entry.ok === false) return CONSEQUENCE_STEPS.has(entry.step) ? 'held' : 'FAIL';
    return 'skip';
}

function stepDetail(entry) {
    const bits = [entry.detail || ''];
    // The machine-readable verdict, right after the prose. `detail` is capped at
    // 120 chars and routinely truncates mid-sentence, so the one field that must
    // never be the part that got cut off gets its own slot.
    if (entry.shape) bits.push(`shape=${entry.shape}`);
    if (entry.status !== undefined && entry.status !== null) bits.push(`status=${entry.status}`);
    if (entry.transport) bits.push(`via=${entry.transport}`);
    if (entry.endpoint) bits.push(`endpoint=${entry.endpoint}`);
    if (entry.ms) bits.push(`${entry.ms}ms`);
    if (entry.error) bits.push(`error=${short(entry.error, 160)}`);
    // Its own line's worth of room, because the reasoning a step carries is
    // routinely longer than the 120-char `detail` budget allows. Rendered last
    // so the measurement stays first on a phone.
    if (entry.note) bits.push(`note: ${short(entry.note, NOTE_BUDGET)}`);
    if (entry.data && typeof entry.data === 'object') {
        for (const [k, v] of Object.entries(entry.data)) {
            if (v === null || v === undefined || v === '' ) continue;
            bits.push(`${k}=${Array.isArray(v) ? (v.length ? v.join('/') : '[]') : short(v, 40)}`);
        }
    }
    const s = bits.filter(Boolean).join(' · ');
    return s || '(no detail)';
}

/**
 * The detailed, copyable block. Every step appears, including the ones that
 * never ran: `- not reached` after the first failure is the part that names the
 * step worth fixing.
 */
export function formatMintReport(report) {
    if (!report || typeof report !== 'object') return 'mint: no report';
    const state = classifyMintOutcome(report);
    const m = report.mint || {};
    const head = [
        `mint: ${state}`,
        `attempted=${m.attempted === null ? 'unknown' : m.attempted}`,
        `outcome=${m.outcome || 'n/a'}`,
        `token=${m.tokenSource || 'none'}${m.tokenLen ? ` len=${m.tokenLen}` : ''}`,
        m.proofKind ? `proof=${m.proofKind}` : null,
        m.visitorData ? `vd=${m.visitorData}` : 'vd=none',
        m.failedAt ? `failedAt=${m.failedAt}` : null,
        m.notAttemptedReason ? `notAttempted="${m.notAttemptedReason}"` : null,
    ]
        .filter(Boolean)
        .join(' ');

    const byStep = new Map();
    for (const e of Array.isArray(report.steps) ? report.steps : []) {
        if (e && e.step) byStep.set(e.step, (byStep.get(e.step) || []).concat([e]));
    }
    const failIdx = (() => {
        const f = failedStepEntry(report);
        return f ? STEP_INDEX.get(f.step) : -1;
    })();

    const lines = [head];
    MINT_STEPS.forEach((s, i) => {
        const entries = byStep.get(s.id);
        if (!entries || !entries.length) {
            lines.push(`  -    ${s.id.padEnd(20)} ${failIdx >= 0 && i > failIdx ? 'not reached' : 'not recorded'}`);
            return;
        }
        for (const e of entries) {
            lines.push(`  ${stepMarker(e)} ${e.step.padEnd(20)} ${stepDetail(e)}`);
        }
    });
    if (Array.isArray(report.unknownSteps) && report.unknownSteps.length) {
        lines.push(`  !! unknown step ids (typo in a recordStep call): ${report.unknownSteps.join(', ')}`);
    }
    return lines.join('\n');
}

/**
 * The single-line form rendered under `clients: …` in the download row.
 * Kept short on purpose — the row is read on a phone, and the Copy report
 * button is where the full block goes.
 */
export function formatMintReportLine(report) {
    if (!report || typeof report !== 'object') return '';
    const state = classifyMintOutcome(report);
    const m = report.mint || {};
    const parts = [
        `state=${state}`,
        m.attempted === false ? 'not-attempted' : `attempted=${m.attempted === null ? 'unknown' : m.attempted}`,
        m.outcome ? `outcome=${m.outcome}` : null,
        m.proofKind ? `proof=${m.proofKind}` : null,
        m.failedAt ? `failedAt=${m.failedAt}` : null,
        report.pot?.action ? `pot=${report.pot.action}` : null,
        report.pot?.winningClient ? `winner=${report.pot.winningClient}` : null,
        m.notAttemptedReason ? `reason="${m.notAttemptedReason}"` : null,
    ]
        .filter(Boolean)
        .join(' ');

    // The tail names the single most useful fact, and which fact that is
    // depends on the state. Appending "the last thing that went wrong" instead
    // would put `pot-apply`'s deliberate withhold on a perfectly good mint and
    // invite the misreading this whole module is built to prevent.
    if (m.failedAt) {
        const f = failedStepEntry(report);
        return `mint: ${parts} — ${m.failedAt}: ${f ? stepDetail(f) : m.failedReason || ''}`;
    }
    // A success that got there over a dead step, or a token whose fate was never
    // determined. Suppressed only for a withhold and for a run that was never
    // attempted — there, the fact below is the more useful one.
    if (state !== MINT_STATES.NOT_ATTEMPTED && state !== MINT_STATES.MINTED_STRIPPED) {
        const p = firstRealProblem(report);
        if (p) return `mint: ${parts} — first problem at ${p.step}: ${stepDetail(p)}`;
    }
    if (state === MINT_STATES.MINTED_STRIPPED && report.pot) {
        return `mint: ${parts} — ${describePotAction(report.pot.action, report.pot.winningClient)}`;
    }
    if (report.page) return `mint: ${parts} — ${describeProbe(report.page)}`;
    return `mint: ${parts}`;
}
