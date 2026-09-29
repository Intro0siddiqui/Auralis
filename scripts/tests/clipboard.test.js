import { test } from 'node:test';
import assert from 'node:assert/strict';

import { copyText, copyWithToast } from '../../ui/js/modules/clipboard.js';

// The bug this guards: the three "Copy ..." buttons were all
// `if (navigator.clipboard) { ... }` with no `else`. In a WebView where the
// clipboard API is absent the handler returned having done nothing — no copy, no
// toast, no error — and the owner reported "the copy button isn't working"
// with nothing on screen to explain it. A regression here is invisible without
// a test, because the symptom is silence.

/** Install a minimal `document` and restore whatever was there before. */
function withDom(stub, fn) {
    const prevDoc = globalThis.document;
    const prevNav = globalThis.navigator;
    globalThis.document = stub;
    if (stub?.__navigator !== undefined) globalThis.navigator = stub.__navigator;
    else delete globalThis.navigator;
    return Promise.resolve()
        .then(fn)
        .finally(() => {
            if (prevDoc === undefined) delete globalThis.document; else globalThis.document = prevDoc;
            if (prevNav === undefined) delete globalThis.navigator; else globalThis.navigator = prevNav;
        });
}

/** A document whose execCommand reports `copied`, recording what was selected. */
function domSucceeding() {
    const state = { copied: null, value: null, removed: false, style: {} };
    const ta = {
        set value(v) { state.value = v; },
        get value() { return state.value; },
        setAttribute() {},
        focus() {}, select() {},
        setSelectionRange() {},
        style: state.style,
    };
    const body = {
        appendChild() {},
        removeChild() { state.removed = true; },
    };
    return {
        state,
        body,
        createElement: () => ta,
        getSelection: () => ({ rangeCount: 0, removeAllRanges() {}, addRange() {} }),
        execCommand: (cmd) => { if (cmd === 'copy') { state.copied = state.value; return true; } return false; },
    };
}

test('copyText falls back to execCommand when navigator.clipboard is absent', async () => {
    // THE regression: clipboard undefined used to mean "do nothing at all".
    await withDom(domSucceeding(), async () => {
        const ok = await copyText('resume log payload');
        assert.equal(ok, true, 'must still copy via the legacy path');
    });
});

test('the legacy path actually receives the text and is removed afterwards', async () => {
    const dom = domSucceeding();
    await withDom(dom, async () => {
        const ok = await copyText('exact payload');
        assert.equal(ok, true);
        assert.equal(dom.state.copied, 'exact payload', 'execCommand must see the text');
        assert.equal(dom.state.removed, true, 'the throwaway textarea must be removed, not left in the DOM');
    });
});

test('the textarea is positioned off-screen, not display:none', async () => {
    // A display:none element cannot be selected, so the copy silently no-ops.
    // This asserts the style guard that keeps the fallback working.
    const dom = domSucceeding();
    await withDom(dom, async () => {
        await copyText('x');
        assert.notEqual(dom.state.style.display, 'none');
        assert.equal(dom.state.style.position, 'fixed');
        assert.equal(dom.state.style.left, '-9999px');
    });
});

test('a present clipboard API is preferred over the legacy path', async () => {
    let usedAsync = false;
    const nav = { clipboard: { writeText: () => { usedAsync = true; return Promise.resolve(); } } };
    await withDom({ ...domSucceeding(), __navigator: nav }, async () => {
        const ok = await copyText('prefer async');
        assert.equal(ok, true);
        assert.equal(usedAsync, true, 'must use navigator.clipboard when it exists');
    });
});

test('a rejecting clipboard API still falls through to the legacy path', async () => {
    // Permission-denied in a WebView is a rejection, not an absence; the old
    // code showed an error and gave up instead of trying the working path.
    const dom = domSucceeding();
    const nav = { clipboard: { writeText: () => Promise.reject(new Error('denied')) } };
    await withDom({ ...dom, __navigator: nav }, async () => {
        const ok = await copyText('after denial');
        assert.equal(ok, true, 'a rejected write must not be the end of the road');
        assert.equal(dom.state.copied, 'after denial');
    });
});

test('a failing execCommand reports false rather than throwing', async () => {
    const dom = { ...domSucceeding(), execCommand: () => false };
    await withDom(dom, async () => {
        assert.equal(await copyText('nope'), false);
    });
});

test('empty input is false and copies nothing', async () => {
    const dom = domSucceeding();
    await withDom(dom, async () => {
        assert.equal(await copyText(''), false);
        assert.equal(dom.state.copied, null, 'must not call execCommand for an empty payload');
    });
});

test('copyWithToast always tells the user something', async () => {
    // The point of the helper: no call site can go quiet again.
    const toasts = [];
    const showToast = (m, k) => toasts.push([m, k]);
    await withDom(domSucceeding(), async () => {
        await copyWithToast({ text: 'a', label: 'client report', showToast });
        await copyWithToast({ text: '', label: 'client report', showToast });
    });
    assert.equal(toasts.length, 2, 'exactly one toast per attempt');
    assert.equal(toasts[0][1], 'success');
    assert.match(toasts[0][0], /client report/);
    assert.equal(toasts[1][1], 'error', 'a failure must not be silent');
});
