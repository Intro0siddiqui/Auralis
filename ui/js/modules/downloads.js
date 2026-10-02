/**
 * Downloads Module
 * Handles download queue, YouTube resolution glue, and sync triggers.
 */

import { copyWithToast } from './clipboard.js';

export const downloadMethods = {
    // Pending download contexts for auto-retry (id -> { resolved, opts, originalUrl, format, key, retryCount, _retrying })
    _pendingDownloadContexts: null,
    // Per-track retry budget (videoId/url -> { attempts, triedClients }).
    // A retry starts a NEW download id, so the "retry once" guard on the id
    // alone is not enough — without this budget a permanently blocked stream
    // would spawn an endless chain of re-resolves.
    _autoRetryBudget: null,
    _downloadRetryListenerBound: false,
    _downloadSubmitListenersBound: false,

    _ensurePendingMap() {
        if (!this._pendingDownloadContexts) this._pendingDownloadContexts = new Map();
        // expose globally so core.js can read same map (both run on same Bridge instance)
        try { window.__auralisPendingDownloadContexts = this._pendingDownloadContexts; } catch (_) {}
        return this._pendingDownloadContexts;
    },

    _ensureRetryBudget() {
        if (!this._autoRetryBudget) this._autoRetryBudget = new Map();
        return this._autoRetryBudget;
    },

    /**
     * Per-client reaction reports from the resolver, kept in a bounded global
     * archive. Release builds write nothing to logcat, so the archive + the
     * "Copy report" button are the only way to see how each InnerTube client
     * behaved (SABR-only / 403 / missing adaptive urls) on this device/network.
     */
    _ensureClientReports() {
        if (!this._clientReports) {
            this._clientReports = [];
            try { window.__auralisClientReports = this._clientReports; } catch (_) {}
        }
        return this._clientReports;
    },

    _recordClientReport(resolved, context = {}) {
        try {
            const list = this._ensureClientReports();
            const entry = {
                at: new Date().toISOString(),
                title: resolved?.title || '',
                videoId: resolved?.videoId || '',
                client: resolved?.client || resolved?.winningClient || null,
                sabrFallback: Boolean(resolved?.sabrFallback),
                selection: resolved?.selection || null,
                report: Array.isArray(resolved?.client_report) ? resolved.client_report : [],
                text: resolved?.client_report_text || '',
                // The PO-token mint report rides in the SAME entry, not a second
                // archive. The two answers are only interpretable together: "no
                // token on the url" means "minting failed" on one run and
                // "a good Web token was correctly withheld from a non-web
                // client" on the next, and those need opposite fixes.
                mint: resolved?.mint_report || null,
                mintText: resolved?.mint_report_text || '',
                mintState: resolved?.mint_state || null,
                context,
            };
            list.push(entry);
            while (list.length > 20) list.shift();
            return entry;
        } catch (_) { return null; }
    },

    /** Human-readable, copyable dump of the most recent client reports. */
    _buildClientReportText() {
        const list = this._ensureClientReports().slice(-5);
        if (!list.length) return 'No InnerTube client report recorded yet.';
        return list.map((e, i) => {
            const sel = e.selection
                ? ` itag=${e.selection.itag} ext=${e.selection.ext} audioOnly=${e.selection.audioOnly ? 'yes' : 'no'}${e.selection.legacyProgressive ? ' legacy' : ''}`
                : '';
            const per = Array.isArray(e.report) && e.report.length
                ? e.report.map((r) => {
                    const bits = [
                        r.chosen ? 'CHOSEN' : null,
                        r.status || r.reason || 'fail',
                        `phase=${r.phase}`,
                        `adaptive=${r.adaptive}`,
                        `progressive=${r.progressive}`,
                        `adaptiveWithUrl=${r.adaptiveWithUrl}`,
                        `audioWithUrl=${r.audioWithUrl}`,
                        // Printed because the one client that has ever completed a
                        // download on the test network served audio through this
                        // field alone, and it was invisible in the report that
                        // would have shown us. A measurement nobody can read is
                        // the same as no measurement.
                        `progressiveWithUrl=${r.progressiveWithUrl ?? 0}`,
                        `audioOnlyWithUrl=${r.audioOnlyWithUrl ?? 0}`,
                        `opusWithUrl=${r.opusWithUrl ?? 0}`,
                        `sabr=${r.sabrStreamingUrl ? 'yes' : 'no'}`,
                        r.ms ? `${r.ms}ms` : null,
                        r.error ? `err=${String(r.error).slice(0, 120)}` : null,
                    ].filter(Boolean).join(' ');
                    return `  - ${r.client}: ${bits}`;
                }).join('\n')
                : '  - (no per-client detail)';
            // The full per-step mint block, verbatim. This is the surface the
            // owner copies when asking "can this WebView mint a PO token?" — a
            // summary line in the download row is not enough, because the
            // answer is which step failed, and that lives in the step list.
            const mint = e.mintText || '';
            return `#${i + 1} ${e.at} "${e.title}" videoId=${e.videoId} client=${e.client} sabrFallback=${e.sabrFallback}${sel}\n${per}${mint ? `\n${mint}` : ''}`;
        }).join('\n\n');
    },

    _ensureDownloadRetryListener() {
        if (this._downloadRetryListenerBound) return;
        this._downloadRetryListenerBound = true;
        // Subscribe to Bridge's download:completed for 403 auto-retry (once, minimal invasive)
        try {
            this.on('download:completed', (p) => {
                if (!p) return;
                // Ensure failed UI is surfaced even when completed carries failed status
                if (p.status === 'failed') {
                    try { this.updateDownloadProgressUI(p); } catch (_) {}
                }
                const map = this._ensurePendingMap();
                if (p.status === 'completed' || p.status === 'cancelled') {
                    const done = map.get(p.id);
                    map.delete(p.id);
                    // Track succeeded/cancelled — the retry chain is over.
                    if (done && done.key) {
                        try { this._ensureRetryBudget().delete(done.key); } catch (_) {}
                    }
                    try { if (window.__auralisDownloadRetryingIds) window.__auralisDownloadRetryingIds.delete(p.id); } catch (_) {}
                    return;
                }
                // fire-and-forget; internal handler is async
                this._handle403AutoRetry(p).catch((e) => console.warn('[Downloads] 403 auto-retry handler error', e?.message || e));
            });
            // Also handle dedicated download:failed events — surface full error including "unplayable" / "0s duration"
            this.on('download:failed', (p) => {
                if (!p) return;
                try { this.updateDownloadProgressUI({ ...p, status: p.status || 'failed' }); } catch (_) {}
                this._handle403AutoRetry({ ...p, status: 'failed' }).catch((e) => console.warn('[Downloads] 403 auto-retry handler (failed event) error', e?.message || e));
            });
            // Guard: progress may also carry failed status (backend emits progress before completed)
            this.on('download:progress', (p) => {
                if (p && p.status === 'failed') {
                    try { this.updateDownloadProgressUI(p); } catch (_) {}
                }
            });
        } catch (_) {}
    },

    _ensureDownloadSubmitListeners() {
        if (this._downloadSubmitListenersBound) return;
        this._downloadSubmitListenersBound = true;
        document.addEventListener('auralis:submit:download', (e) => {
            const form = (e.detail && e.detail.form) || document.getElementById('download-form');
            this.handleDownloadFormSubmit(e, form);
        });
        document.addEventListener('auralis:submit:search', (e) => {
            const form = (e.detail && e.detail.form) || document.getElementById('youtube-search-form');
            this.handleSearchFormSubmit(e, form);
        });
    },

    // Auto-retry a failed download once per budgeted attempt.
    //  - HTTP 403: rotate to the next InnerTube client (the URL is bound to the
    //    client that produced it, and 2026 Jio/Google edges reject some clients).
    //  - Truncated / interrupted / timed-out streams: re-resolve for a fresh URL.
    //    The Rust layer no longer accepts a partial file as "complete", so this
    //    is the path that turns a half song into a whole one.
    // The budget is keyed per track (videoId/URL), not per download id, because
    // every retry creates a new id.
    async _handle403AutoRetry(p) {
        if (!p || p.status !== 'failed') return;
        const map = this._ensurePendingMap();
        const errRaw = typeof this.extractErrorMessage === 'function'
            ? this.extractErrorMessage(p, '')
            : (p.error || p.error_message || '');
        const is403 = errRaw.includes('403') || errRaw.includes('Forbidden');
        // "Truncated download" is the backend's decoded-length verdict: the
        // transfer finished at 100% of the advertised bytes but the file only
        // holds part of the audio, so this needs a NEW url, not a resume.
        const isTruncated = /Truncated download/i.test(errRaw);
        const isResumable = /Incomplete download|Stream interrupted|timed out|timeout|stalled|ECONNRESET|connection reset|HTTP 5\d\d/i.test(errRaw);
        // A request that never left the device. `reqwest` reports every connect,
        // DNS and TLS-handshake failure with this one prefix and no status code, so
        // it matched nothing above and the gate below bailed out — `map.delete`
        // then `return`, with the retry budget never even consulted.
        //
        // Measured on device 2026-10-02: a download died with
        //   "request failed [rr3---sn-gwpa-wage.googlevideo.com] start_byte=0:
        //    error sending request for url (...)"
        // and stopped after a single attempt with the ladder untouched. That reads
        // as "the retry budget ran out", which is the natural assumption and the
        // wrong one: zero retries were attempted, not three.
        //
        // This is the one failure class the ladder exists for. Nothing was
        // downloaded and nothing was refused, and a different class or client means
        // a different CDN hostname, which may resolve and connect when this one did
        // not. Leaving it out made the ladder structurally blind.
        const isTransport = /error sending request|connection refused|connection closed|connect error|failed to lookup address|dns error|client error/i.test(errRaw);
        if (!is403 && !isTruncated && !isResumable && !isTransport) {
            map.delete(p.id);
            return;
        }
        const ctx = map.get(p.id);
        if (!ctx || ctx._retrying) return;

        const key = ctx.key || p.id;
        const budgetMap = this._ensureRetryBudget();
        let budget = budgetMap.get(key);
        if (!budget) { budget = { attempts: 0, triedClients: [], triedClasses: [] }; budgetMap.set(key, budget); }
        const MAX_AUTO_RETRIES = 3;
        // This budget, not the client list, is what bounds the retry. Clients
        // that merely came back empty are re-askable (see rotationRank), so the
        // chain could otherwise revisit a client forever; the counter cannot be
        // outrun because it is per track and incremented once per attempt
        // regardless of which client was chosen.
        if (budget.attempts >= MAX_AUTO_RETRIES) {
            budgetMap.delete(key);
            map.delete(p.id);
            console.warn(`[Downloads] auto-retry budget exhausted for ${key} (${MAX_AUTO_RETRIES} attempts) — surfacing the failure to the user`);
            return;
        }

        const resolved = ctx.resolved || {};
        const tried = budget.triedClients;
        // Rotate the InnerTube client for both 403 and truncation: the stream
        // URL is bound to the client that produced it, and a truncated SABR
        // stream from one client often comes back complete from another.
        if (is403 || isTruncated) {
            const failed = resolved.client || resolved.winningClient;
            if (failed && !tried.includes(failed)) tried.push(failed);
        }
        const reportByClient = new Map();
        for (const e of (resolved.client_report || [])) {
            if (e && e.client && !reportByClient.has(e.client)) reportByClient.set(e.client, e);
        }
        // A client's reaction record answers two DIFFERENT questions, and the
        // retry used to conflate them into one boolean:
        //
        //   1. "did THIS attempt hand out a url we can fetch?" — a preference.
        //   2. "can this client EVER hand out a url?" — a disqualification.
        //
        // Only the second may be treated as permanent, and a record can only
        // answer it positively. Just two shapes qualify: a SABR streaming
        // endpoint handed back instead of CDN urls, and a full adaptive format
        // table with not one url on any of it. Both are the resolver's own
        // `sabr-only` / `adaptive-urls-missing` reasons, transcribed from the
        // counters it records them from rather than from the reason strings, so
        // a rename upstream cannot silently flip the classification.
        //
        // A client that answered with NOTHING — UNPLAYABLE, zero formats, no
        // streamingData, a thrown error — has established nothing at all, and
        // the old code read that as permanent. The device report of 2026-09-27
        // is the measurement that makes the distinction load-bearing rather
        // than theoretical: on track hsXKOsnptw4 the SAME client answered a0/p1
        // on one resolve and a30/p1 on the next, minutes apart — audio-with-url
        // went 0 -> 30 for one client on one video. So "resolved empty" is not
        // a property of the client, and this predicate is where that assumption
        // was being spent.
        //
        // It was not merely a mis-ranked candidate either. Rotation is a search
        // over the candidate list, so one empty reading pushed ANDROID below the
        // line; on the next attempt the better clients were already in `tried`,
        // ANDROID was the only one left, the old predicate refused it, and the
        // retry reported a dead end and gave up with an unused client sitting
        // right there. One transient empty result permanently burned a client
        // for the track.
        //
        // So emptiness is now a PREFERENCE, not a veto:
        //   0 — demonstrably servable: its own record has a url on some class
        //   1 — no evidence either way: deferred, not excluded
        //   null — proven dead end: never asked
        //
        // What bounds the re-asking is `MAX_AUTO_RETRIES`. It is per track and
        // is counted independently of how many clients exist, so making clients
        // re-askable cannot make this loop: a skipped client that resolves
        // empty again just fails the next attempt, and the fourth one stops.
        const hasUrlOnAnyClass = (e) => (e.audioWithUrl || 0) > 0 || (e.progressiveWithUrl || 0) > 0;
        // "Has a url" must count the MUXED progressive class, not just adaptive
        // audio. It did not, and that was not a cosmetic gap: on 2026-09-26 the
        // device report showed ANDROID at adaptiveWithUrl=0 / audioWithUrl=0
        // with progressive=1 — and ANDROID was the only client that completed a
        // download, serving the muxed itag 18. Scored on adaptive audio alone it
        // read as a dead end, so the retry rotated IOS -> ANDROID_VR instead,
        // which 403'd the same way, and burned the last attempt.
        //
        // The two disqualifying shapes below are the resolver's OWN definitions
        // of a client that answered but could not serve, transcribed from where
        // it records them (youtube.js, the `sabr-only` / `adaptive-urls-missing`
        // reasons) rather than invented here. The one thing deliberately NOT
        // treated as proof is "described a format and served no url for it" in
        // general: the 2026-09-27 a0/p1 -> a30/p1 measurement on hsXKOsnptw4 has
        // exactly that shape and it flipped. The separating detail is that the
        // flip was progressive-only, so the second condition below is scoped to
        // the ADAPTIVE table rather than counting progressive formats at all.
        const provenUnservable = (e) => {
            if (!e || hasUrlOnAnyClass(e)) return false;
            // `sabr-only`: answered with SABR metadata instead of CDN urls.
            if (e.sabrStreamingUrl) return true;
            // `adaptive-urls-missing`: a real adaptive list, and not one url on
            // any of it. The adapter returned a full format table, so this is a
            // decision about the client rather than an absence of data.
            return (e.adaptive || 0) > 0 && (e.adaptiveWithUrl || 0) === 0;
        };
        const rotationRank = (c) => {
            const e = reportByClient.get(c);
            if (!e) return 0; // no evidence either way — let it try
            if (provenUnservable(e)) return null;
            return hasUrlOnAnyClass(e) ? 0 : 1;
        };
        // A transport failure rotates too. It is a refusal by nobody — the bytes
        // never moved — and the next rung of the ladder is a different CDN hostname,
        // which is precisely what might connect when this one did not. Leaving it
        // out meant `rotate` was false, so the class walk below was skipped and the
        // single attempt stood as the final answer.
        const rotate = (is403 || isTruncated || isTransport);
        // Candidates: the clients after the winner first, then the rest, minus
        // everything already tried for this track.
        const ordered = resolved.orderedClients || [];
        const winIdx = ordered.indexOf(resolved.client || resolved.winningClient);
        const rotated = winIdx >= 0
            ? ordered.slice(winIdx + 1).concat(ordered.slice(0, winIdx))
            : ordered.slice();
        const fromRetry = (resolved.retryClients || []).filter(c => !tried.includes(c));
        const candidates = fromRetry.length ? fromRetry : rotated.filter(c => !tried.includes(c));
        // Split the candidate list by what the report actually established.
        // `deferred` clients answered with nothing on THIS attempt and are
        // therefore still worth asking once the demonstrably-servable ones have
        // been used up. Note what is NOT here: they are not added to `tried` and
        // not put in `excludeClients`. "We have no evidence this client can
        // serve" must not become "refuse to let the resolver use it", or the
        // de-prioritisation would silently harden back into the veto.
        const servable = candidates.filter(c => rotationRank(c) === 0);
        const deferred = candidates.filter(c => rotationRank(c) === 1);

        // Rescue BEFORE rotating away from the client that just failed.
        //
        // Device report 2026-09-26: the one download that succeeded arrived
        // through the muxed progressive itag 18 from ANDROID, while the adaptive
        // audio-only urls (itag 140) from IOS and ANDROID_VR were refused at byte
        // 0. So when the refused url was the adaptive class, the cheapest thing
        // to try is not a different client — it is the OTHER FORMAT from the
        // client we already know resolves. That keeps full quality whenever the
        // adaptive url is servable, because this only runs after a refusal.
        //
        // Gated on the winner actually having a progressive url, which is the
        // measurement this class of bug hides behind: IOS reported
        // `progressive=0` on every attempt, so it has nothing to fall back to
        // and asking it again would just re-resolve the same refused url.
        const winner = resolved.client || resolved.winningClient;
        const sel = resolved.selection || {};
        // The ladder is over CLASSES OF URL, and it is walked by what has already
        // been tried — not by what the last error happened to be.
        //
        // This is a rewrite, and the reason is a device report that showed the
        // previous shape was unreachable. On 2026-09-27, track yF9nmg_jHNs, four
        // attempts:
        //
        //   #1 ANDROID_VR itag=140 adaptive -> 403 @ byte 0
        //   #2 ANDROID_VR itag=18  MUXED    -> 403
        //   #3 IOS        itag=140 adaptive -> 403 @ byte 0
        //   #4 ANDROID    itag=18  MUXED    -> truncated 75s of 216s
        //
        // and `ANDROID_VR ... opusWithUrl=2` was never requested. The old gate
        // reached for opus only on an observed *truncation*, and the only
        // truncation arrived on the last attempt the budget allows — so the one
        // class never tried was structurally unreachable. Two of the three
        // retries had gone to classes already proven bad: #3 re-tried adaptive
        // after it had 403'd twice, and #4 re-tried muxed after #2.
        //
        // So: record the class that failed, and go to the next class nobody has
        // tried. Client rotation becomes the LAST resort rather than a mid-ladder
        // step, because the evidence says a byte-0 403 is not client-specific —
        // adaptive 403'd on ANDROID_VR and IOS from two different hosts, and muxed
        // has never once succeeded on any client.

        const classOf = (s) => {
            if (!s) return 'adaptive';
            if (s.legacyProgressive === true || s.itag === 18) return 'muxed';
            // Opus is `audio/webm; codecs="opus"`. Read from the reported mime
            // rather than a new field on `selection`, so the class is derivable
            // from what the resolver already hands us.
            if (/opus/i.test(String(s.mime || '')) || s.ext === 'webm') return 'opus';
            return 'adaptive';
        };
        // Availability comes from the per-client report, never assumed. IOS
        // reports `progressiveWithUrl: 0` and `opusWithUrl: 0` on this video, so
        // asking it for either would spend an attempt re-resolving a class just
        // proven bad.
        const offers = (c, klass) => {
            const e = reportByClient.get(c) || {};
            if (klass === 'opus') return (e.opusWithUrl || 0) > 0;
            if (klass === 'muxed') return (e.progressiveWithUrl || 0) > 0;
            return (e.audioWithUrl || 0) > 0 || (e.adaptiveWithUrl || 0) > 0;
        };
        // Order, and the evidence behind it — which is NOT what the previous
        // comment claimed. That comment said muxed "has never succeeded: 403 once
        // and truncated twice". Both halves are dead:
        //
        //   - the truncation was our own decoder. rodio read 25.2% of a complete
        //     216.34s file and the completeness gate believed it. Fixed in v2.6.66.
        //   - muxed then completed end to end (94WoNQyK_KY, v2.6.67).
        //
        // Re-measured on the dev box 2026-10-02 — same phone, same residential line
        // as the failing device — on BOTH tracks that 403'd there (Ral6kFSx7ZY and
        // ALclXvd0QCU), muxed itag 18 from ANDROID:
        //
        //     A unpinned            -> HTTP 206 Partial Content
        //     B source-bound        -> HTTP 206             (local_address)
        //     C destination-bound   -> connect failure       (v2.6.68 as shipped)
        //
        // Meanwhile the class this ladder used to try FIRST is the one measured to
        // be refused at byte 0: §4.7.1 recorded adaptive itag 140 and opus itag 251
        // both 403 at byte 0 on this line, and the device reports agree. For
        // ALclXvd0QCU, ANDROID_VR offered 4 audio formats WITH urls (itag 140) and
        // was chosen, while ANDROID offered only the muxed progressive and was not.
        //
        // So the ladder spent attempt #1 on the class that cannot work and left the
        // class that demonstrably can until last. Muxed goes first.
        //
        // What is NOT claimed: that muxed is better. Muxed is video+audio at 360p,
        // and adaptive itag 140 is audio-only and strictly better quality — it is
        // the right first choice wherever it is servable. This order reflects what
        // is servable on THIS network, and should be revisited the day adaptive
        // stops 403ing, because the ladder is cheap to reorder and adaptive is the
        // better file.
        //
        // adaptive-vs-opus below it is unchanged and still an INFERENCE, not a
        // measurement: both are adaptive CDN urls and may share whatever the 403 is
        // bound to.
        const CLASS_ORDER = ['muxed', 'adaptive', 'opus'];

        if (!Array.isArray(budget.triedClasses)) budget.triedClasses = [];
        const failedClass = classOf(sel);
        if (!budget.triedClasses.includes(failedClass)) budget.triedClasses.push(failedClass);

        const nextClass = CLASS_ORDER.find((k) => !budget.triedClasses.includes(k)
            && ordered.some((c) => offers(c, k))) || null;
        const classCandidates = nextClass ? ordered.filter((c) => offers(c, nextClass)) : [];
        // Prefer the client that just resolved — it is the one we know returns
        // formats at all — and fall back to any client the report says offers
        // this class.
        const classClient = classCandidates.length
            ? (classCandidates.includes(winner) ? winner : classCandidates[0])
            : null;

        // Every class has been tried. Rotation is still allowed, but only now:
        // it is a second opinion, not progress, and putting it earlier is what
        // spent two of three retries re-asking classes already refused.
        const exhausted = !nextClass;
        const rotatedClient = (rotate && exhausted)
            ? (servable[0] || deferred[0] || null)
            : null;

        const nextClient = classClient || rotatedClient;
        // Every remaining client is PROVEN unable to hand out an audio url — not
        // merely silent on this attempt. A candidate that merely came back empty
        // keeps the retry alive, because the next resolve of that same client
        // may well return real formats.
        const deadEnd = rotate && !nextClient;
        const allClients = resolved.orderedClients || [];
        // Never exclude every client — that leaves the resolver nothing to try.
        // When re-asking the winner for a different class it must not also be
        // excluded, or the two instructions contradict each other.
        const excludeClients = (tried.length > 0 && tried.length < allClients.length)
            ? (nextClient ? tried.filter((c) => c !== nextClient) : tried.slice())
            : [];
        if (rotate && !nextClient) {
            console.warn(`[Downloads] 403/truncation auto-retry: no client left worth trying for ${p.id} (winning=${resolved.client}, deadEnd=${deadEnd}, triedClasses=${JSON.stringify(budget.triedClasses)})`);
            map.delete(p.id);
            return;
        }

        budget.attempts += 1;
        ctx._retrying = true;
        try { window.__auralisDownloadRetryingIds = window.__auralisDownloadRetryingIds || new Set(); window.__auralisDownloadRetryingIds.add(p.id); } catch (_) {}
        const shortfall = (errRaw.match(/only \d+s of \d+s|received \d+ bytes of \d+/) || [errRaw.split('\n')[0].slice(0, 80)])[0];
        const CLASS_NAME = { adaptive: 'adaptive audio', opus: 'audio-only opus', muxed: 'the muxed fallback' };
        const label = nextClass
            ? `${CLASS_NAME[failedClass] || failedClass} did not work — trying ${CLASS_NAME[nextClass]} from ${nextClient}`
            : isTruncated
            ? `Truncated stream (${shortfall}), every url class has now been tried — re-resolving via ${nextClient || 'another client'}`
            : is403
                ? `403 on ${resolved.client || 'TV'}, every url class has now been tried — retrying with ${nextClient}`
                : `Download incomplete (${shortfall}), retrying`;
        this.showToast(`${label}… (attempt ${budget.attempts}/${MAX_AUTO_RETRIES})`, 'info', 5000);
        console.warn(`[Downloads] auto-retry key=${key} attempt=${budget.attempts}/${MAX_AUTO_RETRIES} id=${p.id} 403=${is403} truncated=${isTruncated} sabr=${!!resolved.sabrFallback} failedClass=${failedClass} triedClasses=${JSON.stringify(budget.triedClasses)} nextClass=${nextClass} nextClient=${nextClient} exclude=${JSON.stringify(excludeClients)}`);
        try {
            const baseOpts = ctx.opts || this.getDownloadOptions(document.getElementById('download-form')) || {};
            const retryOpts = { ...baseOpts };
            if (nextClient) retryOpts.forceClient = nextClient;
            if (excludeClients.length) retryOpts.excludeClients = excludeClients;
            // The force modes are EXCLUSIVE, and every one of them is set on
            // every attempt rather than only when it applies.
            //
            // Device report 2026-09-29, yF9nmg_jHNs: the ladder correctly moved
            // from adaptive to opus, and then asked for opus THREE more times
            // (attempts #2-#4, all `itag=251 ext=webm`) instead of reaching the
            // muxed rung it had computed next. Two defects, both here:
            //
            //   - `ctx.opts` is whatever the previous attempt was called with, so
            //     a flag set for an earlier rung survives into the next one. Only
            //     the `adaptive` branch cleared the others.
            //   - `youtube.js` applies its force blocks in sequence, so a stale
            //     `forceOpusAudio` overwrote the muxed choice the line above had
            //     just made. Last block to run won, silently.
            //
            // So: set both, always, from `nextClass` alone. `null` (rotation) must
            // clear them too, or a rotated re-resolve inherits the last rung's
            // format and the rotation is a no-op.
            retryOpts.forceLegacyProgressive = nextClass === 'muxed';
            retryOpts.forceOpusAudio = nextClass === 'opus';
            // A previous attempt came back short. Refuse the legacy-progressive
            // fallback only for the client that actually truncated — the short
            // stream is a SABR window, not a property of the muxed container,
            // so the same itag from a different client can be complete. Refusing
            // the format outright meant one truncation could stop every client
            // from ever delivering a file.
            if (isTruncated) {
                retryOpts.avoidLegacyProgressive = true;
                retryOpts.truncatedClient = resolved.client || null;
            }
            const originalUrl = ctx.originalUrl || resolved.originalUrl || p.url;
            if (!originalUrl || !window.AuralisYouTube) throw new Error('No original URL/client for retry');
            const reResolved = await window.AuralisYouTube.resolve(originalUrl, retryOpts);
            if (!reResolved || reResolved.kind !== 'track') throw new Error('Re-resolve did not return track');
            console.log(`[Downloads] retry re-resolved ${originalUrl} via ${reResolved.client} -> ${(reResolved.stream_url || '').slice(0, 80)}`);
            await this.downloadResolvedTrack(reResolved, ctx.format || 'm4a', retryOpts, originalUrl);
        } catch (e) {
            const msg = e?.message || String(e);
            console.error(`[Downloads] auto-retry re-resolve failed for ${p.id}:`, msg);
            this.showToast(`Retry failed: ${msg}`, 'error', 6000);
            try {
                if (e && (e.client_report || e.mint_report)) {
                    this._recordClientReport({
                        title: '', videoId: '', client: null,
                        client_report: e.client_report,
                        client_report_text: e.client_report_text || '',
                        mint_report: e.mint_report || null,
                        mint_report_text: e.mint_report_text || '',
                        mint_state: e.mint_state || null,
                    }, { phase: 'auto_retry_resolve_failed', key, attempt: budget.attempts, triedClients: tried.slice() });
                }
            } catch (_) {}
            map.delete(p.id);
            // Drop the budget so a later manual re-download starts fresh.
            budgetMap.delete(key);
            try { if (window.__auralisDownloadRetryingIds) window.__auralisDownloadRetryingIds.delete(p.id); } catch (_) {}
        } finally {
            ctx._retrying = false;
        }
    },

    async ensureSettings() {
        if (this.currentSettings) return this.currentSettings;
        try {
            this.currentSettings = await this.invoke('get_settings');
        } catch (_) {}
        return this.currentSettings;
    },

    getDownloadOptions(form) {
        const containerSelect = form ? form.querySelector('select[name="container"]') : null;
        const qualitySelect = form ? form.querySelector('select[name="quality"]') : null;
        const opts = {
            container: containerSelect ? containerSelect.value : 'auto',
            quality: qualitySelect ? qualitySelect.value : 'best',
        };
        const dl = (this.currentSettings && this.currentSettings.downloads) || {};
        opts.cookie = dl.youtube_cookie || '';
        opts.poToken = dl.youtube_po_token || '';
        return opts;
    },

    buildDownloadPayload(resolved, format) {
        return {
            request: {
                url: resolved.stream_url,
                title: resolved.title,
                // Written into the file's tags by the backend, so the library
                // scanner reports the real artist instead of `Unknown Artist`
                // and the track is not named after its sanitized filename.
                artist: resolved.author || resolved.artist || null,
                album: resolved.album || null,
                platform: resolved.platform,
                format,
                ext: resolved.ext,
                total_bytes: resolved.total_bytes,
                thumbnail: resolved.thumbnail,
                headers: resolved.headers || null,
                expected_duration_secs: resolved.expected_duration_secs || resolved.duration || resolved.duration_secs || null,
            },
        };
    },

    async downloadResolvedTrack(resolved, format, opts = null, originalUrl = null) {
        if (!resolved || resolved.kind !== 'track') throw new Error('Not a downloadable track');
        // Ensure 403 auto-retry listener is bound once
        this._ensureDownloadRetryListener();
        try {
            const payload = this.buildDownloadPayload(resolved, format);
            console.log('[Downloads] Invoking download_audio', { title: resolved.title, url: resolved.stream_url?.slice(0,120), host: (()=>{try{return new URL(resolved.stream_url).host}catch(_){return 'unknown'}})(), headers: Object.keys(resolved.headers||{}), client: resolved.client, orderedClients: resolved.orderedClients, retryClients: resolved.retryClients });
            const result = await this.invoke('download_audio', payload);
            if (result) {
                this.updateDownloadProgressUI(result);
                // Store context for 403 auto-retry: id -> { resolved, opts, originalUrl, format }
                try {
                    const map = this._ensurePendingMap();
                    const ctxOpts = opts || this.getDownloadOptions(document.getElementById('download-form')) || resolved.resolveOpts || {};
                    const ctxUrl = originalUrl || resolved.originalUrl || resolved.stream_url;
                    // Budget key: the track identity, so every retry attempt of
                    // the same song shares one retry budget.
                    const key = String(resolved.videoId || ctxUrl || result.id);
                    map.set(result.id, {
                        resolved: { ...resolved },
                        opts: { ...ctxOpts },
                        originalUrl: ctxUrl,
                        format,
                        key,
                        retryCount: 0,
                        _retrying: false,
                    });
                    // Keep the per-client reaction report for this attempt so the
                    // download row can show it and "Copy report" can hand it over.
                    this._recordClientReport(resolved, {
                        phase: 'download_audio',
                        id: result.id,
                        key,
                        retry: Boolean(ctxOpts && ctxOpts.forceClient),
                        forceClient: ctxOpts?.forceClient || null,
                        excludeClients: ctxOpts?.excludeClients || null,
                        avoidLegacyProgressive: Boolean(ctxOpts?.avoidLegacyProgressive),
                        truncatedClient: ctxOpts?.truncatedClient || null,
                    });
                    // Also store reverse lookup by stream_url in case completed payload uses different id? not needed
                } catch (_) {}
            }
            return result;
        } catch (err) {
            const msg = typeof err === 'string' ? err : (err && err.message ? err.message : String(err));
            console.groupCollapsed(`%c[download_audio invoke failed] ${resolved.title || resolved.stream_url}`, 'color:#ff4d4f');
            console.error('DIAGNOSTIC download_invoke_failed', { resolved, format, error: msg, stack: err && err.stack });
            console.error(err);
            console.groupEnd();
            this.showToast(`Download start failed: ${msg}`, 'error', 7000);
            throw err;
        }
    },

    async handleDownloadFormSubmit(e, form) {
        if (e) {
            e.preventDefault();
            e.stopPropagation();
        }
        form = form || document.getElementById('download-form');
        if (!form) return;
        const urlInput = form.querySelector('input[name="url"]');
        if (!urlInput || !urlInput.value) return;

        let url = urlInput.value.trim();
        if (!/^https?:\/\//i.test(url)) {
            if (url.includes('youtube.com') || url.includes('youtu.be')) {
                url = 'https://' + url;
                urlInput.value = url;
            } else if (/^[a-zA-Z0-9_-]{11}$/.test(url)) {
                url = `https://www.youtube.com/watch?v=${url}`;
                urlInput.value = url;
            }
        }
        if (!url.startsWith('https://')) {
            this.showToast('Only secure HTTPS URLs are supported', 'error');
            return;
        }
        if (!window.AuralisYouTube) {
            this.showToast('YouTube resolver unavailable', 'error');
            return;
        }

        const opts = this.getDownloadOptions(form);

        // Check if URL is a playlist
        if (window.AuralisYouTube.isPlaylistUrl(url)) {
            this.showToast('Fetching playlist preview…', 'info');
            try {
                const pl = await window.AuralisYouTube.getPlaylist(url, opts);
                if (pl && pl.items && pl.items.length > 0) {
                    this.renderPlaylistPreview(pl, form);
                    this.showToast(`Found ${pl.items.length} track(s) in playlist`, 'success');
                    return;
                }
            } catch (plErr) {
                console.warn('getPlaylist failed, falling back to direct resolve:', plErr);
            }
        }

        this.showToast('Resolving source…', 'info');
        try {
            this._ensureDownloadRetryListener();
            const resolved = await window.AuralisYouTube.resolve(url, opts);
            if (resolved.kind === 'playlist') {
                await this.startPlaylistDownloads(resolved.items, 'm4a', opts, urlInput);
                return;
            }
            const result = await this.downloadResolvedTrack(resolved, 'm4a', opts, url);
            if (result) {
                this.showToast('Download started!', 'success');
                urlInput.value = '';
            }
        } catch (err) {
            const msg = err && err.message ? err.message : String(err);
            console.groupCollapsed(`%c[YouTube Resolve Failed] ${url}`, 'color:#ff7a45');
            console.error('DIAGNOSTIC resolve_failed', { url, opts, error: msg, stack: err && err.stack });
            console.error(err);
            console.groupEnd();
            // Resolver failures carry the per-client reaction report — archive it
            // so "Copy report" still works when nothing was ever downloaded. The
            // mint report rides along: a resolve that died before any client
            // answered is exactly the run where "did the mint even start?" is
            // the question, and there is no other place it would show up.
            try {
                if (err && (err.client_report || err.mint_report)) {
                    this._recordClientReport({
                        title: '', videoId: '', client: null,
                        client_report: err.client_report,
                        client_report_text: err.client_report_text || '',
                        mint_report: err.mint_report || null,
                        mint_report_text: err.mint_report_text || '',
                        mint_state: err.mint_state || null,
                    }, { phase: 'resolve_failed', url });
                }
            } catch (_) {}
            try { window.__auralisDownloadDiagnostics = window.__auralisDownloadDiagnostics || []; window.__auralisDownloadDiagnostics.push({ at: new Date().toISOString(), kind: 'resolve_failed', url, error: msg, clientReport: err?.client_report_text || null }); } catch (_) {}
            this.showToast(`Resolve failed: ${msg}`, 'error', 6000);
        }
    },

    async handleSearchFormSubmit(e, searchForm) {
        if (e) {
            e.preventDefault();
            e.stopPropagation();
        }
        searchForm = searchForm || document.getElementById('youtube-search-form');
        if (!searchForm) return;
        const q = searchForm.querySelector('input[name="q"]');
        if (!q || !q.value.trim()) return;
        const form = document.getElementById('download-form');
        await this.performYouTubeSearch(q.value.trim(), this.getDownloadOptions(form || searchForm));
    },

    _bindSearchBridgeMethods() {
        if (typeof window !== 'undefined') {
            window.Auralis = window.Auralis || {};
            const target = window.Auralis.bridge || this;
            if (target) {
                target.streamYouTubeSearchResult = this.streamYouTubeSearchResult.bind(this);
                target.downloadSearchResult = this.downloadSearchResult.bind(this);
            }
            window.Auralis.streamYouTubeSearchResult = this.streamYouTubeSearchResult.bind(this);
            window.Auralis.downloadSearchResult = this.downloadSearchResult.bind(this);
        }
    },

    async loadDownloadView() {
        this._ensureDownloadSubmitListeners();
        this._bindSearchBridgeMethods();
        const form = document.getElementById('download-form');
        if (!form) return;

        await this.ensureSettings();
    },

    async performYouTubeSearch(query, opts) {
        if (!window.AuralisYouTube) {
            this.showToast('YouTube resolver unavailable', 'error');
            return;
        }
        const resultsEl = document.getElementById('youtube-search-results');
        const spinnerEl = document.getElementById('youtube-search-spinner');
        const searchBtn = document.getElementById('youtube-search-btn');
        if (!resultsEl) return;

        this.showToast('Searching YouTube…', 'info');
        if (spinnerEl) spinnerEl.style.display = 'block';
        resultsEl.style.display = 'block';
        resultsEl.innerHTML = `
            <div class="track-row neu-glass" style="display: flex; align-items: center; justify-content: center; padding: var(--space-6); border-radius: var(--radius-md);">
                <i data-lucide="loader-2" class="spin" style="width: 24px; height: 24px; color: var(--accent);"></i>
                <span style="margin-left: var(--space-2); color: var(--text-3); font-size: var(--text-sm);">Searching YouTube for “${this.escapeHtml(query)}”…</span>
            </div>
        `;
        if (window.lucide) window.lucide.createIcons();
        if (searchBtn) searchBtn.disabled = true;

        try {
            const results = await window.AuralisYouTube.search(query, opts);
            if (spinnerEl) spinnerEl.style.display = 'none';

            if (!results || results.length === 0) {
                resultsEl.innerHTML = `
                    <div class="empty-state" style="padding: var(--space-4); text-align: center;">
                        <p style="color: var(--text-3); font-size: var(--text-sm);">No results found for “${this.escapeHtml(query)}”.</p>
                    </div>`;
                return;
            }
            this._lastSearchResults = results;
            try {
                window.__auralisLastSearchResults = results;
                if (window.Auralis?.bridge) window.Auralis.bridge._lastSearchResults = results;
            } catch (_) {}
            this._bindSearchBridgeMethods();

            resultsEl.innerHTML = results.map((r, i) => {
                const durText = r.duration_text || (r.duration ? this.formatTime(r.duration) : '');
                const durationPill = durText ? `
                    <div class="track-row-duration" style="margin-right: var(--space-2); flex-shrink: 0;">
                        <span class="neu-inset" style="padding: 2px 8px; font-size: var(--text-xs); color: var(--text-3); font-variant-numeric: tabular-nums;">${this.escapeHtml(durText)}</span>
                    </div>
                ` : '';

                const thumbContent = r.thumbnail
                    ? `<img src="${this.escapeHtml(r.thumbnail)}" alt="${this.escapeHtml(r.title)}" style="width: 100%; height: 100%; object-fit: cover;" onerror="this.onerror=null;this.parentElement.innerHTML='<i data-lucide=\\'music\\'></i>';if(window.lucide)window.lucide.createIcons();">`
                    : `<i data-lucide="music"></i>`;

                return `
                    <div class="track-row neu-glass" data-search-index="${i}" data-video-id="${this.escapeHtml(r.id)}" style="cursor: pointer; display: flex; align-items: center; gap: var(--space-3); padding: var(--space-2) var(--space-3); border-radius: var(--radius-md); margin-bottom: var(--space-2); touch-action: manipulation;">
                        <div class="track-row-artwork" style="width: 44px; height: 44px; border-radius: var(--radius-sm); overflow: hidden; flex-shrink: 0; background: var(--glass-weak); display: flex; align-items: center; justify-content: center;">
                            ${thumbContent}
                        </div>
                        <div class="track-row-info" style="flex: 1; min-width: 0;" data-action="stream-search-result" data-index="${i}" onclick="window.Auralis.bridge.streamYouTubeSearchResult(${i})">
                            <div class="track-row-title" style="font-weight: var(--font-medium); font-size: var(--text-sm); color: var(--text-1); overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">${this.escapeHtml(r.title)}</div>
                            <div class="track-row-subtitle" style="font-size: var(--text-xs); color: var(--text-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">${this.escapeHtml(r.channel || 'YouTube')}</div>
                        </div>
                        ${durationPill}
                        <div class="track-row-actions" style="display: flex; align-items: center; gap: var(--space-2); opacity: 1; flex-shrink: 0;">
                            <button type="button" class="btn btn-ghost btn-icon play-yt-btn" title="Stream Now" data-action="stream-search-result" data-index="${i}" onclick="event.stopPropagation(); window.Auralis.bridge.streamYouTubeSearchResult(${i})" style="touch-action: manipulation;">
                                <i data-lucide="play"></i>
                            </button>
                            <button type="button" class="btn btn-primary btn-sm neu download-yt-btn" title="Download Audio" data-action="download-search-result" data-index="${i}" data-video-id="${this.escapeHtml(r.id)}" data-video-url="${this.escapeHtml(r.url)}" data-title="${this.escapeHtml(r.title)}" onclick="event.stopPropagation(); window.Auralis.bridge.downloadSearchResult(${i})" style="touch-action: manipulation;">
                                <i data-lucide="download"></i> Download
                            </button>
                        </div>
                    </div>
                `;
            }).join('');
            if (window.lucide) window.lucide.createIcons();

            if (!resultsEl.dataset.searchActionsBound) {
                resultsEl.dataset.searchActionsBound = 'true';
                const handleAction = (e) => {
                    const dlBtn = e.target.closest && e.target.closest('[data-action="download-search-result"], .download-yt-btn');
                    if (dlBtn) {
                        e.preventDefault();
                        e.stopPropagation();
                        const idx = parseInt(dlBtn.dataset.index, 10);
                        const fallbackItem = {
                            id: dlBtn.dataset.videoId,
                            title: dlBtn.dataset.title,
                            url: dlBtn.dataset.videoUrl || (dlBtn.dataset.videoId ? `https://www.youtube.com/watch?v=${dlBtn.dataset.videoId}` : null),
                        };
                        this.downloadSearchResult(idx, fallbackItem);
                        return;
                    }
                    const streamBtn = e.target.closest && e.target.closest('[data-action="stream-search-result"], .play-yt-btn');
                    if (streamBtn) {
                        e.preventDefault();
                        e.stopPropagation();
                        const idx = parseInt(streamBtn.dataset.index, 10);
                        this.streamYouTubeSearchResult(idx);
                        return;
                    }
                };
                resultsEl.addEventListener('click', handleAction);
                resultsEl.addEventListener('touchend', handleAction, { passive: false });
            }
        } catch (err) {
            if (spinnerEl) spinnerEl.style.display = 'none';
            const msg2 = err && err.message ? err.message : String(err);
            console.error('DIAGNOSTIC search_failed', { query, error: msg2, stack: err && err.stack });
            console.error(err);
            this.showToast(`Search failed: ${msg2}`, 'error', 6000);
            resultsEl.innerHTML = `
                <div class="empty-state" style="padding: var(--space-4); text-align: center;">
                    <p style="color: var(--danger, #ff4d4f); font-size: var(--text-sm);">Search failed: ${this.escapeHtml(msg2)}</p>
                </div>`;
        } finally {
            if (searchBtn) searchBtn.disabled = false;
        }
    },

    async streamYouTubeSearchResult(index) {
        const results = this._lastSearchResults || [];
        const item = results[index];
        if (!item) return;

        // If this same stream item is already loaded, toggle play/pause
        if (window._auralisStreamAudio && window.Auralis?.player?.currentTrack?.title === item.title) {
            if (window._auralisStreamAudio.paused) {
                try {
                    await window._auralisStreamAudio.play();
                } catch (e) {
                    console.warn('Stream play toggle failed:', e);
                }
            } else {
                window._auralisStreamAudio.pause();
            }
            return;
        }

        const rows = document.querySelectorAll('#youtube-search-results .track-row');
        const row = rows[index];
        const playBtn = row ? row.querySelector('.play-yt-btn') : null;
        if (playBtn) {
            playBtn.innerHTML = '<i data-lucide="loader-2" class="spin"></i>';
            if (window.lucide) window.lucide.createIcons();
        }

        this.showToast(`Connecting stream for “${this.escapeHtml(item.title)}”…`, 'info');

        try {
            // Pause any backend Rust playback
            if (window.Auralis && window.Auralis.player) {
                try { window.Auralis.player.pause(); } catch (_) {}
            }
            await this.invoke('pause').catch(() => {});

            // Stop any existing streaming audio element
            if (window._auralisStreamAudio) {
                try {
                    window._auralisStreamAudio.pause();
                    window._auralisStreamAudio.removeAttribute('src');
                    window._auralisStreamAudio.load();
                } catch (_) {}
                window._auralisStreamAudio = null;
            }

            const form = document.getElementById('download-form');
            const opts = this.getDownloadOptions(form);
            const resolved = await window.AuralisYouTube.resolve(item.url, opts);
            if (!resolved || resolved.kind !== 'track' || !resolved.stream_url) {
                throw new Error('Could not resolve playable stream URL');
            }

            const trackObj = {
                title: item.title || resolved.title || 'YouTube Audio',
                artist: item.channel || resolved.author || 'YouTube',
                duration_secs: item.duration || 0,
                album_art_path: item.thumbnail || resolved.thumbnail || null,
            };

            // Set current track and player bar reactive state
            if (window.Auralis && window.Auralis.player) {
                window.Auralis.player.currentTrack = trackObj;
                window.Auralis.player.duration = item.duration || 0;
                window.Auralis.player.progress = 0;
                window.Auralis.player.isPlaying = true;
                window.Auralis.player.updatePlayButton();
                window.Auralis.player.updateProgressUI();
                if (typeof window.Auralis.player.updateFullScreenMetadata === 'function') {
                    window.Auralis.player.updateFullScreenMetadata();
                }
                if (typeof window.Auralis.player.updateMediaSessionMetadata === 'function') {
                    window.Auralis.player.updateMediaSessionMetadata(trackObj);
                }
            }

            if (typeof this.updatePlayerBar === 'function') {
                this.updatePlayerBar(trackObj);
            } else if (window.Auralis?.bridge?.updatePlayerBar) {
                window.Auralis.bridge.updatePlayerBar(trackObj);
            }

            const audio = new Audio();
            audio.crossOrigin = 'anonymous';
            audio.src = resolved.stream_url;
            if (window.Auralis?.player && typeof window.Auralis.player.volume === 'number') {
                audio.volume = window.Auralis.player.volume;
            }
            window._auralisStreamAudio = audio;

            audio.addEventListener('loadedmetadata', () => {
                if (audio.duration && isFinite(audio.duration) && audio.duration > 0) {
                    trackObj.duration_secs = Math.round(audio.duration);
                    if (window.Auralis && window.Auralis.player) {
                        window.Auralis.player.duration = audio.duration;
                        window.Auralis.player.updateProgressUI();
                    }
                }
            });

            audio.addEventListener('timeupdate', () => {
                if (window.Auralis && window.Auralis.player && !window.Auralis.player.isSeeking) {
                    window.Auralis.player.progress = audio.currentTime;
                    if (audio.duration && isFinite(audio.duration) && audio.duration > 0) {
                        window.Auralis.player.duration = audio.duration;
                    }
                    window.Auralis.player.updateProgressUI();
                    window.Auralis.player.updatePositionState();
                }
            });

            audio.addEventListener('play', () => {
                if (window.Auralis && window.Auralis.player) {
                    window.Auralis.player.isPlaying = true;
                    window.Auralis.player.updatePlayButton();
                }
                if (playBtn) {
                    playBtn.innerHTML = '<i data-lucide="pause"></i>';
                    if (window.lucide) window.lucide.createIcons();
                }
            });

            audio.addEventListener('pause', () => {
                if (window.Auralis && window.Auralis.player) {
                    window.Auralis.player.isPlaying = false;
                    window.Auralis.player.updatePlayButton();
                }
                if (playBtn) {
                    playBtn.innerHTML = '<i data-lucide="play"></i>';
                    if (window.lucide) window.lucide.createIcons();
                }
            });

            audio.addEventListener('ended', () => {
                if (window.Auralis && window.Auralis.player) {
                    window.Auralis.player.isPlaying = false;
                    window.Auralis.player.progress = 0;
                    window.Auralis.player.updatePlayButton();
                    window.Auralis.player.updateProgressUI();
                }
                if (playBtn) {
                    playBtn.innerHTML = '<i data-lucide="play"></i>';
                    if (window.lucide) window.lucide.createIcons();
                }
            });

            audio.addEventListener('error', (e) => {
                console.error('Direct audio stream error:', e);
                this.showToast('Direct stream playback error', 'error', 6000);
                if (window.Auralis && window.Auralis.player) {
                    window.Auralis.player.isPlaying = false;
                    window.Auralis.player.updatePlayButton();
                }
                if (playBtn) {
                    playBtn.innerHTML = '<i data-lucide="play"></i>';
                    if (window.lucide) window.lucide.createIcons();
                }
            });

            await audio.play();
            this.showToast(`Streaming “${this.escapeHtml(trackObj.title)}”`, 'success');
        } catch (err) {
            const m = err && err.message ? err.message : String(err);
            console.error('DIAGNOSTIC streamYouTubeSearchResult failed', { item, error: m, stack: err && err.stack });
            this.showToast(`Stream failed: ${m}`, 'error', 6000);
            if (window.Auralis && window.Auralis.player) {
                window.Auralis.player.isPlaying = false;
                window.Auralis.player.updatePlayButton();
            }
            if (playBtn) {
                playBtn.innerHTML = '<i data-lucide="play"></i>';
                if (window.lucide) window.lucide.createIcons();
            }
        }
    },

    async downloadSearchResult(index, fallbackItem = null) {
        const results = this._lastSearchResults || window.__auralisLastSearchResults || window.Auralis?.bridge?._lastSearchResults || [];
        let item = (typeof index === 'number' && !isNaN(index) && results[index]) ? results[index] : fallbackItem;

        const rows = document.querySelectorAll('#youtube-search-results .track-row');
        const row = (typeof index === 'number' && !isNaN(index)) ? rows[index] : (item?.id ? document.querySelector(`#youtube-search-results [data-video-id="${item.id}"]`) : null);
        const dlBtn = row ? row.querySelector('.download-yt-btn') : null;

        if (!item && dlBtn) {
            item = {
                id: dlBtn.dataset.videoId,
                title: dlBtn.dataset.title,
                url: dlBtn.dataset.videoUrl || (dlBtn.dataset.videoId ? `https://www.youtube.com/watch?v=${dlBtn.dataset.videoId}` : null),
            };
        }

        if (dlBtn && dlBtn.disabled) return;
        if (dlBtn) {
            dlBtn.disabled = true;
            dlBtn.innerHTML = '<i data-lucide="loader-2" class="spin"></i> Starting…';
            if (window.lucide) window.lucide.createIcons();
        }

        if (!item || (!item.url && !item.id)) {
            if (dlBtn) dlBtn.disabled = false;
            return;
        }

        const urlToResolve = item.url || (item.id ? `https://www.youtube.com/watch?v=${item.id}` : null);
        if (!urlToResolve) {
            if (dlBtn) dlBtn.disabled = false;
            return;
        }

        const form = document.getElementById('download-form');
        this.showToast(`Resolving “${item.title || 'audio'}”…`, 'info');
        try {
            if (!window.AuralisYouTube) throw new Error('YouTube resolver unavailable');
            await this.ensureSettings();
            const opts = this.getDownloadOptions(form);
            this._ensureDownloadRetryListener();
            const resolved = await window.AuralisYouTube.resolve(urlToResolve, opts);
            if (!resolved || resolved.kind !== 'track') throw new Error('Not a playable track');
            const format = (opts && opts.container && opts.container !== 'auto') ? opts.container : (resolved.ext || 'm4a');
            const result = await this.downloadResolvedTrack(resolved, format, opts, urlToResolve);
            if (result) {
                this.showToast(`Download queued: “${resolved.title || item.title}”`, 'success');
                if (dlBtn) {
                    dlBtn.innerHTML = '<i data-lucide="check"></i> Added';
                    dlBtn.classList.remove('btn-primary');
                    dlBtn.classList.add('btn-secondary');
                    dlBtn.disabled = true;
                    if (window.lucide) window.lucide.createIcons();
                }
            } else {
                if (dlBtn) {
                    dlBtn.disabled = false;
                    dlBtn.innerHTML = '<i data-lucide="download"></i> Download';
                    if (window.lucide) window.lucide.createIcons();
                }
            }
        } catch (err) {
            const m = err && err.message ? err.message : String(err);
            console.error('DIAGNOSTIC downloadSearchResult failed', { item, error: m, stack: err && err.stack });
            this.showToast(`Download failed: ${m}`, 'error', 6000);
            if (dlBtn) {
                dlBtn.disabled = false;
                dlBtn.innerHTML = '<i data-lucide="download"></i> Download';
                if (window.lucide) window.lucide.createIcons();
            }
        }
    },

    renderPlaylistPreview(playlistData, form) {
        const container = document.getElementById('youtube-playlist-preview');
        if (!container) return;

        this._currentPlaylistData = playlistData;
        const items = playlistData.items || [];

        container.style.display = 'block';
        container.innerHTML = `
            <div class="playlist-preview-header">
                <div>
                    <div class="playlist-preview-title">${this.escapeHtml(playlistData.title)}</div>
                    <div class="playlist-preview-stats">${playlistData.author ? `${this.escapeHtml(playlistData.author)} · ` : ''}${items.length} track(s) found</div>
                </div>
                <div style="display: flex; align-items: center; gap: var(--space-3);">
                    <label class="checkbox" style="font-size: var(--text-xs); cursor: pointer; display: inline-flex; align-items: center; gap: var(--space-1);">
                        <input type="checkbox" id="playlist-select-all" checked>
                        Select All
                    </label>
                </div>
            </div>
            <div class="playlist-preview-list" id="playlist-preview-items">
                ${items.map((item, idx) => `
                    <div class="playlist-item-row" data-index="${idx}">
                        <input type="checkbox" class="playlist-item-checkbox" data-index="${idx}" checked style="cursor: pointer;">
                        <div class="playlist-item-info">
                            <div class="playlist-item-title">${this.escapeHtml(item.title)}</div>
                            <div class="playlist-item-author">${item.channel ? this.escapeHtml(item.channel) : 'YouTube'}${item.duration ? ' · ' + this.formatTime(item.duration) : ''}</div>
                        </div>
                    </div>
                `).join('')}
            </div>
            <div style="display: flex; justify-content: space-between; align-items: center; margin-top: var(--space-3); padding-top: var(--space-3); border-top: 1px solid var(--glass-border);">
                <button type="button" class="btn btn-ghost btn-sm" id="btn-cancel-playlist-preview">
                    Cancel
                </button>
                <button type="button" class="btn btn-primary btn-sm neu" id="btn-download-selected">
                    <i data-lucide="download"></i>
                    <span id="btn-download-selected-label">Download All (${items.length})</span>
                </button>
            </div>
        `;

        if (window.lucide) window.lucide.createIcons();

        const selectAllCb = container.querySelector('#playlist-select-all');
        const itemCbs = container.querySelectorAll('.playlist-item-checkbox');
        const dlBtn = container.querySelector('#btn-download-selected');
        const dlBtnLabel = container.querySelector('#btn-download-selected-label');
        const cancelBtn = container.querySelector('#btn-cancel-playlist-preview');

        const updateSelectedCount = () => {
            const checkedCount = container.querySelectorAll('.playlist-item-checkbox:checked').length;
            if (dlBtnLabel) {
                dlBtnLabel.textContent = checkedCount === items.length
                    ? `Download All (${items.length})`
                    : `Download Selected (${checkedCount})`;
            }
            if (dlBtn) dlBtn.disabled = checkedCount === 0;
            if (selectAllCb) {
                selectAllCb.checked = checkedCount === items.length;
                selectAllCb.indeterminate = checkedCount > 0 && checkedCount < items.length;
            }
        };

        if (selectAllCb) {
            selectAllCb.addEventListener('change', () => {
                itemCbs.forEach(cb => { cb.checked = selectAllCb.checked; });
                updateSelectedCount();
            });
        }

        itemCbs.forEach(cb => {
            cb.addEventListener('change', updateSelectedCount);
        });

        if (cancelBtn) {
            cancelBtn.addEventListener('click', () => {
                container.style.display = 'none';
                container.innerHTML = '';
                this._currentPlaylistData = null;
            });
        }

        if (dlBtn) {
            dlBtn.addEventListener('click', async () => {
                const selected = [];
                container.querySelectorAll('.playlist-item-checkbox:checked').forEach(cb => {
                    const idx = parseInt(cb.dataset.index, 10);
                    if (items[idx]) selected.push(items[idx]);
                });
                if (selected.length === 0) {
                    this.showToast('No tracks selected for download', 'warning');
                    return;
                }
                container.style.display = 'none';
                container.innerHTML = '';
                const urlInput = form ? form.querySelector('input[name="url"]') : null;
                if (urlInput) urlInput.value = '';
                await this.downloadPlaylist(selected);
            });
        }
    },

    async downloadPlaylist(items) {
        if (!items || items.length === 0) {
            this.showToast('No tracks selected to download', 'warning');
            return;
        }
        const form = document.getElementById('download-form');
        const opts = this.getDownloadOptions(form);
        const format = 'm4a';
        this.showToast(`Starting batch download for ${items.length} track(s)...`, 'info');

        this._ensureDownloadRetryListener();
        let started = 0;
        for (const item of items) {
            try {
                const t = await window.AuralisYouTube.resolve(item.url, opts);
                if (t.kind !== 'track') continue;
                const result = await this.downloadResolvedTrack(t, format, opts, item.url);
                if (result) started++;
            } catch (err) {
                console.error(`Failed to start download for ${item.title}:`, err);
            }
        }

        if (started > 0) {
            this.showToast(`Started ${started} download(s) from playlist!`, 'success');
        } else {
            this.showToast('Could not start playlist downloads', 'error');
        }
    },

    async startPlaylistDownloads(items, format, opts, urlInput) {
        return this.downloadPlaylist(items);
    },

    updateDownloadProgressUI(progress) {
        const list = document.getElementById('downloads-list');
        if (!list) return;

        if (!list.dataset.copyBound) {
            list.dataset.copyBound = 'true';
            list.addEventListener('click', (e) => {
                const reportBtn = e.target.closest && e.target.closest('[data-action="copy-client-report"]');
                if (reportBtn) {
                    const text = this._buildClientReportText();
                    copyWithToast({ text, label: 'client report', showToast: (m, k) => this.showToast(m, k) });
                    return;
                }
                const btn = e.target.closest && e.target.closest('[data-action="copy-download-error"]');
                if (!btn) return;
                const errText = btn.dataset.error || '';
                copyWithToast({ text: errText, label: 'error', showToast: (m, k) => this.showToast(m, k) });
            });
        }

        let row = list.querySelector(`[data-download-id="${progress.id}"]`);
        if (!row) {
            row = document.createElement('div');
            row.className = 'track-row neu-glass';
            row.dataset.downloadId = progress.id;
            list.prepend(row);
        }

        const pct = Math.round((progress.progress || 0) * 100);
        const errRaw = typeof this.extractErrorMessage === 'function'
            ? this.extractErrorMessage(progress, '')
            : (progress.error || progress.error_message || '');
        const isFailed = progress.status === 'failed';
        const isCompleted = progress.status === 'completed';
        if (isFailed) {
            row.classList.add('download-failed');
            row.style.borderLeft = '3px solid #ff4d4f';
            // Log again for logcat visibility (progress event mirrors diagnostic)
            console.error(`[Downloads UI] DIAGNOSTIC failed row id=${progress.id} title=${progress.title} error=${errRaw} url=${progress.url}`);
        } else {
            row.classList.remove('download-failed');
            row.style.borderLeft = '';
        }
        const host = (() => { try { return new URL(progress.url || '').host || ''; } catch (_) { return ''; } })();
        // Where the file actually landed. The row never showed this, which is
        // why a public copy that silently failed to publish looked identical to
        // one that worked: the user saw "completed" and then could not find the
        // file anywhere. The path is already on the event (`output_path` is
        // overwritten with the public location when MediaStore publishing
        // succeeds, and left as the app-private path when it does not), so this
        // only has to classify it.
        const outPath = (progress && progress.output_path) ? String(progress.output_path) : '';
        const isPublicCopy = outPath.startsWith('/storage/emulated/') || outPath.startsWith('content://');
        const destDir = (() => {
            if (!outPath) return '';
            if (isPublicCopy) {
                const i = outPath.lastIndexOf('/');
                return i > 0 ? outPath.slice(0, i) : outPath;
            }
            return '';
        })();
        const subtitle = isFailed
            ? `<span style="color:#ff4d4f;font-weight:600">failed • ${host ? host + ' • ' : ''}${pct}%</span>`
            : `${this.escapeHtml(progress.status)}${host ? ' • ' + this.escapeHtml(host) : ''} • ${pct}%`;
        // Shown only once a download has actually completed, and only when the
        // file is NOT somewhere the file manager can see. A completed download
        // the user cannot find is a failure that no other surface reports.
        // `publish_error` carries the reason the public copy did not land, which
        // is otherwise only ever a `warn!` into a logcat that release builds do
        // not emit — so without this the app can say "it failed" but never why.
        const publishError = (progress && progress.publish_error) ? String(progress.publish_error) : '';
        const destNote = (!isFailed && progress.status === 'completed')
            ? (isPublicCopy
                ? `<div style="margin-top:4px;font-size:11px;color:var(--text-3);font-family:monospace;word-break:break-all;user-select:text">saved to ${this.escapeHtml(destDir)}</div>`
                : `<div style="margin-top:4px;font-size:11px;color:#e8a33d;font-family:monospace;user-select:text">saved in app storage only — not visible in Files${outPath ? ' (' + this.escapeHtml(outPath) + ')' : ''}${publishError && !/^not android/i.test(publishError) ? '<br>publish failed: ' + this.escapeHtml(publishError) : ''}</div>`)
            : '';
        const errBlock = isFailed && errRaw
            ? `<div style="margin-top:6px;padding:8px 10px;background:rgba(255,77,79,0.08);border:1px solid rgba(255,77,79,0.25);border-radius:8px;font-family:monospace;font-size:11px;line-height:1.4;white-space:pre-wrap;word-break:break-all;user-select:text;max-height:120px;overflow:auto;color:var(--text-2)">${this.escapeHtml(errRaw)}</div>
               <div style="margin-top:6px;display:flex;gap:8px;align-items:center;flex-wrap:wrap">
                 <button type="button" class="btn btn-secondary btn-sm" data-action="copy-download-error" data-error="${this.escapeHtml(errRaw)}" title="Copy full error (for bug report)">Copy error</button>
                  <span style="font-size:11px;color:var(--text-3)">${errRaw.includes('403') ? '403 from the googlevideo host — the url was refused before any byte arrived. Auto-retrying: first the same client&#39;s muxed format, then another client. Minting a PO token usually will not help here — a Web token is platform-bound and invalid on the clients that do hand out audio urls.' : errRaw.includes('Truncated download') ? 'Transfer finished but the file holds only part of the audio (SABR-style partial stream) — re-resolving via another client.' : errRaw.includes('timeout') || errRaw.includes('stalled') ? 'Network timeout — retry on stable connection.' : errRaw.includes('404') ? 'URL expired — resolve again.' : 'Use Copy error and Copy report (per-client InnerTube reactions) in a bug report.'}</span>
               </div>`
            : '';
        const pctBar = isFailed
            ? `<div class="progress-track neu-inset" style="width:120px;height:6px;opacity:0.5"><div class="progress-fill" style="width:${pct}%;background:#ff4d4f;height:100%"></div></div>`
            : `<div class="progress-track neu-inset" style="width: 120px; height: 6px;"><div class="progress-fill" style="width: ${pct}%; background: var(--accent); height: 100%;"></div></div>`;
        // Per-client reaction line: which Innertube client won, what the others
        // answered (SABR-only / 403 / no adaptive urls). Release builds log
        // nothing, so this row + "Copy report" is the only device-visible proof.
        const reportSrc = (() => {
            try {
                const m = this._ensurePendingMap();
                const ctx = m.get(progress.id);
                if (ctx && ctx.resolved) return ctx.resolved;
            } catch (_) {}
            const list = this._ensureClientReports();
            return list.length ? list[list.length - 1] : null;
        })();
        const reportText = (reportSrc && (reportSrc.client_report_text || reportSrc.text)) || '';
        const selInfo = (reportSrc && reportSrc.selection) || null;
        // The PO-token mint line. Reads from either shape: a live `resolved`
        // (from the pending-download context) carries `mint_report_text`, while
        // an archived entry carries `mintText` — same archive, two producers.
        //
        // The state name is printed first and on its own because the four
        // possible values call for four different responses, and "no token on
        // the url" reads identically for the two most important ones. In
        // particular `minted-stripped` is CORRECT behaviour, not a failure: a
        // Web/BotGuard token is platform-bound and is deliberately withheld
        // from `ios`/`android`/`android_vr`. Reading it as "minting is broken"
        // sends the owner to fix the WebView when the WebView is fine.
        const mintText = (reportSrc && (reportSrc.mint_report_text || reportSrc.mintText)) || '';
        const mintLine = (() => {
            if (!mintText && !reportSrc) return '';
            const state = (reportSrc && (reportSrc.mint_state || reportSrc.mintState)) || 'unknown';
            const oneLine = mintText.split('\n').map((l) => l.trim()).filter(Boolean)[0] || '';
            return `<div style="margin-top:4px;font-size:11px;line-height:1.45;color:var(--text-3);font-family:monospace;word-break:break-word;user-select:text">pot: ${this.escapeHtml(state)}${oneLine && !oneLine.includes('state=') ? ' — ' + this.escapeHtml(oneLine) : ''}<br><span style="opacity:.8">${this.escapeHtml(oneLine || 'no mint report')}</span></div>`;
        })();
        const clientBlock = (reportText || mintLine)
            ? `<div style="margin-top:6px;font-size:11px;line-height:1.45;color:var(--text-3);font-family:monospace;word-break:break-word;user-select:text">${reportText ? `clients: ${this.escapeHtml(reportText)}${selInfo ? `<br>picked: itag=${this.escapeHtml(String(selInfo.itag))} ${this.escapeHtml(String(selInfo.ext))}${selInfo.audioOnly ? ' audio-only' : ' MUXED video+audio'}${selInfo.legacyProgressive ? ' — legacy progressive (SABR)' : ''}` : ''}` : ''}
                   <button type="button" class="btn btn-secondary btn-sm" data-action="copy-client-report" title="Copy how every InnerTube client answered (SABR / 403 / format counts) AND the PO-token mint steps">Copy report</button></div>`
            : '';
        row.innerHTML = `
            <div class="track-row-info" style="min-width:0;flex:1">
                <div class="track-row-title" style="white-space:nowrap;overflow:hidden;text-overflow:ellipsis">${this.escapeHtml(progress.title || progress.url || 'Downloading...')}</div>
                <div class="track-row-subtitle">${subtitle}</div>${destNote}
                ${errBlock}
                ${clientBlock}
                ${mintLine}
            </div>
            ${pctBar}
        `;
    },

    async loadSyncView() {
        try {
            const pairingInfo = await this.invoke('start_pairing');
            if (pairingInfo) {
                const deviceIdEl = document.getElementById('sync-device-id');
                if (deviceIdEl) {
                    deviceIdEl.textContent = `Pairing PIN: ${pairingInfo.pin}`;
                }
                const qrContainer = document.getElementById('sync-qr-container');
                if (qrContainer && pairingInfo.qr_image) {
                    qrContainer.innerHTML = `<img src="data:image/png;base64,${pairingInfo.qr_image}" alt="Pairing QR" style="width: 100%; height: 100%; object-fit: contain; border-radius: var(--radius-sm);">`;
                }
            }
        } catch (e) {
            console.warn('Pairing info query failed:', e);
        }

        try {
            const devices = await this.invoke('get_paired_devices');
            const list = document.getElementById('synced-devices-list');
            if (list) {
                if (devices && devices.length > 0) {
                    list.innerHTML = devices.map(d => `
                        <div class="track-row neu-glass" style="margin-bottom: var(--space-2); border-radius: var(--radius-md); display: flex; align-items: center; justify-content: space-between; padding: var(--space-3);">
                            <div style="display: flex; align-items: center; gap: var(--space-3);">
                                <i data-lucide="${d.device_type === 'mobile' ? 'smartphone' : 'laptop'}" style="width: 24px; height: 24px; color: var(--accent);"></i>
                                <div>
                                    <div style="font-weight: var(--font-semibold); color: var(--text-1);">${this.escapeHtml(d.name)}</div>
                                    <div style="font-size: var(--text-xs); color: var(--text-3);">${d.ip_address || 'LAN Peer'} · Status: ${d.status || 'paired'}</div>
                                </div>
                            </div>
                            <div style="display: flex; gap: var(--space-2);">
                                <button class="btn btn-primary btn-sm neu" onclick="window.Auralis.bridge.syncWithDevice('${d.id}')">
                                    <i data-lucide="refresh-cw"></i>
                                    Sync
                                </button>
                            </div>
                        </div>
                    `).join('');
                    if (window.lucide) window.lucide.createIcons();
                } else {
                    list.innerHTML = `
                        <div class="empty-state glass neu" style="padding: var(--space-6); text-align: center; border-radius: var(--radius-md);">
                            <i data-lucide="wifi" style="width: 32px; height: 32px; color: var(--accent); margin-bottom: var(--space-2);"></i>
                            <h4 style="color: var(--text-1); font-size: var(--text-base); margin-bottom: var(--space-1);">No paired devices</h4>
                            <p style="color: var(--text-3); font-size: var(--text-xs);">Use the pairing PIN above on another device to start sharing your library.</p>
                        </div>
                    `;
                    if (window.lucide) window.lucide.createIcons();
                }
            }
        } catch (e) {
            console.error('Failed to load paired devices:', e);
        }
    },

    async syncWithDevice(deviceId) {
        if (!deviceId) return;
        this.showToast('Syncing with device...', 'info');
        try {
            await this.invoke('sync_with_device', { id: deviceId });
            this.showToast('Device synchronization complete!', 'success');
            this.loadSyncView();
        } catch (err) {
            this.showToast(`Sync failed: ${err}`, 'error');
        }
    },

    async connectDirectPeer() {
        const input = document.getElementById('direct-peer-address');
        if (!input || !input.value.trim()) {
            this.showToast('Please enter an IP:Port or Multiaddr', 'warning');
            return;
        }
        const address = input.value.trim();
        this.showToast(`Connecting to ${address}...`, 'info');
        try {
            const res = await this.invoke('connect_peer_address', { address });
            this.showToast(res || 'Direct connection established!', 'success');
            input.value = '';
            this.loadSyncView();
        } catch (err) {
            this.showToast(`Direct connection failed: ${err}`, 'error');
        }
    },

    async syncNow() {
        this.showToast('Initiating peer synchronization...', 'info');
        try {
            const devices = await this.invoke('get_paired_devices');
            if (devices && devices.length > 0) {
                for (const device of devices) {
                    await this.invoke('sync_with_device', { id: device.id });
                }
                this.showToast(`Synced with ${devices.length} device(s)!`, 'success');
            } else {
                await this.scanLibrary();
                this.showToast('Library scan & sync complete!', 'success');
            }
            this.loadSyncView();
        } catch (err) {
            this.showToast(`Sync failed: ${err}`, 'error');
        }
    }
};

try {
    if (typeof window !== 'undefined') {
        window.Auralis = window.Auralis || {};
        if (window.Auralis.bridge) {
            window.Auralis.bridge.streamYouTubeSearchResult = downloadMethods.streamYouTubeSearchResult.bind(window.Auralis.bridge);
            window.Auralis.bridge.downloadSearchResult = downloadMethods.downloadSearchResult.bind(window.Auralis.bridge);
        }
    }
} catch (_) {}
