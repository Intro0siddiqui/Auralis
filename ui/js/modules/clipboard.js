/**
 * Clipboard copy that actually reports what happened.
 *
 * WHY THIS EXISTS. The three "Copy ..." buttons (client report, download error,
 * resume log) were all written as:
 *
 *     if (navigator.clipboard) { navigator.clipboard.writeText(t).then(...).catch(...) }
 *
 * `navigator.clipboard` is only exposed in a **secure context**, and Tauri's
 * Android WebView does not always provide it. When it is missing the guard is
 * false, the handler returns having done **nothing at all** — no copy, no toast,
 * no error — so the button is a silent no-op. The owner reported exactly that:
 * "the copy button isn't working", with no visible failure to explain it.
 *
 * So: try the async API, fall back to the legacy `execCommand('copy')` path via
 * a throwaway textarea, and **always** resolve to a boolean the caller can turn
 * into a toast. A copy that fails quietly has cost this project real debugging
 * time — a resume diagnostic sat unreadable for three rounds for want of it.
 */

/**
 * Copy `text` to the clipboard.
 *
 * @param {string} text
 * @returns {Promise<boolean>} whether the text actually reached the clipboard.
 */
export async function copyText(text) {
    const value = typeof text === 'string' ? text : String(text ?? '');
    if (!value) return false;

    // 1. The modern API, when the WebView has it.
    try {
        if (typeof navigator !== 'undefined' && navigator.clipboard?.writeText) {
            await navigator.clipboard.writeText(value);
            return true;
        }
    } catch (_) {
        // Permission denied, or no user activation. Fall through — the legacy
        // path often still works where the async one does not, because it does
        // not require a secure context.
    }

    // 2. Legacy fallback. Needs a real selection in a real document, hence the
    //    appended textarea rather than a detached one.
    try {
        if (typeof document === 'undefined' || !document.body) return false;
        const ta = document.createElement('textarea');
        ta.value = value;
        ta.setAttribute('readonly', '');
        ta.setAttribute('aria-hidden', 'true');
        // Off-screen rather than display:none — a hidden element cannot be
        // selected, and the copy silently does nothing.
        ta.style.position = 'fixed';
        ta.style.top = '0';
        ta.style.left = '-9999px';
        ta.style.opacity = '0';
        document.body.appendChild(ta);

        const selection = document.getSelection();
        const previous = selection && selection.rangeCount > 0 ? selection.getRangeAt(0) : null;
        ta.focus();
        ta.select();
        if (typeof ta.setSelectionRange === 'function') {
            ta.setSelectionRange(0, ta.value.length);
        }
        const ok = document.execCommand('copy');
        document.body.removeChild(ta);

        // Put the caret back where the user left it.
        if (previous && selection) {
            selection.removeAllRanges();
            selection.addRange(previous);
        }
        return ok === true;
    } catch (_) {
        return false;
    }
}

/**
 * `copyText` plus a toast, so no call site can silently do nothing again.
 *
 * @param {object} opts
 * @param {string} opts.text        the payload to copy
 * @param {string} opts.label       what was copied, for the success toast
 * @param {Function} opts.showToast bound `showToast` from the owning module
 * @returns {Promise<boolean>}
 */
export async function copyWithToast({ text, label, showToast }) {
    const ok = await copyText(text);
    try {
        if (typeof showToast === 'function') {
            showToast(ok ? `Copied ${label} to clipboard` : 'Copy failed — clipboard unavailable', ok ? 'success' : 'error');
        }
    } catch (_) {
        /* a broken toast must not mask the copy result */
    }
    return ok;
}
