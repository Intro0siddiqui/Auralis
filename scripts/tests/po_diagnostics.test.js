#!/usr/bin/env node
/**
 * po_diagnostics.test.js — behavioural tests for the PO-token mint diagnostics.
 *
 *   node --test scripts/tests/po_diagnostics.test.js
 *
 * ── What is under test, and what is not ─────────────────────────────────────
 *
 * There are two layers here and the distinction matters, because this repo has
 * already shipped a test that asserted against its own copy of the code under
 * test (see `pot_scope.test.js`'s header and AGENTS.md §4.6): it defined its own
 * version of the token-append rule and asserted against that, so it passed
 * regardless of what `youtube.js` did, while a live 403 shipped. A test that
 * cannot fail when the product is broken is worse than no test, because it is
 * read as evidence.
 *
 *   1. `ui/js/modules/po_diagnostics.js` — pure, dependency-free, so the real
 *      module is imported and its real functions are called. Nothing here
 *      re-implements the logic under test; the fixtures are inputs, not copies.
 *
 *   2. `ui/js/modules/po_token.js` — driven **end to end** against the real
 *      vendored `bgutils` (which really does load under node) with a fake
 *      `innertube` and a stubbed `fetch`. This is the load-bearing half: it
 *      proves the step recorder, the failure attribution, and the
 *      minted-vs-stripped distinction behave correctly against the *shipped*
 *      code, not a paraphrase of it.
 *
 * What is NOT tested, and cannot be: whether a real Android WebView can mint.
 * That needs the device. What these tests establish is that whichever answer
 * the device gives, it will be reported — including the case that matters most
 * and was previously invisible, where `new Function` succeeds and still leaves
 * `BotGuardClient` nothing to work with.
 */
import { describe, it, before } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

import {
    MINT_STATES,
    MINT_STEPS,
    MINT_STEP_IDS,
    POT_PLACEMENT,
    SNAPSHOT_SHAPES,
    classifyMintOutcome,
    classifySnapshotOutcome,
    createMintReport,
    describeProbe,
    firstFailure,
    formatMintReport,
    formatMintReportLine,
    placementFromPotAction,
    recordMintOutcome,
    recordNotAttempted,
    recordPageProbe,
    recordPotApply,
    recordStep,
    runPageContextProbe,
} from '../../ui/js/modules/po_diagnostics.js';

const here = import.meta.dirname ?? path.dirname(new URL(import.meta.url).pathname);
const poTokenPath = path.resolve(here, '../../ui/js/modules/po_token.js');

/** A report that reached the `mint` step, for classifying placement states. */
function mintedReport({ source = 'minted', action = 'no-token', winner = 'IOS' } = {}) {
    const r = createMintReport({ videoId: 'yF9nmg_jHNs' });
    recordStep(r, 'bgutils-import', true, 'vendored bgutils loaded');
    recordStep(r, 'attestation-challenge', true, 'bg_challenge received');
    recordStep(r, 'mint', true, 'proof produced');
    recordMintOutcome(r, 'succeeded', { token: 'A'.repeat(140), visitorData: 'CgtMT0NhbFZpc2l0b3I', source });
    recordPotApply(r, { action }, winner);
    return r;
}

describe('recordStep — failure attribution', () => {
    it('points failedAt at the first step that actually failed', () => {
        const r = createMintReport();
        recordStep(r, 'attestation-challenge', true, 'ok');
        recordStep(r, 'visitor-data', false, 'no visitorData');
        recordStep(r, 'interpreter-fetch', false, 'no bytes');
        assert.equal(r.mint.failedAt, 'visitor-data');
        assert.equal(r.mint.failedReason, 'no visitorData');
    });

    it('does not let a non-fatal probe overwrite the real failure', () => {
        // `interpreter-fetch` is recorded twice in po_token.js: once per
        // transport attempt, and the first one can fail while the second
        // succeeds. If a failed first attempt could claim `failedAt`, the report
        // would blame a transport that then worked.
        const r = createMintReport();
        recordStep(r, 'new-function-eval', false, 'CSP refused eval');
        recordStep(r, 'interpreter-fetch', false, 'rust http_fetch refused', { fatal: false });
        assert.equal(r.mint.failedAt, 'new-function-eval');
    });

    it('never blames pot-apply — a deliberate withhold is not a mint failure', () => {
        // THE regression this state exists for. A perfectly good Web token kept
        // off an `ios` url is correct behaviour (a Web token on an `ios` url is
        // what produced the byte-0 403 in v2.6.50). If the withhold were
        // recorded as a failure, the report would read "minting failed at
        // pot-apply" and send the owner to fix a WebView that is working.
        const r = mintedReport({ action: 'stripped', winner: 'ANDROID_VR' });
        assert.equal(r.mint.failedAt, null);
        assert.equal(r.mint.outcome, 'succeeded');
        const f = firstFailure(r);
        // There is a non-ok step (the withhold), but it is not a mint failure.
        assert.equal(f?.step, 'pot-apply');
        assert.equal(r.mint.failedAt, null);
    });

    it('keeps a failed mint as "attempted" rather than downgrading it to not-attempted', () => {
        const r = createMintReport();
        recordStep(r, 'bgutils-import', false, 'no candidates', { fatal: false });
        recordMintOutcome(r, 'failed');
        assert.equal(r.mint.attempted, true);
        assert.equal(classifyMintOutcome(r), MINT_STATES.ATTEMPTED_FAILED);
    });

    it('catches an unknown step id instead of letting it vanish', () => {
        // A typo in a step id produces a report whose every step renders
        // "not reached" — indistinguishable from a mint that died early.
        const r = createMintReport();
        recordStep(r, 'nonexistent-step', false, 'boom');
        assert.deepEqual(r.unknownSteps, ['nonexistent-step']);
        assert.equal(r.mint.failedAt, null, 'an unknown id must not claim failedAt');
        assert.match(formatMintReport(r), /unknown step ids/);
    });
});

describe('classifyMintOutcome — the four states must never be confused', () => {
    it('not-attempted: no mint ran and no token of ours is in play', () => {
        const r = createMintReport();
        recordNotAttempted(r, 'modules/po_token.js could not be imported');
        assert.equal(classifyMintOutcome(r), MINT_STATES.NOT_ATTEMPTED);
        assert.match(formatMintReportLine(r), /not-attempted/);
        assert.match(formatMintReportLine(r), /could not be imported/);
    });

    it('attempted-failed: a mint ran and died at a named step', () => {
        const r = createMintReport();
        recordStep(r, 'bgutils-import', true, 'loaded');
        recordStep(r, 'new-function-eval', false, 'new Function(interpreter) threw: CSP blocks unsafe-eval');
        recordMintOutcome(r, 'failed');
        const line = formatMintReportLine(r);
        assert.equal(classifyMintOutcome(r), MINT_STATES.ATTEMPTED_FAILED);
        assert.match(line, /state=attempted-failed/);
        assert.match(line, /failedAt=new-function-eval/);
        assert.match(line, /unsafe-eval/);
    });

    it('minted-attached: a Web token reached a web-family url', () => {
        const r = mintedReport({ action: 'attached', winner: 'MWEB' });
        assert.equal(classifyMintOutcome(r), MINT_STATES.MINTED_ATTACHED);
        assert.equal(placementFromPotAction('attached'), POT_PLACEMENT.ATTACHED);
    });

    it('minted-stripped: a good Web token was withheld from a non-web client', () => {
        const r = mintedReport({ action: 'stripped', winner: 'IOS' });
        assert.equal(classifyMintOutcome(r), MINT_STATES.MINTED_STRIPPED);
        // The distinction that must survive to the report's first token: the
        // mint SUCCEEDED. Nothing here says "failed".
        const line = formatMintReportLine(r);
        assert.match(line, /state=minted-stripped/);
        assert.match(line, /outcome=succeeded/);
        assert.ok(!/failedAt=/.test(line), line);
    });

    it('minted-stripped is reached from `no-token` too, not only from `stripped`', () => {
        // `applyPoTokenToUrl` returns `no-token` when there was no `pot` on the
        // url to remove, and `stripped` when there was. Both mean the same thing
        // to the owner and must land in the same state.
        assert.equal(classifyMintOutcome(mintedReport({ action: 'no-token' })), MINT_STATES.MINTED_STRIPPED);
        assert.equal(classifyMintOutcome(mintedReport({ action: 'not-applicable' })), MINT_STATES.MINTED_STRIPPED);
    });

    it('a cache hit is a minted token, not a not-attempted run', () => {
        // A 6h-old proof is a Web-bound token, so the placement rules apply to
        // it exactly as they do to a fresh one. Classifying this as
        // not-attempted would tell the owner "we never mint here", which is
        // false and is the sort of belief that then survives two releases.
        const r = mintedReport({ source: 'cache', action: 'attached', winner: 'WEB' });
        r.mint.attempted = false; // no mint ran *this* resolve
        assert.equal(classifyMintOutcome(r), MINT_STATES.MINTED_ATTACHED);
    });

    it('user-token: a Settings token is never reported as our mint result', () => {
        const r = createMintReport({ tokenSource: 'user' });
        recordNotAttempted(r, 'a token was supplied by the caller (Settings)');
        recordPotApply(r, { action: 'attached' }, 'IOS');
        assert.equal(classifyMintOutcome(r), MINT_STATES.USER_TOKEN);
    });

    it('minted-not-evaluated: a token whose fate the url step never decided', () => {
        // The resolve died before `applyPoTokenToUrl` ran. Reporting this as
        // `minted-stripped` would assert a deliberate withhold that never
        // happened, and would point the owner at `pot_scope.js` when the cause
        // is upstream (no format, every client UNPLAYABLE).
        const r = mintedReport();
        delete r.pot;
        r.steps = r.steps.filter((s) => s.step !== 'pot-apply');
        assert.equal(classifyMintOutcome(r), MINT_STATES.MINTED_NOT_EVALUATED);
        // …and the one-liner names the dead step rather than inventing a
        // placement story.
        assert.match(formatMintReportLine(r), /state=minted-not-evaluated/);
        assert.ok(!/pot=/.test(formatMintReportLine(r)), formatMintReportLine(r));
    });

    it('a malformed report says unknown rather than guessing', () => {
        assert.equal(classifyMintOutcome(null), MINT_STATES.UNKNOWN);
        assert.equal(classifyMintOutcome({}), MINT_STATES.UNKNOWN);
        // Ran, but the outcome was never recorded: we cannot tell, so we say so
        // rather than defaulting to "failed".
        assert.equal(classifyMintOutcome({ mint: { attempted: true, tokenSource: 'none' } }), MINT_STATES.UNKNOWN);
        assert.equal(classifyMintOutcome({ mint: { attempted: true, outcome: 'failed', tokenSource: 'none' } }), MINT_STATES.ATTEMPTED_FAILED);
    });
});

describe('formatMintReport — the copyable block', () => {
    it('puts the failing step and its message on the one-liner', () => {
        // The one-liner is what the download row shows, and it is the only
        // thing the owner reads before pressing "Copy report". If it does not
        // name the step, the whole instrument is a step list nobody opens.
        const r = createMintReport();
        recordStep(r, 'bgutils-import', true, 'loaded');
        recordStep(r, 'interpreter-fetch', false, 'no interpreter bytes from either transport (webview-fetch)');
        recordMintOutcome(r, 'failed');
        const line = formatMintReportLine(r);
        assert.match(line, /failedAt=interpreter-fetch/);
        assert.match(line, /interpreter-fetch: no interpreter bytes/);
        // The failedAt tail, specifically — not the "first problem" fallback,
        // which formats the same words and would satisfy the line above.
        assert.ok(!/first problem at/.test(line), line);
    });

    it('puts the first dead step on the one-liner of a run that succeeded anyway', () => {
        // The case that makes the whole exercise necessary: the run produced a
        // proof, so "succeeded" is true — and something on the way there was
        // broken. A bare "succeeded" would be read as "minting works in this
        // WebView". `visitor-data` is the realistic non-fatal failure: no
        // visitorData to bind the proof to, so the edge may refuse it later, and
        // nothing at mint time says so.
        const r = createMintReport();
        recordStep(r, 'visitor-data', false, 'NO visitorData anywhere (session or challenge) — a proof minted now cannot be bound to this InnerTube session', { fatal: false });
        recordStep(r, 'cold-start-fallback', true, 'cold-start token produced — no BotGuard involvement');
        recordMintOutcome(r, 'succeeded', { token: 'x'.repeat(40), source: 'minted' });
        r.mint.proofKind = 'cold-start';
        const line = formatMintReportLine(r);
        assert.equal(r.mint.failedAt, null, 'a non-fatal problem must not claim failedAt');
        assert.match(line, /state=minted-not-evaluated/);
        assert.match(line, /proof=cold-start/);
        assert.match(line, /first problem at visitor-data: NO visitorData/);
    });

    it('marks every step after the first failure as not reached', () => {
        // The most useful single line in the report: it names the step worth
        // fixing instead of the pile of steps that merely inherited the failure.
        const r = createMintReport();
        recordStep(r, 'attestation-challenge', true, 'bg_challenge received');
        recordStep(r, 'visitor-data', true, 'vd ok');
        recordStep(r, 'interpreter-fetch', false, 'no bytes from either transport');
        recordMintOutcome(r, 'failed');
        const out = formatMintReport(r);
        assert.match(out, /mint: attempted-failed/);
        assert.match(out, /FAIL interpreter-fetch/);
        assert.match(out, /- {2,}new-function-eval {2,}not reached/);
        assert.match(out, /- {2,}mint {2,}not reached/);
        // A step that merely never ran is distinguished from one that was
        // skipped because of an earlier failure.
        assert.match(out, /- {2,}page-context-probe {2,}not recorded/);
    });

    it('renders a withhold as `held`, not as a failure', () => {
        const out = formatMintReport(mintedReport({ action: 'stripped', winner: 'IOS' }));
        assert.match(out, /held pot-apply {2}/);
        assert.ok(!/FAIL/.test(out), out);
        assert.match(out, /platform-bound/);
    });

    it('includes the page probe as data', () => {
        const r = createMintReport();
        recordPageProbe(r, runPageContextProbe(globalThis));
        const out = formatMintReport(r);
        assert.match(out, /page-context-probe/);
        assert.match(out, /new Function/);
    });
});

// ── the page-context experiment ──────────────────────────────────────────────

describe('runPageContextProbe — reports data, never a verdict', () => {
    it('returns real numbers for a scope that evaluates normally', () => {
        const p = runPageContextProbe({ Function, TextEncoder, atob, btoa, Uint8Array });
        assert.equal(p.newFunction.ctor, 'function');
        assert.equal(p.newFunction.compiled, true);
        assert.equal(p.newFunction.evaluated, true);
        // 42 and a concatenation, not a constant. A `Function` that compiles to
        // a stub cannot produce both.
        assert.equal(p.newFunction.n, 42);
        assert.equal(p.newFunction.echo, 'auralis-probe');
        assert.equal(p.newFunction.error, null);
    });

    it('reports a refused eval as data instead of throwing', () => {
        // The WebView-without-unsafe-eval case. `new Function` refuses at
        // *construction*; the probe must survive it and keep the message.
        const p = runPageContextProbe({
            Function: function () { throw new Error("Refused to evaluate a string as JavaScript because 'unsafe-eval' is not an allowed source"); },
        });
        assert.equal(p.newFunction.compiled, false);
        assert.equal(p.newFunction.evaluated, false);
        assert.match(p.newFunction.error, /unsafe-eval/);
        assert.match(describeProbe(p), /eval REFUSED/);
    });

    it('distinguishes a mangled eval from a refused one', () => {
        // A `Function` that accepts anything and returns a canned function:
        // `compiled` is true, `evaluated` is true, and the *values* are absent.
        // A probe that only checked "did it throw" would call this success.
        // (`function`, not an arrow: `new` on an arrow throws, which would test
        // the refusal path instead of the mangled one.)
        const p = runPageContextProbe({ Function: function () { return function () { return {}; }; } });
        assert.equal(p.newFunction.compiled, true);
        assert.equal(p.newFunction.evaluated, true);
        assert.equal(p.newFunction.n, null);
        assert.equal(p.newFunction.echo, null);
        assert.match(describeProbe(p), /eval compiled but returned n=null/);
    });

    it('reports a Function that throws when called, not only when compiled', () => {
        const p = runPageContextProbe({ Function: function () { return function () { throw new Error('vm exploded'); }; } });
        assert.equal(p.newFunction.compiled, true);
        assert.equal(p.newFunction.evaluated, false);
        assert.match(p.newFunction.error, /vm exploded/);
    });

    it('invents no verdict for globals: absent ones are named, not failed', () => {
        // Half the list is an uncited hypothesis (see PROBE_GLOBALS), so a
        // missing entry must not be a failure verdict. It must be visible.
        const p = runPageContextProbe({ Function });
        assert.ok(p.missing.includes('document'), 'document is absent in this scope');
        assert.ok(p.missing.includes('navigator'));
        assert.equal(p.globals.Function.type, 'function');
        assert.equal(p.globals.document.type, 'undefined');
        assert.equal(p.checked, Object.keys(p.globals).length);
    });

    it('survives a scope that throws on property access', () => {
        const hostile = new Proxy({}, { get(t, k) { if (k === 'Function') return Function; throw new Error('nope'); } });
        const p = runPageContextProbe(hostile);
        // Reading a global threw: recorded as its own outcome, not a crash.
        assert.ok(p.missing.some((n) => n !== 'Function'));
        assert.equal(p.newFunction.n, 42);
    });

    it('flags an Android WebView user agent as data', () => {
        const p = runPageContextProbe({ Function, navigator: { userAgent: 'Mozilla/5.0 (Linux; Android 14; Pixel 8 Build/UD1A) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/124.0.0.0 Mobile Safari/537.36; wv' } });
        assert.equal(p.looksLikeAndroidWebView, true);
        assert.equal(runPageContextProbe({ Function, navigator: { userAgent: 'Mozilla/5.0 (X11; Linux x86_64) Chrome/124' } }).looksLikeAndroidWebView, false);
    });
});

describe('recordPageProbe — only a cited requirement may be a failure', () => {
    it('is ok when the eval genuinely ran', () => {
        const r = createMintReport();
        recordPageProbe(r, runPageContextProbe({ Function }));
        const e = r.steps.find((s) => s.step === 'page-context-probe');
        assert.equal(e.ok, true);
    });

    it('is FAIL when eval was refused, because po_token.js:195 needs it', () => {
        const r = createMintReport();
        recordPageProbe(r, runPageContextProbe({ Function: () => { throw new Error('CSP: unsafe-eval'); } }));
        const e = r.steps.find((s) => s.step === 'page-context-probe');
        assert.equal(e.ok, false);
        assert.equal(r.mint.failedAt, 'page-context-probe');
    });

    it('keeps missing globals as data even when the eval itself worked', () => {
        // Half the probe list is an uncited hypothesis (see PROBE_GLOBALS), so a
        // missing entry must not turn a working eval into a failure verdict. It
        // must be visible, and it must not set `failedAt`.
        const r = createMintReport();
        recordPageProbe(r, runPageContextProbe({ Function }));
        const e = r.steps.find((s) => s.step === 'page-context-probe');
        assert.equal(e.ok, true, 'a working eval is still ok');
        assert.ok(e.missingGlobals.includes('document'));
        assert.equal(r.mint.failedAt, null);
    });

    it('is FAIL when the eval compiled but produced nothing (mangled)', () => {
        const r = createMintReport();
        recordPageProbe(r, runPageContextProbe({ Function: function () { return function () { return {}; }; } }));
        assert.equal(r.steps.find((s) => s.step === 'page-context-probe').ok, false);
    });

    it('is `skip` when there is no usable Function at all, even with globals missing', () => {
        // The remaining third outcome. Without it, "the probe could not run" and
        // "the probe ran and the eval was broken" would collapse into one
        // verdict, and a scope that happens to lack `Function` would be reported
        // as a broken eval.
        const r = createMintReport();
        recordPageProbe(r, runPageContextProbe({ document: {}, navigator: {} }));
        const e = r.steps.find((s) => s.step === 'page-context-probe');
        assert.equal(e.newFunctionCtor, 'undefined');
        assert.equal(e.ok, null);
        assert.equal(r.mint.failedAt, null);
        // The global inventory is still produced either way.
        assert.ok(e.missingGlobals.includes('Function'));
        assert.ok(!e.missingGlobals.includes('document'));
    });
});

// ── the snapshot failure-shape taxonomy ──────────────────────────────────────

describe('classifySnapshotOutcome — the four ways a snapshot can come up empty', () => {
    // The 2026-09-29 device run produced exactly one line for all of these:
    //   "snapshot returned a response and no minter factory — PMD:Undefined"
    // Each test below pins one shape that line was standing in for. They call the
    // real exported function; the fixtures are inputs, not a copy of the logic.

    it('empty-array: a response came back and the VM pushed nothing (the measured device case)', () => {
        const r = classifySnapshotOutcome({ webPoSignalOutput: [], botguardResponse: 'SNAPSHOT-OK' });
        assert.equal(r.shape, SNAPSHOT_SHAPES.EMPTY_ARRAY);
        assert.equal(r.ok, false);
        // The number the old report never printed.
        assert.equal(r.facts.signalLength, 0);
        assert.equal(r.facts.responseLen, 11);
        assert.match(r.detail, /length=0/);
        // …and the reason a run with this shape must not try to "fix" it by
        // minting the integrity token first. In `note`, not `detail`: the
        // measurement has a 120-char budget and this does not fit in it.
        assert.match(r.note, /GenerateIT could not be run first/);
        assert.match(r.note, /\[requestKey, botguardResponse\]/);
    });

    it('empty-array after a bounded wait says so, and is not the same as a late push', () => {
        // The experiment's whole point: "never pushed" and "pushed too late to be
        // seen" are different bugs, and only the re-read tells them apart.
        const stale = classifySnapshotOutcome({
            webPoSignalOutput: [],
            botguardResponse: 'SNAPSHOT-OK',
            settle: { settleGrew: false, settleWaitedMs: 600 },
        });
        assert.equal(stale.shape, SNAPSHOT_SHAPES.EMPTY_ARRAY);
        assert.match(stale.detail, /still 0 after waiting 600ms/);
        const late = classifySnapshotOutcome({
            webPoSignalOutput: [() => {}],
            botguardResponse: 'SNAPSHOT-OK',
            settle: { settleGrew: true, settleWaitedMs: 80 },
        });
        assert.equal(late.shape, SNAPSHOT_SHAPES.OK);
        assert.equal(late.ok, true);
        assert.equal(late.facts.settleGrew, true);
    });

    it('non-function: something WAS pushed, and that is a different bug with a different fix', () => {
        // THE conflation. The old `ok` flag was
        // `botguardResponse && webPoSignalOutput.length`, so this case was
        // recorded as `ok: true, "and a minter factory"` — a positive lie — and
        // the run then died at the `mint` step instead. bgutils agrees this is
        // not PMD:Undefined: `!getMinter` (WebPoMinter.js:17) passes for a truthy
        // non-function, which then throws when called on line 21.
        const r = classifySnapshotOutcome({ webPoSignalOutput: ['not-a-function'], botguardResponse: 'SNAPSHOT-OK' });
        assert.equal(r.shape, SNAPSHOT_SHAPES.NON_FUNCTION);
        assert.equal(r.ok, false, 'a non-function is not a minter factory');
        assert.equal(r.facts.signalLength, 1);
        assert.equal(r.facts.signalTypes[0], '0:string');
        assert.match(r.detail, /\[0\] is 0:string/);
        assert.match(r.note, /NOT the PMD:Undefined case/);
    });

    it('no-response: the VM never called its completion callback at all', () => {
        // Distinct from empty-array: here there is no response to report on, so
        // the 3s race in BotGuardClient.js:131 resolved/rejected without the VM
        // ever calling back. The old ternary printed "returned nothing" for this
        // and "returned a response" for empty-array only by accident of which
        // branch it took.
        const r = classifySnapshotOutcome({ webPoSignalOutput: [], botguardResponse: null, snapshotError: null });
        assert.equal(r.shape, SNAPSHOT_SHAPES.NO_RESPONSE);
        assert.equal(r.ok, false);
        assert.match(r.detail, /never called its completion callback/);
    });

    it('threw: the snapshot error is carried verbatim', () => {
        const r = classifySnapshotOutcome({ webPoSignalOutput: [], botguardResponse: null, snapshotError: 'VM operation timed out' });
        assert.equal(r.shape, SNAPSHOT_SHAPES.THREW);
        assert.equal(r.ok, false);
        assert.match(r.detail, /VM operation timed out/);
    });

    it('unsupported: no snapshot method at all is its own shape, not a throw', () => {
        const r = classifySnapshotOutcome({ webPoSignalOutput: [], botguardResponse: null, unsupported: true });
        assert.equal(r.shape, SNAPSHOT_SHAPES.UNSUPPORTED);
        assert.match(r.detail, /neither snapshot\(\) nor snapshotSynchronous\(\)/);
    });

    it('ok: a function at [0] with a response, and the array is measured not assumed', () => {
        const factory = () => {};
        const r = classifySnapshotOutcome({ webPoSignalOutput: [factory, 'extra'], botguardResponse: 'SNAPSHOT-OK' });
        assert.equal(r.shape, SNAPSHOT_SHAPES.OK);
        assert.equal(r.ok, true);
        assert.equal(r.facts.signalLength, 2);
        assert.deepEqual(r.facts.signalTypes, ['0:function', '1:string']);
    });

    it('precedence: a throw outranks a missing response, which outranks the array contents', () => {
        // Each of these would individually be a failure. Reporting the array
        // first would blame the VM's push for a call that never happened.
        assert.equal(classifySnapshotOutcome({ webPoSignalOutput: [], botguardResponse: 'ok', snapshotError: 'boom' }).shape, SNAPSHOT_SHAPES.THREW);
        assert.equal(classifySnapshotOutcome({ webPoSignalOutput: [], botguardResponse: null, unsupported: true }).shape, SNAPSHOT_SHAPES.UNSUPPORTED);
        assert.equal(classifySnapshotOutcome({ webPoSignalOutput: [], botguardResponse: 'ok' }).shape, SNAPSHOT_SHAPES.EMPTY_ARRAY);
    });

    it('survives a non-array signal and a throwing length getter without lying', () => {
        // The VM wrote this object; it is not ours to trust. A classifier that
        // threw here would take the whole mint down with it, replacing a
        // diagnosis with a stack trace.
        const hostile = { get length() { throw new Error('nope'); } };
        const r = classifySnapshotOutcome({ webPoSignalOutput: hostile, botguardResponse: 'ok' });
        assert.equal(r.facts.signalIsArray, false);
        assert.equal(r.facts.signalLength, null);
        // Not `ok`, and not a shape that claims the VM pushed nothing either.
        assert.equal(r.ok, false);
        assert.equal(r.shape, SNAPSHOT_SHAPES.NON_FUNCTION);
    });
});

// ── the real code, driven end to end ─────────────────────────────────────────

/**
 * Fake `window` + `fetch`. Installed on `globalThis` because `po_token.js`
 * reaches for the bare `fetch` and `window.fetch` identifiers, and because a
 * test that lets a real request escape to youtube.com is not a test.
 */
function installFakeWindow(scriptBody, generateItBody) {
    const calls = [];
    const stub = async (input, init = {}) => {
        const url = typeof input === 'string' ? input : String(input?.url || input);
        calls.push({ url, method: init?.method || 'GET' });
        if (url.includes('GenerateIT')) {
            if (generateItBody === null) return new Response('nope', { status: 400, statusText: 'Bad Request' });
            return new Response(generateItBody, { status: 200, headers: { 'content-type': 'application/json' } });
        }
        if (url.includes('interpreter') || url.includes('botguard')) {
            if (scriptBody === null) return new Response('', { status: 403, statusText: 'Forbidden' });
            return new Response(scriptBody, { status: 200 });
        }
        return new Response('', { status: 404, statusText: 'Not Found' });
    };
    globalThis.window = { fetch: stub, __TAURI__: undefined, __TAURI_INTERNALS__: undefined, Auralis: undefined };
    globalThis.fetch = stub;
    return calls;
}

/** A bg_challenge shaped like the one InnerTube actually returns. */
function challenge({ globalName = 'auralis_bg', interpreterUrl = 'https://www.google.com/botguard/interpreter.js' } = {}) {
    return {
        visitor_data: 'CgtMT0NhbFZpc2l0b3JkYXRh',
        bg_challenge: {
            interpreter_url: { private_do_not_access_or_else_trusted_resource_url_wrapped_value: interpreterUrl },
            program: 'PROGRAM-BYTES',
            global_name: globalName,
            interpreter_hash: 'deadbeef',
        },
    };
}

function fakeInnertube(ch) {
    return {
        session: { context: { client: { visitorData: 'CgtMT0NhbFZpc2l0b3JkYXRh' } } },
        getAttestationChallenge: async () => ch,
    };
}

/** An interpreter that installs a VM whose minter factory returns a proof. */
const GLOBAL_ASSIGNING_INTERPRETER = (NAME) => `
globalThis.${NAME} = {
    a: function (program, setup, sync, userEl, telemetry, extra, undef, flag, loggers) {
        setup(
            function (done, args) {
                // WebPoMinter's factory is smuggled out through the
                // webPoSignalOutput array the caller passed in.
                const out = args[2];
                out.push(function (integrityU8) {
                    return function (contentBindingBytes) {
                        return new Uint8Array([1, 2, 3, contentBindingBytes.length]);
                    };
                });
                done('SNAPSHOT-OK');
                return ['SNAPSHOT-OK'];
            },
            function () {}, function () {}, function () {}
        );
        return [function () { return ['SYNC-SNAPSHOT']; }];
    }
};
`;

/** Same, but the minter callback returns something that is not a Uint8Array,
 *  so the direct-mint fallback yields nothing and the cold-start path runs. */
const USELESS_MINTER_INTERPRETER = (NAME) => `
globalThis.${NAME} = {
    a: function (program, setup) {
        setup(
            function (done, args) {
                args[2].push(function () { return function () { return 'not-a-uint8array'; }; });
                done('SNAPSHOT-OK');
                return ['SNAPSHOT-OK'];
            },
            function () {}, function () {}, function () {}
        );
        return [function () { return []; }];
    }
};
`;

/** An interpreter that DECLARES the VM — the `new Function` scoping trap. */
const VAR_DECLARING_INTERPRETER = (NAME) => `
var ${NAME} = {
    a: function (program, setup) { setup(function () {}, function () {}, function () {}, function () {}); return [function () { return []; }]; }
};
`;

const GOOD_INTEGRITY = () => JSON.stringify([btoa('integrity'), 7200, 1800, null]);

/** A minter factory good enough for a real `WebPoMinter.create` + mint. */
const WORKING_FACTORY_BODY = `
    return function (contentBindingBytes) {
        return new Uint8Array([1, 2, 3, contentBindingBytes.length]);
    };
`;

/**
 * The 2026-09-29 device shape: the VM answers the snapshot and pushes nothing.
 *
 * Models a VM that called its completion callback and left `webPoSignalOutput`
 * empty — the only behaviour the device run could distinguish from a dozen
 * others. Whether Google's live blob behaves this way is NOT claimed; what is
 * claimed is that when it does, the report now says so in words.
 */
const NON_PUSHING_INTERPRETER = (NAME) => `
globalThis.${NAME} = {
    a: function (program, setup) {
        setup(
            function (done, args) {
                done('SNAPSHOT-OK');
                return ['SNAPSHOT-OK'];
            },
            function () {}, function () {}, function () {}
        );
        return [function () { return []; }];
    }
};
`;

/**
 * A VM that pushes the factory AFTER calling its completion callback.
 *
 * This is the race the bounded settle exists to rule out, and it is a plausible
 * reading of the contract rather than an observed one: `webPoSignalOutput` is
 * passed by reference into the VM while `snapshot()` resolves from a callback
 * the VM invokes (BotGuardClient.js:150-158), and nothing there orders the push
 * before the callback. If Google's blob does this, the device trace's
 * `webPoSignalOutput.length === 0` is our read racing its write.
 */
const LATE_PUSHING_INTERPRETER = (NAME, DELAY) => `
globalThis.${NAME} = {
    a: function (program, setup) {
        setup(
            function (done, args) {
                const out = args[2];
                done('SNAPSHOT-OK');
                setTimeout(function () {
                    out.push(function (integrityU8) { ${WORKING_FACTORY_BODY} });
                }, ${DELAY});
                return ['SNAPSHOT-OK'];
            },
            function () {}, function () {}, function () {}
        );
        return [function () { return []; }];
    }
};
`;

/** A VM that pushes a truthy value that is not a function. */
const NON_FUNCTION_PUSHING_INTERPRETER = (NAME) => `
globalThis.${NAME} = {
    a: function (program, setup) {
        setup(
            function (done, args) {
                args[2].push({ notAFunction: true });
                done('SNAPSHOT-OK');
                return ['SNAPSHOT-OK'];
            },
            function () {}, function () {}, function () {}
        );
        return [function () { return []; }];
    }
};
`;

describe('po_token.js end to end — the real module, the real vendored bgutils', () => {
    let po;

    before(async () => {
        po = await import('../../ui/js/modules/po_token.js');
    });

    it('reaches minted-attached when the whole path works', async () => {
        // Everything below is the shipped code path with a stubbed network: the
        // real `loadBgUtils` importing the real `ui/vendor/bgutils`, the real
        // `BotGuardClient` reading the global our interpreter installed, the
        // real `WebPoMinter` base64-encoding a proof. If this passes, the
        // success path is genuinely wired — which is the other half of "can the
        // WebView mint": the instrumentation must be able to say yes.
        installFakeWindow(GLOBAL_ASSIGNING_INTERPRETER('bg_ok1'), GOOD_INTEGRITY());
        const report = createMintReport({ videoId: 'yF9nmg_jHNs' });
        const out = await po.generatePoTokenForVideo(fakeInnertube(challenge({ globalName: 'bg_ok1' })), 'yF9nmg_jHNs', report);

        assert.ok(out?.poToken, 'expected a proof');
        assert.equal(report.mint.outcome, 'succeeded');
        assert.equal(report.mint.failedAt, null, formatMintReport(report));
        // The page-context experiment is recorded by the mint path itself, not
        // only on demand: it is the cheapest step and the one that answers the
        // WebView question, so a report that lacks it would be silent about the
        // thing it exists to find out.
        assert.ok(report.page, 'runPageContextProbe must run inside the mint path');
        assert.equal(report.page.newFunction.n, 42);
        assert.equal(report.steps.find((s) => s.step === 'page-context-probe').ok, true);
        // A full WebPO proof, not the cold-start or the no-integrity-token one:
        // this is the only configuration that proves BotGuard actually ran.
        assert.equal(report.mint.proofKind, 'webpo');
        for (const id of ['bgutils-import', 'attestation-challenge', 'visitor-data', 'interpreter-url', 'interpreter-fetch', 'new-function-eval', 'botguard-global', 'botguard-load', 'snapshot', 'generate-it', 'mint']) {
            const entries = report.steps.filter((s) => s.step === id);
            assert.ok(entries.length, `step ${id} was never recorded`);
            assert.equal(entries[0].ok, true, `step ${id} not ok: ${formatMintReport(report)}`);
        }
        // Minter created, attached, and written to the visitorData-bound cache.
        recordPotApply(report, { action: 'attached' }, 'MWEB');
        assert.equal(classifyMintOutcome(report), MINT_STATES.MINTED_ATTACHED);
        po.setCachedPoToken('yF9nmg_jHNs', out, report);
        assert.equal(report.steps.find((s) => s.step === 'cache-write').ok, true);
    });

    it('CATCHES the trap: eval succeeds, and the WebPO path still cannot start', async () => {
        // This is the case the old instrumentation could not see at all.
        //
        // `po_token.js:195` runs the interpreter through `new Function(body)`.
        // A body that declares its VM with `var` binds into that function's own
        // scope, NOT globalThis — so the eval returns cleanly and
        // `BotGuardClient` (BotGuardClient.js:22,34-37) finds nothing and throws
        // `EGOU: BotGuard unavailable`. Previously that error was swallowed by a
        // bare `catch (_) {}`, and its only other symptom was a 3-second
        // `VM operation timed out` that named neither the step nor the cause.
        //
        // Whether Google's real interpreter blob declares or assigns is a
        // property of a script we cannot read until we download it — which is
        // exactly why this is measured on the device rather than argued here.
        installFakeWindow(VAR_DECLARING_INTERPRETER('bg_trap1'), GOOD_INTEGRITY());
        const report = createMintReport();
        const out = await po.generatePoTokenForVideo(fakeInnertube(challenge({ globalName: 'bg_trap1' })), 'trapvideo1', report);

        const evalStep = report.steps.find((s) => s.step === 'new-function-eval');
        const globalStep = report.steps.find((s) => s.step === 'botguard-global');
        assert.equal(evalStep.ok, true, 'the eval genuinely succeeded — that is the trap');
        assert.equal(globalStep.ok, false, 'and yet there is no global for BotGuardClient');
        assert.match(globalStep.detail, /undefined/);
        // The real vendored BotGuardClient raised it, and it is now visible.
        const loadStep = report.steps.find((s) => s.step === 'botguard-load');
        assert.equal(loadStep.ok, false);
        assert.match(loadStep.error, /EGOU|BotGuard unavailable/);
        // The snapshot then burned its full 3s VM timeout, and bgutils' cold-start
        // helper produced a proof anyway — so the run REPORTS success while the
        // entire BotGuard path is dead. That is precisely why the report carries
        // a step list and a `proofKind`: "succeeded" on its own would have been
        // read as "BotGuard works in this WebView".
        assert.equal(report.mint.outcome, 'succeeded');
        assert.equal(report.mint.proofKind, 'cold-start');
        assert.equal(report.steps.find((s) => s.step === 'snapshot').ok, false);
        assert.ok(out?.poToken);
        // …and the one-liner must still name the dead step.
        const line = formatMintReportLine(report);
        assert.match(line, /proof=cold-start/);
        assert.match(line, /botguard-global/);
    });

    it('reports a reused global as a skip, not as a successful eval', async () => {
        // `po_token.js:192` only evaluates when the global is absent. Without a
        // distinct step outcome, a run that skipped the eval is indistinguishable
        // from one that ran it — which matters because the skip is exactly what
        // a stale global from a previous resolve looks like.
        installFakeWindow(GLOBAL_ASSIGNING_INTERPRETER('bg_stale1'), GOOD_INTEGRITY());
        await po.generatePoTokenForVideo(fakeInnertube(challenge({ globalName: 'bg_stale1' })), 'firstrun1', createMintReport());
        const report = createMintReport();
        await po.generatePoTokenForVideo(fakeInnertube(challenge({ globalName: 'bg_stale1' })), 'secondrun', report);
        const e = report.steps.find((s) => s.step === 'new-function-eval');
        assert.equal(e.ok, null, 'a skipped eval is not a successful one');
        assert.match(e.detail, /already exists from an earlier resolve/);
        // …and the global is still verified, because it is still what
        // BotGuardClient will use.
        assert.equal(report.steps.find((s) => s.step === 'botguard-global').ok, true);
    });

    it('names the interpreter fetch as the failure when nothing can be downloaded', async () => {
        installFakeWindow(null, null);
        const report = createMintReport();
        const out = await po.generatePoTokenForVideo(fakeInnertube(challenge({ globalName: 'bg_dl1' })), 'deadbeef11', report);
        assert.equal(out, null);
        const f = firstFailure(report);
        assert.equal(f.step, 'interpreter-fetch');
        assert.match(f.detail, /no interpreter bytes from either transport/);
        // …and it is the step the mint is attributed to, not merely one that
        // logged a complaint. `failedAt` is what the report leads with.
        assert.equal(report.mint.failedAt, 'interpreter-fetch');
        assert.equal(report.mint.attempted, true);
        assert.equal(classifyMintOutcome(report), MINT_STATES.ATTEMPTED_FAILED);
        // The transport is named, because an allowlisted-away `http_fetch`
        // degrades silently to the WebView fetch and then meets CORS.
        assert.match(formatMintReport(report), /webview-fetch/);
    });

    it('names the challenge as the failure when InnerTube returns no bg_challenge', async () => {
        installFakeWindow(GLOBAL_ASSIGNING_INTERPRETER('bg_ch1'), GOOD_INTEGRITY());
        const report = createMintReport();
        const itb = {
            session: { context: { client: { visitorData: 'CgtMT0NhbA' } } },
            getAttestationChallenge: async () => { throw new Error('HTTP 429 from attestation'); },
        };
        assert.equal(await po.generatePoTokenForVideo(itb, 'ratelimit1', report), null);
        const f = firstFailure(report);
        assert.equal(f.step, 'attestation-challenge');
        assert.match(f.detail, /getAttestationChallenge threw/);
        // Entering the mint at all is an attempt. Reporting this as
        // "not-attempted" would tell the owner to look at Settings when the
        // actual answer is that InnerTube refused us.
        assert.equal(report.mint.attempted, true);
        assert.match(formatMintReportLine(report), /state=attempted-failed/);
    });

    it('reports a missing visitorData as its own step, without calling it the mint failure', async () => {
        // Independently failable AND consequential: a proof minted with no
        // visitorData to bind to is rejected when presented, which surfaces much
        // later as an inexplicable token failure.
        installFakeWindow(GLOBAL_ASSIGNING_INTERPRETER('bg_vd1'), GOOD_INTEGRITY());
        const report = createMintReport();
        const itb = {
            session: { context: { client: {} } },
            getAttestationChallenge: async () => ({ bg_challenge: {} }),
        };
        assert.equal(await po.generatePoTokenForVideo(itb, 'novisitor1', report), null);
        const vd = report.steps.find((s) => s.step === 'visitor-data');
        assert.equal(vd.ok, false);
        assert.match(vd.detail, /NO visitorData/);
        // The step that actually ended the run is still the one named.
        assert.equal(report.mint.failedAt, 'interpreter-url');
    });

    it('falls back to a cold-start token when every WebPO proof route fails', async () => {
        // The real fallback chain, and it is longer than it looks:
        //   1. GenerateIT (jnn-pa, then the youtube.com endpoint) for the
        //      integrity token — refused here.
        //   2. The "direct mint" path, which calls the minter factory with an
        //      EMPTY integrity token (bgutils' own `WebPoMinter.create` refuses
        //      to do this, WebPoMinter.js:19-20) — yields nothing usable here.
        //   3. bgutils' cold-start helper, which needs no BotGuard at all.
        // The report has to show all three, because "succeeded" on its own would
        // read as "BotGuard works in this WebView".
        const calls = installFakeWindow(USELESS_MINTER_INTERPRETER('bg_it1'), null);
        const report = createMintReport();
        const out = await po.generatePoTokenForVideo(fakeInnertube(challenge({ globalName: 'bg_it1' })), 'coldstart1', report);
        assert.ok(out?.poToken, 'cold-start token should rescue the run');
        // Both GenerateIT endpoints were attempted (jnn-pa, then youtube.com)
        // but reported as ONE step: two entries would let the first refusal
        // claim `failedAt` even when the second had succeeded.
        assert.equal(calls.filter((c) => c.url.includes('GenerateIT')).length, 2);
        const git = report.steps.filter((s) => s.step === 'generate-it');
        assert.equal(git.length, 1);
        assert.equal(git[0].ok, false);
        assert.equal(git[0].status, 400);
        assert.match(git[0].detail, /refused the request \(status 400/);
        // snapshot succeeded, so the failure really is downstream of it.
        assert.equal(report.steps.find((s) => s.step === 'snapshot').ok, true);
        const cold = report.steps.find((s) => s.step === 'cold-start-fallback');
        assert.equal(cold.ok, true);
        assert.match(cold.detail, /no BotGuard/);
        assert.equal(report.mint.outcome, 'succeeded');
        assert.equal(report.mint.proofKind, 'cold-start');
        // failedAt survives a success on purpose: "the WebPO path died at
        // GenerateIT and a cold-start token rescued it" is the whole story.
        assert.equal(report.mint.failedAt, 'generate-it');
        recordPotApply(report, { action: 'no-token' }, 'IOS');
        assert.equal(classifyMintOutcome(report), MINT_STATES.MINTED_STRIPPED);
    });

    it('flags a proof minted with no integrity token as `webpo-direct`', async () => {
        // The weakest proof this file can produce. It will be base64 and it will
        // be attached, and the edge is expected to refuse it — so the report
        // says so, instead of letting "succeeded" imply BotGuard attested.
        installFakeWindow(GLOBAL_ASSIGNING_INTERPRETER('bg_dir1'), null);
        const report = createMintReport();
        await po.generatePoTokenForVideo(fakeInnertube(challenge({ globalName: 'bg_dir1' })), 'directmint', report);
        assert.equal(report.mint.proofKind, 'webpo-direct');
        assert.match(formatMintReportLine(report), /proof=webpo-direct/);
        const mintStep = report.steps.filter((s) => s.step === 'mint');
        assert.match(mintStep[0].detail, /NO GenerateIT integrity token/);
    });

    it('records a cache hit as a hit, and a miss as a miss', async () => {
        installFakeWindow(GLOBAL_ASSIGNING_INTERPRETER('bg_cache1'), GOOD_INTEGRITY());
        const miss = createMintReport();
        assert.equal(po.getCachedPoToken('brandnewvid', 'CgtMT0NhbA', miss), null);
        const missStep = miss.steps.find((s) => s.step === 'cache-read');
        assert.equal(missStep.ok, null, 'a miss is not a failure');
        assert.match(missStep.detail, /no live entry/);

        const out = await po.generatePoTokenForVideo(fakeInnertube(challenge({ globalName: 'bg_cache1' })), 'cachehit01', createMintReport());
        po.setCachedPoToken('cachehit01', out);
        const hit = createMintReport();
        assert.ok(po.getCachedPoToken('cachehit01', out.visitorData, hit), 'expected a cache hit');
        const hitStep = hit.steps.find((s) => s.step === 'cache-read');
        assert.equal(hitStep.ok, true);
        assert.equal(hit.mint.tokenSource, 'cache');
        // A hit means we hold a Web-bound token even though nothing was minted.
        recordPotApply(hit, { action: 'stripped' }, 'IOS');
        assert.equal(classifyMintOutcome(hit), MINT_STATES.MINTED_STRIPPED);
    });

    // ── the snapshot shapes, driven through the real vendored BotGuardClient ──
    //
    // These are the tests that would have caught the 2026-09-29 device run being
    // undiagnosable. They install a fake VM on globalThis and let the *shipped*
    // `po_token.js` and the *real* `ui/vendor/bgutils` run over it, so what is
    // asserted is the report the owner would actually receive.

    it('names the device case: the VM answered and pushed nothing', async () => {
        installFakeWindow(NON_PUSHING_INTERPRETER('bg_nopush1'), GOOD_INTEGRITY());
        const report = createMintReport();
        const out = await po.generatePoTokenForVideo(fakeInnertube(challenge({ globalName: 'bg_nopush1' })), 'nopushvid1', report);

        const s = report.steps.find((x) => x.step === 'snapshot');
        assert.equal(s.ok, false, formatMintReport(report));
        assert.equal(s.shape, SNAPSHOT_SHAPES.EMPTY_ARRAY);
        // The measurement the old report never printed. Read through `data`
        // because that is the channel `stepDetail` renders, so these numbers
        // reach the copied report and not only this module's internals.
        assert.equal(s.data.signalLength, 0);
        assert.equal(s.data.responseType, 'string');
        assert.equal(s.data.responseLen, 'SNAPSHOT-OK'.length);
        assert.match(s.detail, /length=0/);
        // The reasoning half must survive into the *copied report*, not just the
        // object: `recordStep` caps `detail` at 120 chars, and the sentence that
        // stops the next device run being spent on the reordering theory is
        // longer than that. It is asserted through the renderer for that reason.
        assert.match(s.note, /GenerateIT could not be run first/);
        assert.match(formatMintReport(report), /GenerateIT could not be run first/);
        // …alongside the numbers, in the block the owner actually copies.
        const rendered = formatMintReport(report);
        assert.match(rendered, /shape=empty-array/);
        assert.match(rendered, /signalLength=0/);
        assert.match(rendered, /settleGrew=false/);

        // GenerateIT is still unreachable, and the report must not pretend we
        // tried: its payload is this very snapshot response.
        assert.equal(report.steps.filter((x) => x.step === 'generate-it').length, 0);
        // The cold-start fallback is the only thing producing a token, so it
        // must still run. Losing it here would be a regression, not a fix.
        assert.ok(out?.poToken, 'the cold-start fallback must still rescue the run');
        assert.equal(report.mint.proofKind, 'cold-start');
        assert.equal(report.mint.outcome, 'succeeded');
        assert.equal(report.mint.failedAt, 'snapshot');
    });

    it('finds a factory the VM pushes AFTER answering, instead of racing it', async () => {
        // The experiment. If the push lands late, the old code read `.length` on
        // the next line, saw 0, and reported the same PMD:Undefined as the
        // "never pushed" case above. The bounded settle separates them — and when
        // it separates them in the direction of "late", the whole WebPO path
        // completes for the first time in this WebView.
        installFakeWindow(LATE_PUSHING_INTERPRETER('bg_late1', 80), GOOD_INTEGRITY());
        const report = createMintReport();
        const out = await po.generatePoTokenForVideo(fakeInnertube(challenge({ globalName: 'bg_late1' })), 'latepush01', report);

        const s = report.steps.find((x) => x.step === 'snapshot');
        assert.equal(s.shape, SNAPSHOT_SHAPES.OK, formatMintReport(report));
        assert.equal(s.ok, true);
        // Proof that the wait is what found it, and how long it cost.
        assert.equal(s.data.settleGrew, true);
        assert.ok(s.data.settleWaitedMs > 0 && s.data.settleWaitedMs < 600, `settleWaitedMs=${s.data.settleWaitedMs}`);
        assert.equal(s.data.signalTypes[0], '0:function');
        // A real WebPO proof — not cold-start, not the no-integrity-token one.
        assert.equal(report.mint.proofKind, 'webpo');
        assert.equal(report.mint.outcome, 'succeeded');
        assert.equal(report.mint.failedAt, null, formatMintReport(report));
        assert.ok(out?.poToken);
        assert.equal(report.steps.find((x) => x.step === 'generate-it').ok, true);
    });

    it('does not wait at all when the factory is already there', async () => {
        // The settle is only allowed to cost anything on a path that has already
        // failed. A working mint must not acquire 600ms of latency.
        installFakeWindow(GLOBAL_ASSIGNING_INTERPRETER('bg_nowait1'), GOOD_INTEGRITY());
        const report = createMintReport();
        await po.generatePoTokenForVideo(fakeInnertube(challenge({ globalName: 'bg_nowait1' })), 'nowaitvid1', report);
        const s = report.steps.find((x) => x.step === 'snapshot');
        assert.equal(s.data.settleWaitedMs, 0, 'a populated array must short-circuit the wait');
        assert.equal(s.data.settleGrew, false);
        assert.equal(s.shape, SNAPSHOT_SHAPES.OK);
    });

    it('reports a truthy non-function as the snapshot failing, not the mint', async () => {
        // The positive lie the old `ok: length>0` flag told: a run in which the
        // array held an object was recorded as "and a minter factory", then died
        // at `mint` with a TypeError that named the wrong stage of BotGuard.
        installFakeWindow(NON_FUNCTION_PUSHING_INTERPRETER('bg_nonfn1'), GOOD_INTEGRITY());
        const report = createMintReport();
        const out = await po.generatePoTokenForVideo(fakeInnertube(challenge({ globalName: 'bg_nonfn1' })), 'nonfuncvid', report);

        const s = report.steps.find((x) => x.step === 'snapshot');
        assert.equal(s.ok, false, formatMintReport(report));
        assert.equal(s.shape, SNAPSHOT_SHAPES.NON_FUNCTION);
        assert.equal(s.data.signalLength, 1);
        assert.equal(s.data.signalTypes[0], '0:object');
        // Attributed to the step that measured it, and GenerateIT is not reached
        // (there is no factory to spend an integrity token on).
        assert.equal(report.mint.failedAt, 'snapshot');
        assert.equal(report.steps.filter((x) => x.step === 'generate-it').length, 0);
        // The direct-mint fallback must not have called a non-function and
        // reported the resulting TypeError as a mint failure.
        assert.equal(report.steps.filter((x) => x.step === 'mint').length, 0);
        // …and the run still ends in a usable proof.
        assert.ok(out?.poToken);
        assert.equal(report.mint.proofKind, 'cold-start');
    });

    it('carries the proof KIND through the 6h cache, so a cold-start hit is visible', async () => {
        // The cache suppresses the mint for 6h, and it will keep doing that. But
        // a cached cold-start token rendering identically to a cached BotGuard
        // one is how six hours of resolves could each look like evidence that
        // minting works. A token is not self-describing; only this field is.
        installFakeWindow(NON_PUSHING_INTERPRETER('bg_cp1'), GOOD_INTEGRITY());
        const cold = createMintReport();
        const minted = await po.generatePoTokenForVideo(fakeInnertube(challenge({ globalName: 'bg_cp1' })), 'cachekind01', cold);
        assert.equal(cold.mint.proofKind, 'cold-start');
        po.setCachedPoToken('cachekind01', minted, cold);

        const hit = createMintReport();
        const entry = po.getCachedPoToken('cachekind01', minted.visitorData, hit);
        assert.ok(entry, 'expected a cache hit');
        assert.equal(hit.mint.proofKind, 'cold-start');
        assert.equal(hit.mint.tokenSource, 'cache');
        // …and it survives into the rendered report, not just the object.
        assert.match(formatMintReportLine(hit), /proof=cold-start/);

        // Second write path, and the reason `setCachedPoToken` normalises
        // `proofKind` at all rather than trusting the spread: a caller that
        // stores a hand-built object (no `proofKind` on it) still gets the kind
        // the mint recorded on its report. Without the normalisation the entry
        // silently loses the field and the next resolve reports `proofKind=
        // null` — back to a cached cold-start token looking like any other.
        const bare = createMintReport();
        bare.mint.proofKind = 'cold-start';
        po.setCachedPoToken('cachekind02', { poToken: 'Y'.repeat(40), visitorData: 'CgtMT0NhbA', contentBinding: 'cachekind02' }, bare);
        const bareHit = createMintReport();
        assert.ok(po.getCachedPoToken('cachekind02', 'CgtMT0NhbA', bareHit), 'expected a cache hit');
        assert.equal(bareHit.mint.proofKind, 'cold-start');

        // The suppression itself is unchanged: a hit means no mint machinery ran
        // at all this resolve. That is the deliberate decision, and this asserts
        // it so a future "let's retry BotGuard on every cache hit" change cannot
        // land silently.
        assert.equal(hit.mint.attempted, null);
        assert.equal(hit.steps.filter((x) => x.step === 'botguard-load').length, 0);
    });

    it('records every step id it uses as a declared step', () => {
        // A wiring invariant, checked against the source text because a step id
        // is a string literal in both files and nothing at runtime links them.
        // Without this, a renamed step renders as "not reached" forever.
        const src = fs.readFileSync(poTokenPath, 'utf8');
        const used = new Set();
        for (const m of src.matchAll(/recordStep\(\s*diag\s*,\s*'([a-z0-9-]+)'/g)) used.add(m[1]);
        assert.ok(used.size >= 10, `expected the mint path to be instrumented, saw ${used.size} steps`);
        for (const id of used) {
            assert.ok(MINT_STEP_IDS.includes(id), `po_token.js records undeclared step "${id}"`);
        }
        // `pot-apply` is owned by youtube.js, and `cache-*` are reachable from
        // both modules — so every declared step must be claimed by someone, or
        // it is a step that will always render as "not reached".
        const declared = new Set([...used, 'pot-apply', 'cache-read', 'cache-write', 'page-context-probe']);
        for (const id of MINT_STEP_IDS) {
            assert.ok(declared.has(id), `step "${id}" is declared but nothing ever records it`);
        }
        assert.equal(new Set(MINT_STEP_IDS).size, MINT_STEP_IDS.length, 'duplicate step id');
        assert.equal(MINT_STEPS.every((s) => typeof s.label === 'string' && s.label.length), true);
    });
});
