#!/usr/bin/env node
/**
 * youtube_resolver.test.js — JS unit tests for ui/js/youtube.js
 *
 * Runs with Node's built-in runner (no npm deps):
 *   node --test scripts/tests/youtube_resolver.test.js
 *
 * Covers:
 *  - pure helpers (isDirectAudio, extFromMime/Url, pickThumb, basename, isPlaylistUrl, extractVideoId)
 *  - streaming-data audio-format detection (hasDirectOrDecipherableAudio)
 *  - regression guard: 6-client fallback list must be present (prevents revert to 3)
 *  - nativeFetch header/body extraction (Tauri http_fetch bridge)
 *
 * These tests catch the ARM stream bug (IOS-only LOGIN_REQUIRED) before an APK is built.
 */
import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { coreMethods } from '../../ui/js/modules/core.js';
import { downloadMethods } from '../../ui/js/modules/downloads.js';
import { applyPoTokenToUrl } from '../../ui/js/modules/pot_scope.js';

// ── helpers extracted verbatim from youtube.js so tests don't need a WebView ──
function isDirectAudio(url) { return /\.(mp3|m4a|aac|ogg|oga|opus|wav|flac|webm)(\?.*)?$/i.test(url); }
function extFromMime(mime) {
    if (!mime) return 'm4a';
    const m = String(mime).toLowerCase();
    if (m.includes('webm') || m.includes('opus')) return 'webm';
    if (m.includes('ogg')) return 'ogg';
    if (m.includes('wav')) return 'wav';
    if (m.includes('flac')) return 'flac';
    if (m.includes('mp4') || m.includes('m4a') || m.includes('aac')) {
        // video/mp4 progressive (itag 18 muxed) must stay mp4, not m4a
        if (m.startsWith('video/')) return 'mp4';
        return 'm4a';
    }
    if (m.includes('mpeg') || m.includes('mp3')) return 'mp3';
    return 'm4a';
}
function extFromUrl(url) { const m = url.split('?')[0].match(/\.([a-z0-9]+)$/i); return m ? m[1].toLowerCase() : 'mp3'; }
function pickThumb(thumb) {
    if (!thumb) return null;
    if (typeof thumb === 'string') return thumb;
    try {
        if (Array.isArray(thumb) && thumb.length) return thumb[thumb.length - 1]?.url || thumb[0]?.url || null;
        if (Array.isArray(thumb.contents) && thumb.contents.length) return thumb.contents[thumb.contents.length - 1]?.url || thumb.contents[0]?.url || null;
        if (thumb.url) return thumb.url;
    } catch (_) {}
    return null;
}
function basename(url) {
    try { const u = new URL(url); const last = u.pathname.split('/').filter(Boolean).pop() || 'audio_track'; return decodeURIComponent(last); } catch (_) { return 'audio_track'; }
}
function isPlaylistUrl(url) { return /[?&]list=([^&]+)/.test(url) && !/watch\?/.test(url); }
function extractVideoId(rawUrl) { const url = (rawUrl || '').trim(); const idMatch = url.match(/(?:v=|youtu\.be\/|shorts\/|embed\/|^)([a-zA-Z0-9_-]{11})/); return idMatch ? idMatch[1] : url; }
function isAudioFormat(f) {
    if (!f) return false;
    if (f.has_audio && !f.has_video) return true;
    if (typeof f.mime_type === 'string' && f.mime_type.startsWith('audio/')) return true;
    if (f.has_audio) return true;
    return false;
}
function hasDirectOrDecipherableAudio(r) {
    if (!r || !r.streaming_data) return false;
    const sd = r.streaming_data;
    const all = [...(sd.adaptive_formats || []), ...(sd.formats || [])];
    return all.some((f) => isAudioFormat(f) && Boolean(f.url || f.signature_cipher || f.cipher || typeof f.decipher === 'function'));
}
function hasLegacyProgressiveFallback(r) {
    if (!r || !r.streaming_data) return false;
    const fmts = r.streaming_data.formats || [];
    if (!fmts.length) return false;
    return fmts.some((f) => Boolean(f.url || f.signature_cipher || f.cipher || typeof f.decipher === 'function'));
}

// ── Tests ──
describe('youtube.js pure helpers', () => {
    it('isDirectAudio detects audio URLs', () => {
        assert.equal(isDirectAudio('https://cdn.example.com/a.mp3'), true);
        assert.equal(isDirectAudio('https://cdn.example.com/a.m4a?token=1'), true);
        assert.equal(isDirectAudio('https://cdn.example.com/a.webm'), true);
        assert.equal(isDirectAudio('https://cdn.example.com/a.flac'), true);
        assert.equal(isDirectAudio('https://www.youtube.com/watch?v=dQw4w9WgXcQ'), false);
        assert.equal(isDirectAudio('https://youtu.be/dQw4w9WgXcQ'), false);
    });

    it('extFromMime maps MIME to container', () => {
        assert.equal(extFromMime('audio/mp4; codecs="mp4a.40.2"'), 'm4a');
        assert.equal(extFromMime('audio/webm; codecs="opus"'), 'webm');
        assert.equal(extFromMime('audio/ogg'), 'ogg');
        assert.equal(extFromMime('audio/wav'), 'wav');
        assert.equal(extFromMime('audio/flac'), 'flac');
        assert.equal(extFromMime('audio/mpeg'), 'mp3');
        assert.equal(extFromMime('video/mp4'), 'mp4');
        assert.equal(extFromMime('video/mp4; codecs="avc1.42001E, mp4a.40.2"'), 'mp4');
        assert.equal(extFromMime('audio/mp4'), 'm4a');
        assert.equal(extFromMime(null), 'm4a');
        assert.equal(extFromMime(''), 'm4a');
    });

    it('extFromUrl extracts extension', () => {
        assert.equal(extFromUrl('https://cdn.example.com/file.MP3?x=1'), 'mp3');
        assert.equal(extFromUrl('https://cdn.example.com/file.webm'), 'webm');
        assert.equal(extFromUrl('https://cdn.example.com/noext'), 'mp3');
    });

    it('pickThumb prefers last thumbnail', () => {
        assert.equal(pickThumb(null), null);
        assert.equal(pickThumb('https://i.ytimg.com/hq.jpg'), 'https://i.ytimg.com/hq.jpg');
        assert.equal(pickThumb([{ url: 'a.jpg' }, { url: 'b.jpg' }]), 'b.jpg');
        assert.equal(pickThumb({ contents: [{ url: 'a.jpg' }, { url: 'c.jpg' }] }), 'c.jpg');
        assert.equal(pickThumb({ url: 'single.jpg' }), 'single.jpg');
        assert.equal(pickThumb([]), null);
    });

    it('basename extracts filename', () => {
        assert.equal(basename('https://cdn.example.com/music/hello%20world.mp3?x=1'), 'hello world.mp3');
        assert.equal(basename('not a url'), 'audio_track');
    });

    it('isPlaylistUrl detects only non-watch list URLs', () => {
        assert.equal(isPlaylistUrl('https://www.youtube.com/playlist?list=PL123'), true);
        assert.equal(isPlaylistUrl('https://www.youtube.com/watch?v=abc&list=PL123'), false);
        assert.equal(isPlaylistUrl('https://www.youtube.com/watch?v=abc'), false);
    });

    it('extractVideoId handles all YouTube URL forms', () => {
        assert.equal(extractVideoId('https://www.youtube.com/watch?v=dQw4w9WgXcQ'), 'dQw4w9WgXcQ');
        assert.equal(extractVideoId('https://youtu.be/dQw4w9WgXcQ'), 'dQw4w9WgXcQ');
        assert.equal(extractVideoId('https://www.youtube.com/shorts/dQw4w9WgXcQ'), 'dQw4w9WgXcQ');
        assert.equal(extractVideoId('https://www.youtube.com/embed/dQw4w9WgXcQ'), 'dQw4w9WgXcQ');
        assert.equal(extractVideoId('dQw4w9WgXcQ'), 'dQw4w9WgXcQ');
        assert.equal(extractVideoId('  https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=PL1  '), 'dQw4w9WgXcQ');
    });
});

describe('streaming_data audio detection', () => {
    it('hasDirectOrDecipherableAudio accepts url', () => {
        assert.equal(hasDirectOrDecipherableAudio({ streaming_data: { adaptive_formats: [{ has_audio: true, mime_type: 'audio/mp4', url: 'https://googlevideo.com/a' }], formats: [] } }), true);
    });
    it('accepts signature_cipher', () => {
        assert.equal(hasDirectOrDecipherableAudio({ streaming_data: { adaptive_formats: [{ has_audio: true, mime_type: 'audio/webm', signature_cipher: 's=...' }], formats: [] } }), true);
    });
    it('rejects video-only', () => {
        assert.equal(hasDirectOrDecipherableAudio({ streaming_data: { adaptive_formats: [{ has_audio: false, has_video: true, mime_type: 'video/mp4', url: 'x' }], formats: [] } }), false);
    });
    it('rejects missing streaming_data', () => {
        assert.equal(hasDirectOrDecipherableAudio(null), false);
        assert.equal(hasDirectOrDecipherableAudio({}), false);
    });
});

describe('SABR-only fallback (FreeTube#6977)', () => {
    it('rejects SABR-only adaptive_formats with no URL but accepts legacy progressive via fallback', () => {
        // 2026 WEB SABR-only: adaptive_formats have no url/cipher, only serverAbrStreamingUrl (not parsed)
        // Legacy 18 may have has_audio false due to mapping bug (video/mp4 + missing audioQuality) — isAudioFormat fails, but fallback must still pass
        const sabrOnly = {
            streaming_data: {
                adaptive_formats: [
                    { has_audio: true, mime_type: 'audio/webm', url: undefined, signature_cipher: undefined },
                    { has_audio: true, mime_type: 'audio/mp4', url: undefined }
                ],
                formats: [
                    { itag: 18, mime_type: 'video/mp4', has_audio: false, has_video: true, url: 'https://googlevideo.com/videoplayback?itag=18' }
                ]
            }
        };
        // isAudioFormat requires has_audio true, so progressive with has_audio false fails hasDirectOrDecipherableAudio
        assert.equal(hasDirectOrDecipherableAudio(sabrOnly), false, 'adaptive SABR-only should fail when progressive has_audio false mapping');
        // Legacy fallback must succeed even though isAudioFormat gating would not cover progressive has_audio false case (FreeTube#6977)
        assert.equal(hasLegacyProgressiveFallback(sabrOnly), true);

        // Also verify that when has_audio true, hasDirect would already succeed — fallback is extra safety
        const sabrWithAudioFlag = {
            streaming_data: {
                adaptive_formats: [{ has_audio: true, mime_type: 'audio/webm', url: undefined }],
                formats: [{ itag: 18, mime_type: 'video/mp4', has_audio: true, url: 'https://googlevideo.com/videoplayback?itag=18' }]
            }
        };
        assert.equal(hasDirectOrDecipherableAudio(sabrWithAudioFlag), true);
        assert.equal(hasLegacyProgressiveFallback(sabrWithAudioFlag), true);
    });

    it('accepts legacy progressive with signature_cipher (decipher needed)', () => {
        // Cipher case with has_audio false mapping — hasDirect fails, fallback passes and decipher path must work
        const cipherSabr = {
            streaming_data: {
                adaptive_formats: [{ has_audio: true, mime_type: 'audio/mp4', url: undefined }],
                formats: [{ itag: 18, mime_type: 'video/mp4', has_audio: false, signature_cipher: 's=abc&url=https%3A%2F%2Fgooglevideo.com%2Fvideoplayback' }]
            }
        };
        assert.equal(hasDirectOrDecipherableAudio(cipherSabr), false);
        assert.equal(hasLegacyProgressiveFallback(cipherSabr), true);
        // When has_audio true, hasDirect would be true via isAudioFormat + cipher
        const cipherWithAudio = {
            streaming_data: {
                adaptive_formats: [{ has_audio: true, mime_type: 'audio/mp4', url: undefined }],
                formats: [{ itag: 18, mime_type: 'video/mp4', has_audio: true, signature_cipher: 's=abc&url=https%3A%2F%2Fgooglevideo.com%2Fvideoplayback' }]
            }
        };
        assert.equal(hasDirectOrDecipherableAudio(cipherWithAudio), true);
        assert.equal(hasLegacyProgressiveFallback(cipherWithAudio), true);
    });

    it('accepts legacy with decipher function', () => {
        const decipherSabr = {
            streaming_data: {
                adaptive_formats: [{ has_audio: true, mime_type: 'audio/mp4' }],
                formats: [{ itag: 18, mime_type: 'video/mp4', has_audio: true, decipher: () => 'https://googlevideo.com/deciphered' }]
            }
        };
        assert.equal(hasLegacyProgressiveFallback(decipherSabr), true);
    });

    it('rejects when both adaptive and formats lack decipherable URL', () => {
        const empty = {
            streaming_data: {
                adaptive_formats: [{ has_audio: true, mime_type: 'audio/mp4' }],
                formats: [{ itag: 18, mime_type: 'video/mp4' }]
            }
        };
        assert.equal(hasDirectOrDecipherableAudio(empty), false);
        assert.equal(hasLegacyProgressiveFallback(empty), false);
    });

    it('regression: KGQG5Fv4Yrw_E-like WEB SABR-only must not throw No audio stream found', () => {
        // Simulate final fmt selection fallback: if audioCandidates empty but formats has legacy, fallback should pick it
        function isAudio(f) { return isAudioFormat(f); }
        const sd = {
            adaptive_formats: [{ has_audio: true, mime_type: 'audio/webm' }], // SABR no url
            formats: [{ itag: 18, mime_type: 'video/mp4', has_audio: true, has_video: true, url: 'https://googlevideo.com/legacy18' }]
        };
        const allCandidates = [...(sd.adaptive_formats || []), ...(sd.formats || [])];
        const audioCandidates = allCandidates.filter(isAudio);
        // audioCandidates will contain the progressive? has_audio true so yes, but if mapping bug made has_audio false, fallback still needed
        // Test the explicit legacy fallback path used in youtube.js after allCandidates[0] check
        let fmt = null;
        if (!fmt && audioCandidates.length > 0) {
            fmt = audioCandidates.find((f) => f.url && f.mime_type?.includes('mp4')) || audioCandidates.find((f) => f.url) || audioCandidates[0];
        }
        if (!fmt) fmt = allCandidates.find((f) => f && f.has_audio) || allCandidates[0];
        // Now apply SABR final fallback as in youtube.js
        if ((!fmt || (!fmt.url && !fmt.signature_cipher && !fmt.cipher)) && sd.formats && sd.formats.length) {
            const legacy = sd.formats.find((f) => f.url || f.signature_cipher || f.cipher || typeof f.decipher === 'function') || sd.formats[0];
            if (legacy) fmt = legacy;
        }
        assert.ok(fmt && fmt.url.includes('googlevideo'), 'legacy progressive should be selected for SABR-only');
        assert.equal(fmt.itag, 18);
    });
});

describe('regression: 6-client fallback must be present in youtube.js', () => {
    const ytPath = path.resolve(import.meta.dirname ?? path.dirname(new URL(import.meta.url).pathname), '../../ui/js/youtube.js');
    const src = fs.readFileSync(ytPath, 'utf8');

    it('source contains all 6 clients IOS, ANDROID, ANDROID_VR, TV, MWEB, WEB', () => {
        for (const c of ['IOS', 'ANDROID', 'ANDROID_VR', 'TV', 'MWEB', 'WEB']) {
            assert.ok(src.includes(`'${c}'`), `youtube.js missing client '${c}'`);
        }
    });

    it('clients are grouped by whether the held token can serve them', () => {
        // Asserting the three GROUPS is the durable form: the order is derived
        // from group membership, so a test that pinned the flattened array would
        // break on any reorder while saying nothing about the rule. The rule is:
        //   servable (web-family, the only ones our Web token works on)
        //   -> token-free (need no token at all)
        //   -> unmintable (need DroidGuard/iOSGuard, which we cannot produce)
        const group = (name) => {
            const m = src.match(new RegExp(`const ${name} = \\[([^\\]]+)\\]`));
            assert.ok(m, `${name} group not found in youtube.js`);
            return (m[1].match(/'([A-Z_]+)'/g) || []).map((x) => x.replace(/'/g, ''));
        };
        const servable = group('SERVABLE_CLIENTS');
        const tokenFree = group('TOKEN_FREE_CLIENTS');
        const unmintable = group('UNMINTABLE_CLIENTS');

        assert.deepEqual(servable, ['MWEB', 'WEB'],
            'web-family clients are the only ones a BotGuard token is valid on');
        assert.deepEqual(tokenFree, ['ANDROID_VR', 'TV'],
            'ANDROID_VR before TV: tv is DRM-capped without cookies and we send none');
        assert.deepEqual(unmintable, ['IOS', 'ANDROID'],
            'kept but last — ios does resolve, so this is a prediction, not an observation');

        // Disjoint and complete: no client may be in two groups, or in none.
        const all = [...servable, ...tokenFree, ...unmintable];
        assert.equal(new Set(all).size, all.length, 'a client appears in more than one group');
        for (const c of ['IOS', 'ANDROID', 'ANDROID_VR', 'TV', 'MWEB', 'WEB']) {
            assert.ok(all.includes(c), `${c} is in no group, so it can never be tried`);
        }

        // The two branches must differ only in whether SERVABLE leads: a token
        // we hold makes the servable group worth trying first, and without one
        // nothing in it can be served, so the token-free group should lead.
        const spread = (a, b, c) =>
            new RegExp(`\\[\\.\\.\\.${a}, \\.\\.\\.${b}, \\.\\.\\.${c}\\]`).test(src);
        assert.ok(spread('SERVABLE_CLIENTS', 'TOKEN_FREE_CLIENTS', 'UNMINTABLE_CLIENTS'),
            'with a token, the servable group must lead');
        assert.ok(spread('TOKEN_FREE_CLIENTS', 'SERVABLE_CLIENTS', 'UNMINTABLE_CLIENTS'),
            'without a token, the token-free group must lead since nothing can be served');
    });

    it('every client youtube.js can emit has its own User-Agent', () => {
        // A client with no uaMap entry silently falls back to ANDROID's, which
        // is the same UA/client mismatch that produced the byte-0 403.
        const uaBlock = src.match(/const uaMap = \{([\s\S]*?)\n\s*\};/);
        assert.ok(uaBlock, 'uaMap not found');
        for (const c of ['IOS', 'ANDROID', 'ANDROID_VR', 'TV', 'MWEB', 'WEB']) {
            assert.ok(uaBlock[1].includes(`'${c}':`), `uaMap has no entry for ${c}`);
        }
    });

    it('every client the resolver can emit is a name the vendored InnerTube library accepts', () => {
        // WEB_SAFARI shipped in v2.6.51 as "the one web client we had never
        // tried". It is not a client name at all: youtubei.esm.mjs validates
        // against a fixed allowlist and throws `Invalid client: WEB_SAFARI`, so
        // it failed in 8ms on every resolve and could never return a url. The
        // device report is what surfaced it, which means no test was reading the
        // thing that decides whether a client name is real.
        const allowlist = ['IOS', 'WEB', 'MWEB', 'YTKIDS', 'YTMUSIC', 'ANDROID',
            'ANDROID_VR', 'VISIONOS', 'YTSTUDIO_ANDROID', 'YTMUSIC_ANDROID',
            'TV', 'TV_SIMPLY', 'TV_EMBEDDED', 'WEB_EMBEDDED', 'WEB_CREATOR'];
        const group = (name) => {
            const m = src.match(new RegExp(`const ${name} = \\[([^\\]]+)\\]`));
            assert.ok(m, `${name} group not found in youtube.js`);
            return (m[1].match(/'([A-Z_]+)'/g) || []).map((x) => x.replace(/'/g, ''));
        };
        const emitted = [...group('SERVABLE_CLIENTS'), ...group('TOKEN_FREE_CLIENTS'),
            ...group('UNMINTABLE_CLIENTS')];
        assert.ok(emitted.length > 0, 'no clients parsed — the regex must match the const declarations');
        for (const c of emitted) {
            assert.ok(allowlist.includes(c),
                `${c} is not in the vendored library's client allowlist, so actions.execute throws ` +
                `"Invalid client: ${c}" and it can never resolve`);
        }
    });

    it('getInfo fallback tries 6 clients', () => {
        // fallback now uses orderedClients.map(cl => getInfo(... client: cl)) — still must cover all 6
        assert.ok(src.includes('orderedClients') && src.includes('getInfo'), 'fallback must be PO-token-aware via orderedClients + getInfo');
        assert.ok(src.includes("'IOS'") && src.includes("'WEB'"), 'fallback must mention IOS and WEB');
        // at least one explicit client literal check remains (uaMap etc)
        const hasTV = src.includes("'TV'") || src.includes('"TV"');
        assert.ok(hasTV, 'fallback must mention TV');
    });

    it('nativeFetch bridges via http_fetch (bypasses WebView CORS)', () => {
        assert.ok(src.includes('http_fetch'), 'youtube.js must delegate to Rust http_fetch via nativeFetch');
        assert.ok(src.includes('nativeFetch'), 'nativeFetch helper missing');
    });

    it('CSP in tauri.conf.json allows https connect-src (required for youtubei)', () => {
        const cspPath = path.resolve(path.dirname(ytPath), '../../tauri.conf.json');
        const csp = JSON.parse(fs.readFileSync(cspPath, 'utf8')).app.security.csp;
        assert.ok(csp.includes('connect-src'), 'CSP missing connect-src');
        assert.ok(csp.includes('https:'), 'CSP connect-src must include https: for youtubei/googlevideo');
    });

    it('resolver returns client-matched headers (prevents googlevideo 403)', () => {
        // ytDl fix for 8BWnhTscTMs: title resolved but Rust reqwest 403'd
        // because UA/Referer mismatched IOS context. Headers must travel to Rust.
        assert.ok(src.includes("headers") && src.includes("User-Agent"), 'youtube.js must return headers with User-Agent');
        assert.ok(src.includes("winningClient") && src.includes("uaMap"), 'youtube.js must map winningClient -> UA');
        assert.ok(src.includes("Referer") && src.includes("Origin"), 'headers must include Referer/Origin');
    });

    it('SABR-only fallback must be present (FreeTube#6977)', () => {
        assert.ok(src.includes('hasLegacyProgressiveFallback'), 'youtube.js missing hasLegacyProgressiveFallback helper');
        assert.ok(src.includes('SABR-only'), 'youtube.js missing SABR-only fallback comment/marker');
        assert.ok(src.includes('formats') && src.includes('signature_cipher'), 'fallback must handle signature_cipher for progressive');
        // Client order follows the token we hold, not a guess. With a Web token
        // only MWEB/WEB can use it; TV and ANDROID_VR need no token and still
        // return plain CDN urls; ANDROID needs a DroidGuard token we cannot mint
        // and was measured returning no audio urls at all.
        //
        // The old expectation repeated a claim measurement disproved — that
        // ANDROID_VR "returns only muxed itag 18 and 403s past ~60s" and belongs
        // last. On device it returned 22 adaptive / 4 audio WITH urls at itag
        // 140 audio-only, while ANDROID was the client returning nothing.
        // The order is now derived from three named groups; see the grouping
        // test in the 6-client suite for the assertion of record. What matters
        // here is only that the SABR-era demotions are gone: ANDROID_VR is no
        // longer last resort, and ANDROID (which needs a token we cannot mint
        // and was measured returning no audio urls) is.
        assert.ok(src.includes('TOKEN_FREE_CLIENTS') && src.includes('UNMINTABLE_CLIENTS'),
            'orderedClients must be derived from the named groups');
        const unmintable = src.match(/const UNMINTABLE_CLIENTS = \[([^\]]+)\]/);
        assert.ok(unmintable, 'UNMINTABLE_CLIENTS not found');
        assert.equal((unmintable[1].match(/'([A-Z_]+)'/g) || []).pop(), "'ANDROID'",
            'ANDROID must be last of the unmintable pair');
        assert.ok(!/const\s+UNMINTABLE_CLIENTS\s*=\s*\[[^\]]*'ANDROID_VR'/.test(src),
            'ANDROID_VR must NOT be demoted to unmintable — it needs no token and returns real audio');
    });

    it('resolver records a per-client reaction report (SABR/403/format counts)', () => {
        // Release builds write nothing to logcat, so every attempted client must
        // leave a structured record: playability status, format counts, whether
        // adaptive formats carried urls, and whether a SABR streaming url was
        // present. The report is surfaced in the download row + "Copy report".
        assert.ok(src.includes('clientReport'), 'youtube.js must build a clientReport array');
        assert.ok(src.includes('sabrStreamingUrl'), 'report must record serverAbrStreamingUrl presence');
        assert.ok(src.includes('adaptiveWithUrl'), 'report must count adaptive formats carrying urls');
        assert.ok(src.includes('audioWithUrl'), 'report must count audio formats carrying urls');
        assert.ok(src.includes('formatClientReport'), 'youtube.js must format the report for display');
        assert.ok(src.includes('client_report_text'), 'resolved track must expose client_report_text');
        assert.ok(src.includes('entry.chosen = true') || src.includes('chosen: false'), 'report must mark the winning client');
        assert.ok(src.includes("reason = 'sabr-only'") || src.includes("'sabr-only'"), 'sabr-only clients must be labelled');
    });

    it('downloads.js surfaces the per-client report with a copy action', () => {
        const dpath = path.resolve(import.meta.dirname ?? path.dirname(new URL(import.meta.url).pathname), '../../ui/js/modules/downloads.js');
        const dsrc = fs.readFileSync(dpath, 'utf8');
        assert.ok(dsrc.includes('client_report'), 'downloads.js must read resolved.client_report');
        assert.ok(dsrc.includes('copy-client-report'), 'downloads.js must offer a Copy report action');
        assert.ok(dsrc.includes('_buildClientReportText'), 'downloads.js must build a copyable report');
        assert.ok(dsrc.includes('avoidLegacyProgressive'), 'retry must still refuse the SABR legacy path');
    });
});

describe('Download error payload handling (error vs error_message)', () => {
    it('downloads.js updateDownloadProgressUI extracts error or error_message correctly', () => {
        let capturedHtml = '';
        const mockRow = {
            classList: { add: () => {}, remove: () => {} },
            style: {},
            dataset: {},
            set innerHTML(val) { capturedHtml = val; }
        };
        const mockList = {
            dataset: {},
            addEventListener: () => {},
            querySelector: () => mockRow,
            prepend: () => {}
        };

        // Minimal mock document for downloads.js updateDownloadProgressUI
        global.document = {
            getElementById: (id) => (id === 'downloads-list' ? mockList : null)
        };

        const context = {
            ...downloadMethods,
            escapeHtml: (str) => str || ''
        };

        // Test with `error` field
        context.updateDownloadProgressUI({ id: 'd1', status: 'failed', error: 'HTTP 403 Forbidden', url: 'https://example.com/audio' });
        assert.ok(capturedHtml.includes('HTTP 403 Forbidden'), 'HTML should contain error message from `error` property');

        // Test with `error_message` field
        context.updateDownloadProgressUI({ id: 'd2', status: 'failed', error_message: 'Connection timed out', url: 'https://example.com/audio' });
        assert.ok(capturedHtml.includes('Connection timed out'), 'HTML should contain error message from `error_message` property');

        // Clean up global mock
        delete global.document;
    });

    it('coreMethods.extractErrorMessage handles error, error_message, message, and fallbacks', () => {
        assert.equal(coreMethods.extractErrorMessage({ error: 'Direct error' }), 'Direct error');
        assert.equal(coreMethods.extractErrorMessage({ error: { message: 'Nested object error message' } }), 'Nested object error message');
        assert.equal(coreMethods.extractErrorMessage({ error: { error: 'Nested object error string' } }), 'Nested object error string');
        assert.equal(coreMethods.extractErrorMessage({ error_message: 'Error message field' }), 'Error message field');
        assert.equal(coreMethods.extractErrorMessage({ error: 'Primary error', error_message: 'Secondary error_message' }), 'Primary error');
        assert.equal(coreMethods.extractErrorMessage({ message: 'Message field' }), 'Message field');
        assert.equal(coreMethods.extractErrorMessage(null, 'Custom fallback'), 'Custom fallback');
        assert.equal(coreMethods.extractErrorMessage('Plain string error'), 'Plain string error');
    });
});

describe('nativeFetch header extraction (Tauri bridge)', () => {
    it('extracts headers from plain object, array, and Headers-like', () => {
        // mirrors the fixed extractHeaders in youtube.js — Array.isArray before Headers-like
        function extract(clean, hdrs) {
            if (!hdrs) return;
            if (Array.isArray(hdrs)) for (const [k, v] of hdrs) clean[String(k)] = String(v);
            else if (typeof hdrs.forEach === 'function') hdrs.forEach((v, k) => { clean[String(k)] = String(v); });
            else if (typeof hdrs === 'object') for (const [k, v] of Object.entries(hdrs)) clean[String(k)] = String(v);
        }
        const c1 = {};
        extract(c1, { 'x-foo': 'bar' });
        assert.equal(c1['x-foo'], 'bar');
        const c2 = {};
        extract(c2, [['x-baz', 'qux']]);
        assert.equal(c2['x-baz'], 'qux');
        const c3 = {};
        const h = { forEach(fn) { fn('val', 'x-hdr'); } };
        extract(c3, h);
        assert.equal(c3['x-hdr'], 'val');
    });
});

describe('PlayerController queue pre-rendered HTML & observer cleanup', () => {
    const playerPath = path.resolve(import.meta.dirname ?? path.dirname(new URL(import.meta.url).pathname), '../../ui/js/player.js');
    const src = fs.readFileSync(playerPath, 'utf8');

    it('ui/js/player.js renders queue via get_queue_html and has no renderQueueTrackRow', () => {
        assert.ok(src.includes("get_queue_html"), 'renderQueuePanel must invoke get_queue_html');
        assert.ok(!src.includes('renderQueueTrackRow'), 'redundant renderQueueTrackRow should be removed');
    });

    it('ui/js/player.js removes MutationObserver overhead', () => {
        assert.ok(!src.includes('new MutationObserver'), 'MutationObserver overhead should be removed from player.js');
    });
});

// Video IDs are kept because this is where the behaviour was first seen on a
// Jio IPv6 residential line. The expectation has since been INVERTED, and the
// reason matters: the token this app can mint is a BotGuard/WEB one, and a PO
// token is platform-bound. Carrying it on a TV (or ios/android_vr) URL puts a
// foreign token on a request that is sent with a client-matched User-Agent, and
// googlevideo answers 403 Forbidden at byte 0 with an empty text/plain body.
// The old test asserted the opposite because it exercised a hand-written *copy*
// of the append logic rather than the real code path, so it passed no matter
// what actually shipped.
describe('pot is not carried on a non-web client url (TV / YAD 7C4-TAWg7QA lineage)', () => {
    it('a web-minted token stays off a TV url', () => {
        const fakeToken = 'TEST_POT_TOKEN_6h_CACHE_123';
        const googlevideoUrl = 'https://rr1---sn-gwpa-cived.googlevideo.com/videoplayback?expire=1234567890&ei=test&ip=2409%3A40c4%3A35b%3Ab681%3A8000%3A%3A&itag=140&c=TV&cplayer=UNIPLAYER&pot_placeholder=0';
        const r = applyPoTokenToUrl(googlevideoUrl, { winningClient: 'TV', token: fakeToken, tokenIsWebBound: true });
        assert.equal(r.action, 'no-token', 'TV must not receive a Web-bound token');
        assert.ok(!r.url.includes('pot='), `web token leaked onto a TV url: ${r.url}`);
        assert.equal(new URL(r.url).searchParams.get('itag'), '140', 'other params must survive');
    });

    it('strips a pot that the vendored decipher already put on a TV url', () => {
        const fakeToken = 'TEST_POT_TOKEN_6h_CACHE_123';
        const withPot = 'https://rr1---sn-gwpa-cived.googlevideo.com/videoplayback?itag=140&c=TV&pot=' + fakeToken;
        const r = applyPoTokenToUrl(withPot, { winningClient: 'TV', token: fakeToken, tokenIsWebBound: true });
        assert.equal(r.action, 'stripped');
        assert.ok(!r.url.includes('pot='), r.url);
    });

    it('still carries a token the user supplied in Settings, even on TV', () => {
        // We did not mint that one, so it may legitimately be a TV/iOS token and
        // it is not ours to second-guess.
        const url = 'https://rr1---sn-gwpa-cived.googlevideo.com/videoplayback?itag=140&c=TV';
        const r = applyPoTokenToUrl(url, { winningClient: 'TV', token: 'USER_SUPPLIED', tokenIsWebBound: false });
        assert.equal(r.action, 'attached');
        assert.equal(new URL(r.url).searchParams.get('pot'), 'USER_SUPPLIED');
    });
});

describe('pot placement in youtube.js and the vendored decipher', () => {
    const ytPath = path.resolve(import.meta.dirname ?? path.dirname(new URL(import.meta.url).pathname), '../../ui/js/youtube.js');
    const vendorPath = path.resolve(path.dirname(ytPath), '../vendor/youtubei.esm.mjs');

    it('youtube.js delegates pot placement to the tested pot_scope module', () => {
        const src = fs.readFileSync(ytPath, 'utf8');
        // The decision used to be inline and unconditional, which is what put a
        // Web/BotGuard token on ios/android_vr URLs and earned a 403 at byte 0.
        // It now lives in modules/pot_scope.js so it can be unit-tested; this
        // test only asserts the delegation exists and still handles both
        // spellings of the option. The behaviour itself is covered by
        // scripts/tests/pot_scope.test.js, which imports the real module.
        assert.ok(
            src.includes("import('./modules/pot_scope.js')"),
            'youtube.js must import modules/pot_scope.js for the pot decision'
        );
        assert.ok(
            src.includes('applyPoTokenToUrl'),
            'youtube.js must delegate to applyPoTokenToUrl'
        );
        assert.ok(
            src.includes('tokenIsWebBound = true'),
            'youtube.js must record that a token it minted itself is Web-bound'
        );
        assert.ok(
            src.includes('opts.poToken') && src.includes('opts.po_token'),
            'youtube.js pot logic must handle both poToken spellings'
        );
        // The old unconditional append must be gone from youtube.js itself.
        assert.ok(
            !src.includes("u.searchParams.set('pot'"),
            "youtube.js must not append pot inline any more (it is pot_scope.js's job)"
        );
    });

    it('vendor youtubei.esm.mjs no longer guards pot on sabr', () => {
        const v = fs.readFileSync(vendorPath, 'utf8');
        // Vendor must still set pot
        const hasPot = v.includes("set('pot'") || v.includes('set("pot"') || v.includes("searchParams.set('pot'") || v.includes('.set("pot"');
        assert.ok(hasPot, 'vendor must contain pot set logic');
        // Old buggy code: a.searchParams.get("sabr")!=="1"&&this.po_token&&a.searchParams.set("pot",...
        // Must not contain sabr-guarded pattern at all (minified file is single line, so check substring)
        assert.ok(!v.includes('get("sabr")!=="1"&&this.po_token'), 'vendor should not contain sabr-guarded pot logic (old: sabr!=1 && po_token)');
        assert.ok(!v.includes("get('sabr')") || !v.includes('sabr') || v.indexOf('sabr') === -1 || !v.slice(Math.max(0, v.indexOf('a.searchParams.set("pot"')-200), v.indexOf('a.searchParams.set("pot"')).includes('sabr'), 'pot context must not be sabr-guarded');
        // Ensure unconditional pot append exists (this.po_token && ... set pot) and its 200-char context has no sabr
        const potIdx = v.indexOf('a.searchParams.set("pot"');
        const potIdx2 = v.indexOf("a.searchParams.set('pot'");
        const idx = potIdx !== -1 ? potIdx : potIdx2;
        assert.ok(idx !== -1, 'pot set index should be found');
        const before = v.slice(Math.max(0, idx - 200), idx);
        assert.ok(!before.includes('sabr'), `pot context should not contain sabr guard: ${before.slice(-100)}`);
        assert.ok(v.includes('this.po_token&&') || v.includes('this.po_token &&'), 'vendor should have unconditional this.po_token && set pot');
    });
});

describe('Phase 1 Frontend HTMX & JS Reduction', () => {
    const viewsPath = path.resolve(import.meta.dirname ?? path.dirname(new URL(import.meta.url).pathname), '../../ui/js/modules/views.js');
    const viewsSrc = fs.readFileSync(viewsPath, 'utf8');

    it('views.js does not contain _albumMap or _artistMap memory caches', () => {
        assert.ok(!viewsSrc.includes('this._albumMap'), 'views.js should not retain this._albumMap memory cache');
        assert.ok(!viewsSrc.includes('this._artistMap'), 'views.js should not retain this._artistMap memory cache');
    });

    it('loadAlbumsView and loadArtistsView invoke get_albums_grid_html and get_artists_grid_html', () => {
        assert.ok(viewsSrc.includes("get_albums_grid_html"), 'loadAlbumsView should invoke get_albums_grid_html');
        assert.ok(viewsSrc.includes("get_artists_grid_html"), 'loadArtistsView should invoke get_artists_grid_html');
    });

    it('renderLibraryTracks invokes get_library_tracks_html', () => {
        assert.ok(viewsSrc.includes("get_library_tracks_html"), 'renderLibraryTracks should invoke get_library_tracks_html');
    });

    it('loadHomeView invokes get_home_shelves_html', () => {
        assert.ok(viewsSrc.includes("get_home_shelves_html"), 'loadHomeView should invoke get_home_shelves_html');
    });

    it('loadSearchView invokes get_search_results_html', () => {
        assert.ok(viewsSrc.includes("get_search_results_html"), 'loadSearchView should invoke get_search_results_html');
    });
});

describe('Phase 2 & Phase 3 Frontend Streamlining (Queue Drawer, Settings Binding, DOM Delegation)', () => {
    const viewsPath = path.resolve(import.meta.dirname ?? path.dirname(new URL(import.meta.url).pathname), '../../ui/js/modules/views.js');
    const viewsSrc = fs.readFileSync(viewsPath, 'utf8');
    const corePath = path.resolve(import.meta.dirname ?? path.dirname(new URL(import.meta.url).pathname), '../../ui/js/modules/core.js');
    const coreSrc = fs.readFileSync(corePath, 'utf8');
    const playerPath = path.resolve(import.meta.dirname ?? path.dirname(new URL(import.meta.url).pathname), '../../ui/js/player.js');
    const playerSrc = fs.readFileSync(playerPath, 'utf8');

    it('loadSettingsView uses concise unified listener and invokes get_settings & update_settings', () => {
        assert.ok(viewsSrc.includes("get_settings"), 'loadSettingsView should invoke get_settings');
        assert.ok(viewsSrc.includes("update_settings"), 'loadSettingsView should invoke update_settings');
        assert.ok(viewsSrc.includes("settingsView.addEventListener('change'"), 'loadSettingsView should have unified change listener');
        assert.ok(viewsSrc.includes("settingsView.addEventListener('click'"), 'loadSettingsView should have unified click listener');
    });

    it('core.js handles global event delegation for play-row and play-card', () => {
        assert.ok(coreSrc.includes('data-role="play-row"'), 'core.js must delegate data-role="play-row"');
        assert.ok(coreSrc.includes('data-role="play-card"'), 'core.js must delegate data-role="play-card"');
        assert.ok(coreSrc.includes('playDelegationBound'), 'core.js should mark play delegation bound');
    });

    it('player.js renders queue via get_queue_html and removes MutationObserver', () => {
        assert.ok(playerSrc.includes('get_queue_html'), 'player.js must invoke get_queue_html');
        assert.ok(!playerSrc.includes('renderQueueTrackRow'), 'player.js should not have renderQueueTrackRow');
        assert.ok(!playerSrc.includes('new MutationObserver'), 'player.js should not have MutationObserver');
    });

    it('views.js and core.js resolve play event delegation without duplicate listeners', () => {
        // views.js should not have redundant document play click listener or inline onclick on play-track-btn
        assert.ok(!viewsSrc.includes("closest('.play-shelf-btn, .play-track-btn')"), 'views.js should not have redundant play click listener');
        assert.ok(!viewsSrc.includes("onclick=\"event.stopPropagation(); window._safePlayTrack"), 'views.js renderTrackRows should not have inline onclick play handler');
        // core.js should have unified delegation
        assert.ok(coreSrc.includes('handlePlayDelegate'), 'core.js must have handlePlayDelegate');
    });

    it('player.js Escape key closes overlay-root in addition to queue drawer', () => {
        assert.ok(playerSrc.includes("overlay-root"), 'player.js should reference overlay-root on Escape');
        assert.ok(playerSrc.includes("toggleFullScreenQueue"), 'player.js should handle queue drawer toggle');
    });

    it('core.js provides global back dismissal handler for popstate and modals', () => {
        assert.ok(coreSrc.includes('dismissOpenOverlays'), 'core.js must implement dismissOpenOverlays');
        assert.ok(coreSrc.includes('initBackDismissalHandler'), 'core.js must implement initBackDismissalHandler');
        assert.ok(coreSrc.includes('popstate'), 'core.js should handle popstate');
        assert.ok(coreSrc.includes('.modal-backdrop'), 'core.js should dismiss modal-backdrop');
    });
});

describe('Audio Stream Selection & Format Integrity', () => {
    const ytPath = path.resolve(import.meta.dirname ?? path.dirname(new URL(import.meta.url).pathname), '../../ui/js/youtube.js');
    const src = fs.readFileSync(ytPath, 'utf8');

    it('youtube.js exposes selectBestAudioFormat and pickAudioFormat', () => {
        assert.ok(src.includes('selectBestAudioFormat'), 'youtube.js missing selectBestAudioFormat');
        assert.ok(src.includes('pickAudioFormat'), 'youtube.js missing pickAudioFormat');
        assert.ok(src.includes('hasValidAudioContainer'), 'youtube.js missing hasValidAudioContainer');
        assert.ok(src.includes('hasValidAudioCodec'), 'youtube.js missing hasValidAudioCodec');
    });

    it('selectBestAudioFormat prioritizes valid audio codecs and Content-Length', () => {
        function hasValidAudioCodec(f) {
            if (!f) return false;
            const mime = String(f.mime_type || '').toLowerCase();
            return mime.includes('opus') || mime.includes('mp4a') || mime.includes('aac') || mime.includes('vorbis') || mime.includes('flac') || mime.startsWith('audio/mp4') || mime.startsWith('audio/webm') || mime.startsWith('audio/ogg');
        }
        function selectBestAudioFormat(candidates, quality = 'best', targetContainer = null) {
            if (!candidates || !candidates.length) return null;
            const isDecipherable = (f) => Boolean(f && (f.url || f.signature_cipher || f.cipher || typeof f.decipher === 'function'));
            const valid = candidates.filter((f) => f && isDecipherable(f));
            if (!valid.length) return candidates[0] || null;
            const scoreFormat = (f) => {
                let score = 0;
                const mime = String(f.mime_type || '').toLowerCase();
                if (hasValidAudioCodec(f)) score += 1000;
                if (targetContainer) {
                    if (targetContainer === 'webm' && (mime.includes('webm') || mime.includes('opus'))) score += 500;
                    else if (targetContainer === 'mp4' && (mime.includes('mp4') || mime.includes('m4a'))) score += 500;
                } else {
                    if (mime.includes('opus') || mime.includes('webm')) score += 100;
                }
                if ((f.has_audio && !f.has_video) || !f.has_video) score += 200;
                if (f.url) score += 50;
                if ((f.content_length && f.content_length > 0) || (f.contentLength && parseInt(f.contentLength, 10) > 0)) {
                    score += 50;
                }
                return score;
            };
            const sorted = [...valid].sort((a, b) => (scoreFormat(b) - scoreFormat(a)) || ((b.bitrate || 0) - (a.bitrate || 0)));
            return sorted[0] || valid[0];
        }

        const candidates = [
            { itag: 999, mime_type: 'audio/unknown', bitrate: 256000, url: 'https://example.com/u' },
            { itag: 140, mime_type: 'audio/mp4; codecs="mp4a.40.2"', bitrate: 128000, url: 'https://example.com/140', content_length: 5000000 },
            { itag: 251, mime_type: 'audio/webm; codecs="opus"', bitrate: 160000, url: 'https://example.com/251', content_length: 6000000 }
        ];

        const best = selectBestAudioFormat(candidates);
        assert.equal(best.itag, 251, 'Should prioritize opus/webm with valid codec, higher bitrate and content length');

        const bestMp4 = selectBestAudioFormat(candidates, 'best', 'mp4');
        assert.equal(bestMp4.itag, 140, 'Should respect target container mp4');
    });

    it('SABR container validation filters out non-audio containers', () => {
        function hasValidAudioContainer(f) {
            if (!f || !f.mime_type) return false;
            const mime = String(f.mime_type).toLowerCase();
            return (
                mime.startsWith('audio/mp4') ||
                mime.startsWith('audio/webm') ||
                mime.startsWith('audio/m4a') ||
                mime.startsWith('audio/ogg') ||
                mime.startsWith('audio/opus') ||
                mime.startsWith('video/mp4') ||
                mime.startsWith('video/webm')
            );
        }

        assert.equal(hasValidAudioContainer({ mime_type: 'audio/mp4' }), true);
        assert.equal(hasValidAudioContainer({ mime_type: 'audio/webm; codecs="opus"' }), true);
        assert.equal(hasValidAudioContainer({ mime_type: 'video/mp4; codecs="avc1.42001E, mp4a.40.2"' }), true);
        assert.equal(hasValidAudioContainer({ mime_type: 'video/3gpp' }), false);
        assert.equal(hasValidAudioContainer({ mime_type: 'video/x-flv' }), false);
        assert.equal(hasValidAudioContainer({ mime_type: 'application/x-mpegURL' }), false);
    });
});

describe('Format preference scoring (itag 140 m4a rodio compat — opus would DecodeError)', () => {
    it('scoreFormat({itag:140, mimeType:"audio/mp4"}) > scoreFormat({itag:251, mimeType:"audio/webm"})', () => {
        function scoreFormat(fmt) {
            const mime = (fmt.mimeType || '').toLowerCase();
            if (fmt.itag === 140) return 3;
            if (mime.includes('mp4') && !mime.includes('video')) return 2;
            if (mime.includes('m4a')) return 2;
            return 1;
        }
        assert.ok(scoreFormat({ itag: 140, mimeType: 'audio/mp4' }) > scoreFormat({ itag: 251, mimeType: 'audio/webm' }), 'itag 140 must outrank webm/opus');
        assert.equal(scoreFormat({ itag: 140, mimeType: 'audio/mp4' }), 3);
        assert.equal(scoreFormat({ itag: 140, mimeType: 'audio/mp4; codecs="mp4a.40.2"' }), 3);
        assert.equal(scoreFormat({ itag: 251, mimeType: 'audio/webm' }), 1);
        assert.equal(scoreFormat({ itag: 251, mimeType: 'audio/webm; codecs="opus"' }), 1);
        assert.equal(scoreFormat({ itag: 139, mimeType: 'audio/mp4' }), 2);
        assert.equal(scoreFormat({ itag: 18, mimeType: 'video/mp4' }), 1, 'video/mp4 must not get mp4 audio score');
    });

    it('youtube.js source implements scoreFormat with itag 140 and m4a preference (no opus DecodeError)', () => {
        const ytPath = path.resolve(import.meta.dirname ?? path.dirname(new URL(import.meta.url).pathname), '../../ui/js/youtube.js');
        const src = fs.readFileSync(ytPath, 'utf8');
        assert.ok(src.includes('function scoreFormat'), 'youtube.js must define global scoreFormat function');
        assert.ok(src.includes('if (fmt.itag === 140) return 3'), 'youtube.js scoreFormat must handle itag 140');
        assert.ok(src.includes("mime.includes('mp4') && !mime.includes('video')"), 'youtube.js scoreFormat must check mp4 without video');
        assert.ok(src.includes("mime.includes('m4a')"), 'youtube.js scoreFormat must check m4a');
        // Ensure rodio compat: old opus-preferring code removed
        assert.ok(!src.includes("mime.includes('opus') || mime.includes('webm')) score += 100"), 'youtube.js must NOT prefer opus/webm by default (rodio lacks opus)');
    });

    it('selectBestAudioFormat via youtube.js scoring prefers itag 140 m4a over webm/opus', () => {
        function scoreFormat(fmt) {
            const mime = (fmt.mimeType || fmt.mime_type || '').toLowerCase();
            if (fmt.itag === 140) return 3;
            if (mime.includes('mp4') && !mime.includes('video')) return 2;
            if (mime.includes('m4a')) return 2;
            return 1;
        }
        const candidates = [
            { itag: 251, mime_type: 'audio/webm; codecs="opus"', bitrate: 160000, url: 'https://example.com/251', has_audio: true, has_video: false },
            { itag: 140, mime_type: 'audio/mp4; codecs="mp4a.40.2"', bitrate: 128000, url: 'https://example.com/140', has_audio: true, has_video: false },
            { itag: 139, mime_type: 'audio/mp4', bitrate: 48000, url: 'https://example.com/139', has_audio: true, has_video: false },
        ];
        const sorted = [...candidates].sort((a, b) => scoreFormat(b) - scoreFormat(a) || (b.bitrate - a.bitrate));
        assert.equal(sorted[0].itag, 140, 'itag 140 should be first after m4a-preferring sort');
        assert.ok(scoreFormat(sorted[0]) > scoreFormat(candidates[0]), 'top sorted must outrank webm');
    });
});

describe('YouTube Search & Streaming Integration', () => {
    const ytPath = path.resolve(import.meta.dirname ?? path.dirname(new URL(import.meta.url).pathname), '../../ui/js/youtube.js');
    const dlPath = path.resolve(import.meta.dirname ?? path.dirname(new URL(import.meta.url).pathname), '../../ui/partials/download.html');
    const dlsModulePath = path.resolve(import.meta.dirname ?? path.dirname(new URL(import.meta.url).pathname), '../../ui/js/modules/downloads.js');
    const playerJsPath = path.resolve(import.meta.dirname ?? path.dirname(new URL(import.meta.url).pathname), '../../ui/js/player.js');

    it('ui/js/youtube.js search returns normalized structure up to 15 items', () => {
        const src = fs.readFileSync(ytPath, 'utf8');
        assert.ok(src.includes('async search(query, opts = {})'), 'search method must exist');
        assert.ok(src.includes('if (out.length >= 15) break;'), 'search results must be capped at 15');
        assert.ok(src.includes("url: `https://www.youtube.com/watch?v=${id}`"), 'url must be standard watch url');
        assert.ok(src.includes('channel:'), 'channel field must be returned');
        assert.ok(src.includes('duration_text:'), 'duration_text field must be returned');
        assert.ok(src.includes('thumbnail:'), 'thumbnail field must be returned');
    });

    it('ui/partials/download.html has sleek neu-glass search card with placeholder and spinner', () => {
        const html = fs.readFileSync(dlPath, 'utf8');
        assert.ok(html.includes('id="youtube-search-results"'), 'must have #youtube-search-results container');
        assert.ok(html.includes('id="youtube-search-spinner"'), 'must have #youtube-search-spinner');
        assert.ok(html.includes('Search YouTube songs, artists, albums…'), 'must have specified placeholder');
        assert.ok(html.includes('card neu-glass'), 'must have card neu-glass glassmorphic styling');
        assert.ok(html.includes('data-lucide="search"'), 'must have search icon');
    });

    it('ui/js/modules/downloads.js provides streamYouTubeSearchResult and downloadSearchResult', () => {
        assert.equal(typeof downloadMethods.streamYouTubeSearchResult, 'function', 'streamYouTubeSearchResult must be a function');
        assert.equal(typeof downloadMethods.downloadSearchResult, 'function', 'downloadSearchResult must be a function');
        const src = fs.readFileSync(dlsModulePath, 'utf8');
        assert.ok(src.includes('.track-row.neu-glass') || src.includes('track-row neu-glass'), 'must render track-row neu-glass');
        assert.ok(src.includes('play-yt-btn'), 'must include play-yt-btn');
        assert.ok(src.includes('download-yt-btn'), 'must include download-yt-btn');
        assert.ok(src.includes('streamYouTubeSearchResult'), 'must call streamYouTubeSearchResult');
        assert.ok(src.includes('new Audio'), 'must instantiate Audio element for streaming');
    });

    it('downloads.js auto-retries truncated streams by rotating InnerTube client', () => {
        // The backend rejects a file whose DECODED audio is short
        // ("Truncated download: only 99s of 287s ..."). That is not a dropped
        // connection — the transfer finished at 100% of the advertised bytes —
        // so resuming cannot help; a new url from another client must be tried.
        const src = fs.readFileSync(dlsModulePath, 'utf8');
        assert.ok(
            /isTruncated\s*=\s*\/Truncated download\/i\.test\(errRaw\)/.test(src),
            'auto-retry must detect the backend "Truncated download" verdict (it previously only matched "Incomplete download", so retries never fired)'
        );
        assert.ok(
            src.includes('avoidLegacyProgressive'),
            'a truncated retry must refuse the SABR-only legacy progressive fallback'
        );
        assert.ok(
            /MAX_AUTO_RETRIES\s*=\s*3/.test(src),
            'auto-retry must be budgeted per track to avoid an endless retry chain'
        );
    });

    it('youtube.js deprioritises a legacy-progressive-only result', () => {
        // Race evidence from the device (v2.6.44): ANDROID answered first but could
        // only offer the SABR-only muxed itag 18, while IOS and ANDROID_VR had real
        // audio-only urls ready. `Promise.any` then picked the muxed 360p rendition,
        // which is exactly the one the server truncates. A legacy-only result must
        // therefore wait briefly so a genuine audio url can win the race.
        const ysrc = fs.readFileSync(ytPath, 'utf8');
        assert.ok(ysrc.includes('LEGACY_RESULT_DELAY_MS'), 'a legacy-only delay must be defined');
        const delays = ysrc.match(/setTimeout\(resolve, LEGACY_RESULT_DELAY_MS\)/g) || [];
        assert.equal(delays.length, 2, 'both the actions.execute and getInfo races must delay legacy results');
        assert.ok(
            /hasLegacyProgressiveFallback\([\s\S]{0,600}LEGACY_RESULT_DELAY_MS/.test(ysrc),
            'the delay must sit on the legacy-progressive branch'
        );
    });

    it('downloads.js only rotates to a client the report proves can serve audio', async () => {
        // Real per-client report from the device (v2.6.43, track BElct8HWkp8):
        // only IOS and ANDROID_VR handed out audio urls; ANDROID was SABR-only
        // and MWEB/TV/WEB came back UNPLAYABLE. Rotating into the SABR-only one
        // burns a download (it ends on a 403 for muxed itag 18).
        //
        // The UNPLAYABLE three were lumped in with it here, and that lumping was
        // the defect. "Returned nothing on this attempt" and "provably cannot
        // ever serve" are different facts, and only the second is safe to make
        // permanent — see the 2026-09-27 hsXKOsnptw4 measurement quoted in the
        // next test. So this test now pins the half that is real (the SABR-only
        // client is never asked) and asserts the corrected behaviour for the
        // half that was not.
        const report = [
            { client: 'IOS', status: 'OK', audioWithUrl: 2, adaptiveWithUrl: 20 },
            { client: 'ANDROID', status: 'OK', audioWithUrl: 0, adaptive: 25, adaptiveWithUrl: 0, progressiveWithUrl: 0, sabrStreamingUrl: true },
            { client: 'ANDROID_VR', status: 'OK', audioWithUrl: 4, adaptiveWithUrl: 22 },
            { client: 'TV', status: 'UNPLAYABLE', audioWithUrl: 0 },
            { client: 'MWEB', status: 'UNPLAYABLE', audioWithUrl: 0 },
            { client: 'WEB', status: 'UNPLAYABLE', audioWithUrl: 0 },
        ];
        const ordered = ['MWEB', 'ANDROID', 'IOS', 'TV', 'ANDROID_VR', 'WEB'];
        const truncated = 'Truncated download: only 99s of 287s of audio is actually present (>66% missing).';

        const makeCtx = (tried) => {
            const calls = [];
            const obj = {
                ...downloadMethods,
                _pendingDownloadContexts: new Map(),
                extractErrorMessage: (p) => p.error || '',
                showToast: () => {},
                getDownloadOptions: () => ({}),
                downloadResolvedTrack: async (r, f, o) => { calls.push({ client: r.client, opts: o }); return { id: 'next' }; },
            };
            obj._autoRetryBudget = new Map([['BElct8HWkp8', { attempts: tried.length ? 1 : 0, triedClients: tried.slice(), triedClasses: ['adaptive', 'opus', 'muxed'] }]]);
            obj._pendingDownloadContexts.set('dl1', {
                key: 'BElct8HWkp8',
                resolved: { client: tried.length ? 'ANDROID_VR' : 'IOS', orderedClients: ordered, retryClients: tried.length ? ['WEB'] : ['TV', 'ANDROID_VR', 'WEB'], client_report: report },
                opts: {}, originalUrl: 'https://youtu.be/BElct8HWkp8', format: 'm4a', _retrying: false,
            });
            return { obj, calls };
        };

        // Attempt 1: IOS truncated -> rotate to the only other client that can
        // actually serve audio (ANDROID_VR), skipping UNPLAYABLE TV.
        let { obj, calls } = makeCtx([]);
        global.window = global.window || {};
        global.window.AuralisYouTube = { resolve: async (_u, o) => ({ kind: 'track', stream_url: 'https://x/', client: o.forceClient, client_report: report }) };
        await obj._handle403AutoRetry({ id: 'dl1', status: 'failed', error: truncated });
        assert.equal(calls.length, 1, 'exactly one retry expected');
        assert.equal(calls[0].client, 'ANDROID_VR', 'must retry with a client that handed out audio urls');

        // Attempt 2: ANDROID_VR truncated too, and only WEB is left. WEB's record
        // is UNPLAYABLE with no formats at all, which establishes nothing about
        // the client beyond this attempt — so it is a legitimate last resort, and
        // the budget is what stops the chain, not the client list.
        ({ obj, calls } = makeCtx(['IOS', 'ANDROID_VR']));
        await obj._handle403AutoRetry({ id: 'dl1', status: 'failed', error: truncated });
        assert.equal(calls.length, 1, 'a merely-empty client stays eligible');
        assert.equal(calls[0].client, 'WEB', 'the only client left is asked before giving up');

        // ...and the SABR-only client is never asked, even when it is the single
        // remaining candidate. That is the half of the original assertion that
        // was correct: ANDROID described 25 adaptive formats and served a url on
        // none of them, answering with SABR metadata instead. That is a fact
        // about the client, not about the attempt.
        const sabrOnly = report.map((e) => (e.client === 'WEB' ? { ...e, status: 'OK', audioWithUrl: 0, adaptive: 18, adaptiveWithUrl: 0, progressiveWithUrl: 0, sabrStreamingUrl: true } : e));
        const sabrObj = {
            ...downloadMethods,
            _pendingDownloadContexts: new Map(),
            extractErrorMessage: (p) => p.error || '',
            showToast: () => {},
            getDownloadOptions: () => ({}),
            downloadResolvedTrack: async () => { throw new Error('must not be called'); },
        };
        sabrObj._autoRetryBudget = new Map([['BElct8HWkp8', { attempts: 1, triedClients: ['IOS', 'ANDROID_VR'], triedClasses: ['adaptive', 'opus', 'muxed'] }]]);
        sabrObj._pendingDownloadContexts.set('dl1', {
            key: 'BElct8HWkp8',
            resolved: { client: 'ANDROID_VR', orderedClients: ordered, retryClients: ['ANDROID'], client_report: sabrOnly },
            opts: {}, originalUrl: 'https://youtu.be/BElct8HWkp8', format: 'm4a', _retrying: false,
        });
        await sabrObj._handle403AutoRetry({ id: 'dl1', status: 'failed', error: truncated });
        assert.equal(sabrObj._autoRetryBudget.get('BElct8HWkp8').attempts, 1,
            'a proven-dead-end candidate must not spend an attempt');
    });

    // ── "resolved empty" is not a property of the client ────────────────────
    //
    // The measurement that forced this split, device report 2026-09-27, track
    // hsXKOsnptw4: the SAME client answered twice, minutes apart, on the SAME
    // video —
    //
    //   resolve #1   ANDROID  a0/p1   (adaptive 0, progressive 1)  audio-with-url 0
    //   resolve #2   ANDROID  a30/p1  (adaptive 30, progressive 1) audio-with-url 30
    //
    // `a` = adaptive, `p` = progressive, exactly as the report prints them. So a
    // zero is not a property of the client. The old predicate called it one, and
    // because rotation is a search over candidates, one zero was enough to burn
    // ANDROID for the whole track: on the next attempt every better client was
    // already in `tried`, ANDROID was the only one left, the predicate refused
    // it, and the retry reported a dead end with a usable client sitting there.
    const HX_VID = 'hsXKOsnptw4';
    // Ordered the way the resolver emits it with a token present: SERVABLE,
    // then TOKEN_FREE, then UNMINTABLE. Asserted only as a fixture here — the
    // ordering itself is owned by youtube.js and pinned there.
    const HX_ORDERED = ['MWEB', 'ANDROID_VR', 'TV', 'IOS', 'ANDROID'];
    const err403 = 'HTTP 403 Forbidden [rr1---sn-gwpa-civey.googlevideo.com] body: (empty body), start_byte=0';
    const errTrunc = 'Truncated download: only 75s of 216s of audio is actually present (>66% missing).';

    // One retry attempt on a shared per-track budget, returning the options the
    // resolver was actually asked with. Driving the real method rather than a
    // copy of its predicate is deliberate: a test that re-implements the rule it
    // is checking has already shipped one defect in this file (the pot-for-TV
    // case, AGENTS.md v2.6.50).
    const attemptOnce = async ({ key, winner, report, ordered, tried, attempts, sel, error, optsIn = {}, triedClasses }) => {
        const seen = [];
        const obj = {
            ...downloadMethods,
            _pendingDownloadContexts: new Map(),
            extractErrorMessage: (p) => p.error || '',
            showToast: () => {},
            getDownloadOptions: () => ({}),
            downloadResolvedTrack: async (r, f, o) => { seen.push({ client: r.client, opts: o }); return { id: 'next' }; },
        };
        // Default to every class already tried, because that is the state in which
        // the ladder falls through to client rotation — and rotation is what most
        // of these assertions are about. A case that is asserting a CLASS rung has
        // to pass the classes that are still untried, or the rung is unreachable by
        // construction and the assertion is vacuous.
        obj._autoRetryBudget = new Map([[key, {
            attempts,
            triedClients: tried.slice(),
            triedClasses: triedClasses || ['adaptive', 'opus', 'muxed'],
        }]]);
        obj._pendingDownloadContexts.set('dl1', {
            key,
            resolved: { client: winner, orderedClients: ordered, client_report: report, selection: sel || { itag: 18, ext: 'mp4', audioOnly: false, legacyProgressive: true } },
            opts: optsIn, originalUrl: `https://youtu.be/${key}`, format: 'm4a', _retrying: false,
        });
        global.window = global.window || {};
        global.window.AuralisYouTube = {
            resolve: async (_u, o) => ({ kind: 'track', stream_url: 'https://x/', client: o.forceClient, client_report: report }),
        };
        await obj._handle403AutoRetry({ id: 'dl1', status: 'failed', error });
        return { seen, budget: obj._autoRetryBudget.get(key) };
    };

    it('a client that merely resolved empty stays eligible on a later attempt', async () => {
        // Resolve #1 as the device recorded it: ANDROID answered a0/p1.
        const emptyRead = [
            { client: 'MWEB', status: 'UNPLAYABLE', audioWithUrl: 0, progressiveWithUrl: 0 },
            { client: 'ANDROID_VR', status: 'OK', audioWithUrl: 4, adaptiveWithUrl: 21, progressiveWithUrl: 0 },
            { client: 'TV', status: 'UNPLAYABLE', audioWithUrl: 0, progressiveWithUrl: 0 },
            { client: 'IOS', status: 'OK', audioWithUrl: 2, adaptiveWithUrl: 20, progressiveWithUrl: 0 },
            // a0/p1 — described one progressive format and served a url on none.
            // No SABR endpoint, so it established nothing about the client.
            { client: 'ANDROID', status: 'OK', adaptive: 0, progressive: 1, adaptiveWithUrl: 0, audioWithUrl: 0, progressiveWithUrl: 0, sabrStreamingUrl: false },
        ];

        // Every client that could be preferred is already used, so ANDROID is the
        // only candidate left. It came back empty, which is not a reason to spend
        // an attempt on nothing and give up — it is a reason to try LAST, and
        // there is nothing better left to try.
        let { seen } = await attemptOnce({
            key: HX_VID, winner: 'ANDROID_VR', report: emptyRead, ordered: HX_ORDERED,
            tried: ['ANDROID_VR', 'MWEB', 'TV', 'IOS'], attempts: 2, error: err403,
        });
        assert.equal(seen.length, 1, 'an empty reading must not be read as a dead end');
        assert.equal(seen[0].client, 'ANDROID', 'the only unused client must be asked');

        // ...and it must not be *excluded* from the resolve either. Deferring a
        // client is a preference; if it also reached excludeClients the
        // de-prioritisation would have hardened back into the same permanent
        // veto, one layer down.
        assert.ok(!(seen[0].opts.excludeClients || []).includes('ANDROID'),
            'a deferred client must not be handed to the resolver as excluded');

        // Resolve #2, minutes later, same client, same video: a30/p1, thirty
        // adaptive formats all carrying urls. The track's budget is shared, and
        // this resolve sees the full 30 — so a client is never written off on
        // the strength of one reading.
        const fullRead = [
            { client: 'MWEB', status: 'UNPLAYABLE', audioWithUrl: 0, progressiveWithUrl: 0 },
            { client: 'ANDROID_VR', status: 'OK', audioWithUrl: 4, adaptiveWithUrl: 21, progressiveWithUrl: 0 },
            { client: 'TV', status: 'UNPLAYABLE', audioWithUrl: 0, progressiveWithUrl: 0 },
            { client: 'IOS', status: 'OK', audioWithUrl: 2, adaptiveWithUrl: 20, progressiveWithUrl: 0 },
            { client: 'ANDROID', status: 'OK', adaptive: 30, progressive: 1, adaptiveWithUrl: 30, audioWithUrl: 30, progressiveWithUrl: 0, sabrStreamingUrl: true },
        ];
        ({ seen } = await attemptOnce({
            key: HX_VID, winner: 'ANDROID_VR', report: fullRead, ordered: HX_ORDERED,
            tried: ['ANDROID_VR', 'MWEB', 'TV', 'IOS'], attempts: 2, error: err403,
        }));
        assert.equal(seen.length, 1);
        assert.equal(seen[0].client, 'ANDROID', 'the client that now has 30 audio urls is the one to ask');

        // The preference is not gone, only the veto. Two candidates must be left
        // for this to mean anything — one servable, one deferred — or there is
        // nothing to prefer between and the assertion would pass either way.
        // The report is the a0/p1 one, so ANDROID is the deferred candidate and
        // IOS is the servable one, on the same attempt.
        ({ seen } = await attemptOnce({
            key: HX_VID, winner: 'ANDROID_VR', report: emptyRead, ordered: HX_ORDERED,
            tried: ['ANDROID_VR', 'MWEB', 'TV'], attempts: 2, error: err403,
        }));
        assert.equal(seen.length, 1);
        assert.equal(seen[0].client, 'IOS',
            'a client that handed out urls must still be preferred over an empty-reading one');
    });

    it('a client proven unable to serve a class is still excluded', async () => {
        // The real report from the device, 2026-09-27, track yF9nmg_jHNs, which
        // exhausted four attempts:
        const report = [
            { client: 'MWEB', status: 'UNPLAYABLE', audioWithUrl: 0, progressiveWithUrl: 0 },
            { client: 'WEB', status: 'UNPLAYABLE', audioWithUrl: 0, progressiveWithUrl: 0 },
            { client: 'TV', status: 'UNPLAYABLE', audioWithUrl: 0, progressiveWithUrl: 0 },
            // IOS: real urls for adaptive audio, and a positive record that it has
            // NONE for the other two classes — progressiveWithUrl 0, opusWithUrl 0.
            { client: 'IOS', status: 'OK', audioWithUrl: 2, adaptiveWithUrl: 24, progressiveWithUrl: 0, audioOnlyWithUrl: 2, opusWithUrl: 0, sabrStreamingUrl: true },
            { client: 'ANDROID_VR', status: 'OK', audioWithUrl: 4, adaptiveWithUrl: 26, progressiveWithUrl: 1, audioOnlyWithUrl: 4, opusWithUrl: 2, sabrStreamingUrl: true },
            { client: 'ANDROID', status: 'OK', audioWithUrl: 0, adaptive: 25, adaptiveWithUrl: 0, progressiveWithUrl: 0, sabrStreamingUrl: true },
        ];
        const ordered = ['MWEB', 'WEB', 'ANDROID_VR', 'TV', 'IOS', 'ANDROID'];

        // (i) The muxed rung is gated on the winner actually HAVING a muxed url.
        // A 403 on IOS's adaptive audio must not spend an attempt re-resolving
        // the class IOS's own record proves it does not offer.
        let { seen } = await attemptOnce({
            key: 'yF9nmg_jHNs', winner: 'IOS', report, ordered, tried: ['IOS'], attempts: 1, error: err403,
            sel: { client: 'IOS', itag: 140, ext: 'm4a', audioOnly: true, legacyProgressive: false },
        });
        assert.equal(seen.length, 1);
        assert.equal(seen[0].opts.forceLegacyProgressive, false,
            'IOS reports progressiveWithUrl=0, so the muxed rung must not fire for it. Asserted as an explicit false rather than undefined: the caller now sets every force mode on every attempt, because leaving one unset let a previous rung\'s flag survive — measured 2026-09-29, opus requested three times on yF9nmg_jHNs while the ladder had moved to muxed');
        assert.equal(seen[0].opts.forceOpusAudio, false,
            'and the opus rung must be explicitly off too, for the same reason');
        assert.equal(seen[0].client, 'ANDROID_VR', 'rotate to a client that has other classes');

        // (ii) The opus rung picks on what each client actually offers. IOS
        // reports opusWithUrl=0, so it must never be the one asked for opus.
        // `triedClasses` is passed explicitly: with every class already tried the
        // ladder goes to rotation and the opus rung is unreachable, which would
        // make this assertion vacuous rather than wrong.
        ({ seen } = await attemptOnce({
            key: 'yF9nmg_jHNs', winner: 'ANDROID_VR', report, ordered,
            tried: ['ANDROID_VR'], attempts: 2, error: errTrunc,
            triedClasses: ['adaptive', 'muxed'],
            sel: { client: 'ANDROID_VR', itag: 18, ext: 'mp4', audioOnly: false, legacyProgressive: true },
        }));
        assert.equal(seen.length, 1);
        assert.equal(seen[0].opts.forceOpusAudio, true, 'both other classes have failed, so the opus rung is the right one');
        assert.equal(seen[0].client, 'ANDROID_VR', 'ANDROID_VR is the only client with opusWithUrl>0, so it is the one asked');
        assert.notEqual(seen[0].client, 'IOS', 'IOS reports opusWithUrl=0 and must not be re-asked for opus');

        // (iii) The counter branch of "proven dead": described formats, no url on
        // any of them, and no SABR endpoint either. That is a positive record
        // about the client, so it is never rotated into even as a last resort.
        // ANDROID above is exactly this shape (adaptive 25, nothing served).
        // ANDROID is the only candidate left, so this is the case where a veto
        // actually costs a download. Its record is the strongest negative the
        // resolver can emit: the adapter returned a 25-entry adaptive table and
        // not one url on any of it.
        const withAndroidLast = [
            { client: 'MWEB', status: 'UNPLAYABLE', audioWithUrl: 0, progressiveWithUrl: 0 },
            { client: 'ANDROID_VR', status: 'UNPLAYABLE', audioWithUrl: 0, progressiveWithUrl: 0 },
            { client: 'TV', status: 'UNPLAYABLE', audioWithUrl: 0, progressiveWithUrl: 0 },
            { client: 'IOS', status: 'UNPLAYABLE', audioWithUrl: 0, progressiveWithUrl: 0 },
            { client: 'ANDROID', status: 'OK', audioWithUrl: 0, adaptive: 25, adaptiveWithUrl: 0, progressiveWithUrl: 0, sabrStreamingUrl: false },
        ];
        // Ordered to match the fixture exactly. An unlisted client would be a
        // candidate with "no evidence either way", which is eligible by design —
        // so a stray name here would test the wrong branch.
        const androidLastOrdered = ['MWEB', 'ANDROID_VR', 'TV', 'IOS', 'ANDROID'];
        const budget = { attempts: 1, triedClients: ['MWEB', 'ANDROID_VR', 'TV', 'IOS'], triedClasses: ['adaptive', 'opus', 'muxed'] };
        const calls = [];
        const obj = {
            ...downloadMethods,
            _pendingDownloadContexts: new Map(),
            extractErrorMessage: (p) => p.error || '',
            showToast: () => {},
            getDownloadOptions: () => ({}),
            downloadResolvedTrack: async () => { throw new Error('must not be asked'); },
        };
        obj._autoRetryBudget = new Map([['proven', budget]]);
        obj._pendingDownloadContexts.set('dl1', {
            key: 'proven',
            resolved: { client: 'IOS', orderedClients: androidLastOrdered, client_report: withAndroidLast, selection: { itag: 18, ext: 'mp4', audioOnly: false, legacyProgressive: true } },
            opts: {}, originalUrl: 'https://youtu.be/proven', format: 'm4a', _retrying: false,
        });
        global.window = global.window || {};
        global.window.AuralisYouTube = { resolve: async (_u, o) => { calls.push(o); return { kind: 'track', stream_url: 'https://x/', client: o.forceClient }; } };
        await obj._handle403AutoRetry({ id: 'dl1', status: 'failed', error: errTrunc });
        assert.equal(calls.length, 0, 'a client that described formats and served none must never be asked');
        assert.equal(budget.attempts, 1, 'and it must not spend an attempt');
    });

    it('the retry still terminates, and never excludes every client', async () => {
        // The hard guarantee this change had to keep. Emptiness became a
        // preference, so clients are now re-askable — which means the client list
        // can no longer be what stops the chain. `MAX_AUTO_RETRIES` has to be.
        //
        // Worst case for that: one client with urls and every other client
        // coming back empty, forever. The empty ones are re-asked, `tried` grows
        // by one per failure, and the budget has to be the thing that ends it.
        const report = [
            { client: 'MWEB', status: 'UNPLAYABLE', audioWithUrl: 0, progressiveWithUrl: 0 },
            { client: 'ANDROID_VR', status: 'UNPLAYABLE', audioWithUrl: 0, progressiveWithUrl: 0 },
            { client: 'TV', status: 'UNPLAYABLE', audioWithUrl: 0, progressiveWithUrl: 0 },
            { client: 'IOS', status: 'OK', audioWithUrl: 2, adaptiveWithUrl: 20, progressiveWithUrl: 0 },
            { client: 'ANDROID', status: 'UNPLAYABLE', audioWithUrl: 0, progressiveWithUrl: 0 },
        ];
        const ordered = ['MWEB', 'ANDROID_VR', 'TV', 'IOS', 'ANDROID'];
        const budget = { attempts: 0, triedClients: [], triedClasses: ['adaptive', 'opus', 'muxed'] };
        const chosen = [];
        let winner = 'IOS';

        // Six failures, six chances to keep going.
        for (let i = 0; i < 6; i++) {
            const seen = [];
            const obj = {
                ...downloadMethods,
                _pendingDownloadContexts: new Map(),
                extractErrorMessage: (p) => p.error || '',
                showToast: () => {},
                getDownloadOptions: () => ({}),
                downloadResolvedTrack: async (r, f, o) => { seen.push({ client: r.client, opts: o }); return { id: 'next' }; },
            };
            obj._autoRetryBudget = new Map([['loop', budget]]);
            obj._pendingDownloadContexts.set('dl' + i, {
                key: 'loop',
                resolved: { client: winner, orderedClients: ordered, client_report: report, selection: { itag: 18, ext: 'mp4', audioOnly: false, legacyProgressive: true } },
                opts: {}, originalUrl: 'https://youtu.be/loop', format: 'm4a', _retrying: false,
            });
            global.window = global.window || {};
            global.window.AuralisYouTube = { resolve: async (_u, o) => ({ kind: 'track', stream_url: 'https://x/', client: o.forceClient, client_report: report }) };
            // Snapshot before the call: the handler pushes the failed winner onto
            // `tried` itself, so reading it afterwards would compare against a
            // set that has since grown.
            const triedBefore = budget.triedClients.slice();
            await obj._handle403AutoRetry({ id: 'dl' + i, status: 'failed', error: err403 });
            if (!seen.length) break;
            winner = seen[0].client;
            chosen.push({ ...seen[0], triedBefore });
        }

        assert.equal(chosen.length, 3, 'exactly MAX_AUTO_RETRIES attempts, then the chain ends');
        // Rotation begins AFTER the client that just failed, so with IOS winning
        // the order is ANDROID, MWEB, ANDROID_VR — and each appears once, which is
        // what shows `tried` still stops an immediate repeat of the same client.
        assert.deepEqual(chosen.map((c) => c.client), ['ANDROID', 'MWEB', 'ANDROID_VR'],
            'each re-askable client is used once, in rotation order — `tried` still prevents a repeat');
        // Never exclude every client: the resolver must always be left with the
        // client being asked, so `excludeClients` can neither contain it nor
        // cover the whole list.
        for (const c of chosen) {
            const ex = c.opts.excludeClients || [];
            assert.ok(!ex.includes(c.client), 'the client being asked must not also be excluded');
            assert.ok(ex.length < ordered.length, 'excluding every client leaves the resolver nothing to try');
            // Excluding only the ones already asked is the point: anything else
            // must stay available, or a rotation that is merely a reorder would
            // quietly become "pick one and shut the rest out".
            assert.ok(c.triedBefore.every((cl) => ex.includes(cl)),
                'every client already asked must be excluded from the next resolve');
        }

        // The boundary case the guard exists for: all but one client already
        // tried. That last one must still be asked — a resolver asked to rotate
        // with everything excluded has nothing left to do but fail.
        const { seen } = await attemptOnce({
            key: 'lastone', winner: 'MWEB', report, ordered,
            tried: ['MWEB', 'ANDROID_VR', 'TV', 'ANDROID'], attempts: 2, error: err403,
        });
        assert.equal(seen.length, 1, 'the one remaining client must still be used');
        assert.equal(seen[0].client, 'IOS', 'and it must be the one asked');
        assert.equal((seen[0].opts.excludeClients || []).length, 4, 'the other four are excluded');
        assert.ok(!(seen[0].opts.excludeClients || []).includes('IOS'), 'which must not include the client being asked');

        // The saturated case: the only state where the guard can actually differ.
        // Every client is already in `tried`, so handing the resolver that whole
        // list as `excludeClients` would leave it an empty pool — and, worse,
        // would name the very client being asked, contradicting forceClient.
        //
        // It is reachable only through the muxed/opus rungs, because plain
        // rotation returns early when no candidate is left. That is also why the
        // plain loop above cannot pin the guard: `tried` never reaches the full
        // list there, so the loop passes whether or not the guard exists. A
        // mutation deleting the guard went undetected until this case existed.
        const opusReport = [
            { client: 'MWEB', status: 'UNPLAYABLE', audioWithUrl: 0, progressiveWithUrl: 0, opusWithUrl: 0 },
            { client: 'ANDROID_VR', status: 'OK', audioWithUrl: 4, adaptiveWithUrl: 26, progressiveWithUrl: 1, opusWithUrl: 2 },
            { client: 'TV', status: 'UNPLAYABLE', audioWithUrl: 0, progressiveWithUrl: 0, opusWithUrl: 0 },
            { client: 'IOS', status: 'OK', audioWithUrl: 2, adaptiveWithUrl: 24, progressiveWithUrl: 0, opusWithUrl: 0 },
            { client: 'ANDROID', status: 'OK', audioWithUrl: 0, adaptive: 25, adaptiveWithUrl: 0, progressiveWithUrl: 0, opusWithUrl: 0, sabrStreamingUrl: true },
        ];
        const opusOrdered = ['MWEB', 'ANDROID_VR', 'TV', 'IOS', 'ANDROID'];
        const saturated = await attemptOnce({
            key: 'saturated', winner: 'ANDROID_VR', report: opusReport, ordered: opusOrdered,
            tried: ['MWEB', 'ANDROID_VR', 'TV', 'IOS', 'ANDROID'], attempts: 2, error: errTrunc,
            // The saturated CLIENT set is the subject here; the class ladder must
            // still have opus left, or the rung this case exists to exercise is
            // unreachable and the assertion passes for the wrong reason.
            triedClasses: ['adaptive', 'muxed'],
            sel: { client: 'ANDROID_VR', itag: 18, ext: 'mp4', audioOnly: false, legacyProgressive: true },
        });
        assert.equal(saturated.seen.length, 1, 'a saturated tried-set must not stop the opus rescue');
        assert.equal(saturated.seen[0].client, 'ANDROID_VR', 'the only client offering opus is still asked');
        assert.deepEqual(saturated.seen[0].opts.excludeClients || [],
            [],
            'every client is in `tried`, so the exclusion must be dropped rather than handed over empty-handed');
    });

    it('a client serving audio through the muxed progressive url counts as servable', async () => {
        // The real per-client report from the device, 2026-09-26, track
        // hsXKOsnptw4 — the download that failed:
        //
        //   IOS        OK  adaptiveWithUrl=21 audioWithUrl=4   -> 403 at byte 0
        //   ANDROID_VR OK  adaptiveWithUrl=21 audioWithUrl=4   -> 403 at byte 0
        //   ANDROID    OK  adaptiveWithUrl=0  audioWithUrl=0 progressive=1
        //
        // and the report from the track that SUCCEEDED, Ral6kFSx7ZY:
        //
        //   ANDROID    CHOSEN OK adaptiveWithUrl=0 audioWithUrl=0 progressive=1
        //               -> itag=18 muxed progressive -> downloaded
        //
        // So ANDROID served audio, and only through the muxed progressive url.
        // The old predicate asked `audioWithUrl > 0`, which counts adaptive audio
        // only, so ANDROID scored 0 and was classified a dead end — locking out
        // the only client that has ever completed a download on this network.
        // The retry then rotated IOS -> ANDROID_VR, which 403'd identically and
        // consumed the final attempt. This test pins the corrected predicate.
        const report = [
            { client: 'IOS', status: 'OK', audioWithUrl: 4, adaptiveWithUrl: 21, progressiveWithUrl: 0 },
            { client: 'ANDROID_VR', status: 'OK', audioWithUrl: 4, adaptiveWithUrl: 21, progressiveWithUrl: 1 },
            { client: 'ANDROID', status: 'OK', audioWithUrl: 0, adaptiveWithUrl: 0, progressiveWithUrl: 1, sabrStreamingUrl: true },
            { client: 'TV', status: 'UNPLAYABLE', audioWithUrl: 0, progressiveWithUrl: 0 },
            { client: 'MWEB', status: 'UNPLAYABLE', audioWithUrl: 0, progressiveWithUrl: 0 },
            { client: 'WEB', status: 'UNPLAYABLE', audioWithUrl: 0, progressiveWithUrl: 0 },
        ];
        // Ordered as the device report shows it, with a token present:
        // SERVABLE, then TOKEN_FREE, then UNMINTABLE.
        const ordered = ['MWEB', 'WEB', 'ANDROID_VR', 'TV', 'IOS', 'ANDROID'];
        const err403 = 'HTTP 403 Forbidden [rr1---sn-gwpa-civey.googlevideo.com] body: (empty body), start_byte=0';

        const calls = [];
        const obj = {
            ...downloadMethods,
            _pendingDownloadContexts: new Map(),
            extractErrorMessage: (p) => p.error || '',
            showToast: () => {},
            getDownloadOptions: () => ({}),
            downloadResolvedTrack: async (r, f, o) => { calls.push({ client: r.client }); return { id: 'next' }; },
        };
        obj._autoRetryBudget = new Map([['hsXKOsnptw4', { attempts: 1, triedClients: ['IOS'], triedClasses: ['adaptive', 'opus', 'muxed'] }]]);
        obj._pendingDownloadContexts.set('dl1', {
            key: 'hsXKOsnptw4',
            // IOS won, then 403'd, so it is the client being rotated away from.
            resolved: { client: 'IOS', orderedClients: ordered, client_report: report },
            opts: {}, originalUrl: 'https://youtu.be/hsXKOsnptw4', format: 'm4a', _retrying: false,
        });
        global.window = global.window || {};
        global.window.AuralisYouTube = {
            resolve: async (_u, o) => ({ kind: 'track', stream_url: 'https://x/', client: o.forceClient, client_report: report }),
        };

        await obj._handle403AutoRetry({ id: 'dl1', status: 'failed', error: err403 });
        assert.equal(calls.length, 1, 'exactly one retry expected');
        assert.equal(calls[0].client, 'ANDROID',
            'the progressive-only client is the one that has actually downloaded a file; ' +
            'it must be chosen over another adaptive client that already 403\'d');

        // And the report itself must carry the measurement the predicate reads.
        // Without progressiveWithUrl in the recorded entry the filter is guessing,
        // so pin the field at its source rather than only at the consumer.
        const uiDir = path.resolve(import.meta.dirname ?? path.dirname(new URL(import.meta.url).pathname), '../../ui/js');
        const ysrc = fs.readFileSync(path.join(uiDir, 'youtube.js'), 'utf8');
        assert.ok(/progressiveWithUrl = \(sd\?\.formats \|\| \[\]\)\.filter\(urlOf\)\.length/.test(ysrc),
            'the actions.execute report must record whether progressive formats carry urls');
        assert.ok(/entry\.progressiveWithUrl = \(sd\.formats \|\| \[\]\)\.filter\(urlOf\)\.length/.test(ysrc),
            'the getInfo report must record the same, or the two paths disagree');
    });

    it('a byte-0 403 on an adaptive url retries the SAME client with its muxed format first', async () => {
        // From the device report of 2026-09-26, track hsXKOsnptw4, attempt #2:
        //   IOS CHOSEN itag=140 ext=m4a audioOnly=yes -> 403 at byte 0
        // The successful download on the same network came through the muxed
        // itag 18, so the cheapest retry is the other format class from the
        // client that already resolves — not a different client. Rotating first
        // is what burned the remaining budget on ANDROID_VR.
        const report = [
            { client: 'IOS', status: 'OK', audioWithUrl: 4, adaptiveWithUrl: 21, progressiveWithUrl: 1 },
            { client: 'ANDROID_VR', status: 'OK', audioWithUrl: 4, adaptiveWithUrl: 21, progressiveWithUrl: 1 },
            { client: 'ANDROID', status: 'OK', audioWithUrl: 0, adaptiveWithUrl: 0, progressiveWithUrl: 1, sabrStreamingUrl: true },
            { client: 'MWEB', status: 'UNPLAYABLE', audioWithUrl: 0, progressiveWithUrl: 0 },
        ];
        const ordered = ['MWEB', 'WEB', 'ANDROID_VR', 'TV', 'IOS', 'ANDROID'];
        const err403 = 'HTTP 403 Forbidden [rr1---sn-gwpa-civey.googlevideo.com] body: (empty body), start_byte=0';

        const run = async (sel) => {
            const calls = [];
            const obj = {
                ...downloadMethods,
                _pendingDownloadContexts: new Map(),
                extractErrorMessage: (p) => p.error || '',
                showToast: () => {},
                getDownloadOptions: () => ({}),
                downloadResolvedTrack: async (r, f, o) => { calls.push({ client: r.client, forceLegacy: !!o.forceLegacyProgressive }); return { id: 'next' }; },
            };
            obj._autoRetryBudget = new Map([['hsXKOsnptw4', { attempts: 1, triedClients: ['IOS'], triedClasses: ['adaptive'] }]]);
            obj._pendingDownloadContexts.set('dl1', {
                key: 'hsXKOsnptw4',
                resolved: { client: 'IOS', orderedClients: ordered, client_report: report, selection: sel },
                opts: {}, originalUrl: 'https://youtu.be/hsXKOsnptw4', format: 'm4a', _retrying: false,
            });
            global.window = global.window || {};
            global.window.AuralisYouTube = {
                resolve: async (_u, o) => ({ kind: 'track', stream_url: 'https://x/', client: o.forceClient, client_report: report }),
            };
            await obj._handle403AutoRetry({ id: 'dl1', status: 'failed', error: err403 });
            return calls;
        };

        // The refused url was adaptive audio-only and IOS does have a progressive
        // url -> re-ask IOS for the muxed format before rotating anywhere.
        let calls = await run({ itag: 140, ext: 'm4a', audioOnly: true, legacyProgressive: false });
        assert.equal(calls.length, 1, 'exactly one retry expected');
        assert.equal(calls[0].client, 'IOS', 'must re-ask the failing client, not rotate away from it');
        assert.equal(calls[0].forceLegacy, true, 'the retry must ask for the muxed format');

        // IOS reported progressiveWithUrl=0 on every real attempt, so it has
        // nothing to fall back to and must not be re-asked for the class. The
        // muxed class is still untried, though, so the correct move is to take it
        // from a client that HAS one — not to abandon the class and rotate.
        // The previous assertion here demanded the rotation, which is the
        // behaviour that burned the budget re-asking classes already refused.
        const noMuxed = report.map((e) => (e.client === 'IOS' ? { ...e, progressiveWithUrl: 0 } : e));
        const orig = JSON.stringify(report);
        report.length = 0; report.push(...noMuxed);
        calls = await run({ itag: 140, ext: 'm4a', audioOnly: true, legacyProgressive: false });
        assert.equal(calls.length, 1, 'a 403 must still be retried');
        assert.notEqual(calls[0].client, 'IOS', 'a client with no progressive url must not be re-asked for it');
        assert.equal(calls[0].forceLegacy, true, 'the untried class must be taken from a client that has it, not dropped');
        report.length = 0; report.push(...JSON.parse(orig));

        // And when NO client has the class, the ladder must not request it —
        // asking a client for a format it has no url for spends an attempt
        // re-resolving the same refused url.
        const noMuxedAnywhere = report.map((e) => ({ ...e, progressiveWithUrl: 0 }));
        report.length = 0; report.push(...noMuxedAnywhere);
        calls = await run({ itag: 140, ext: 'm4a', audioOnly: true, legacyProgressive: false });
        assert.equal(calls[0] && calls[0].forceLegacy, false, 'no client has a muxed url, so none may be requested');
        report.length = 0; report.push(...JSON.parse(orig));

        // Already on the muxed path: there is nothing to fall back to, so the
        // rescue must not fire and cost an attempt.
        calls = await run({ itag: 18, ext: 'mp4', audioOnly: false, legacyProgressive: true });
        assert.equal(calls[0] && calls[0].forceLegacy, false, 'must not ask for a fallback from a fallback');

        // The resolver must honour the option rather than quietly ignoring it.
        const ysrc = fs.readFileSync(path.join(
            path.resolve(import.meta.dirname ?? path.dirname(new URL(import.meta.url).pathname), '../../ui/js'),
            'youtube.js'), 'utf8');
        assert.ok(/opts\.forceLegacyProgressive \|\| opts\.force_legacy_progressive/.test(ysrc),
            'youtube.js must honour forceLegacyProgressive');
        assert.ok(/forceLegacyProgressive UNAVAILABLE/.test(ysrc),
            'an unrescuable client must say so, since IOS has progressive=0');
    });

    it('opus is reported as an available audio-only class, and the ranking is not asserted to exclude it', () => {
        // The user was right and the comment in youtube.js was wrong. Opus IS
        // decodable by this project — `player.rs` routes WebM/Opus to our own
        // `OpusSource` as `DecodedAudioSource::Opus`, bypassing rodio entirely
        // (`rusty-opus` on 64-bit, `opus_decoder` on 32-bit). The old comment
        // read rodio's missing opus feature and concluded the project could not
        // play opus, which never followed.
        const root = path.resolve(import.meta.dirname ?? path.dirname(new URL(import.meta.url).pathname), '../..');
        const opusRs = fs.readFileSync(path.join(root, 'src/infrastructure/media/opus.rs'), 'utf8');
        const playerRs = fs.readFileSync(path.join(root, 'src/infrastructure/media/player.rs'), 'utf8');
        assert.ok(/rusty_opus::|opus_decoder::/.test(opusRs), 'a real opus decoder must be wired, not just declared');
        assert.ok(/DecodedAudioSource::Opus/.test(playerRs),
            'the player must route opus through our own decoder rather than rodio');

        // ...and the consequence must be measured, not guessed. Whether the CDN
        // serves an opus url is a question about the edge, so the report has to
        // carry the number. A decoder that works locally is not evidence that a
        // googlevideo url will be served -- the same error as trusting a
        // decoder's opinion about a file's completeness.
        const ysrc = fs.readFileSync(path.join(root, 'ui/js/youtube.js'), 'utf8');
        assert.ok(/entry\.audioOnlyWithUrl = /.test(ysrc), 'the report must count audio-only urls per client');
        // Both resolve paths separately. A single loose assertion is satisfied by
        // EITHER one, so deleting the other still passes — which is how a
        // measurement ends up present on the path nobody uses.
        assert.ok(/entry\.opusWithUrl = audioOnlyUrls\.filter\(\(f\) => \/opus\/i\.test\(String\(f\.mimeType/.test(ysrc),
            'the actions.execute report must count opus urls');
        assert.ok(/entry\.opusWithUrl = audioOnlyUrls\.filter\(\(f\) => \/opus\/i\.test\(String\(f\.mime_type/.test(ysrc),
            'the getInfo report must count opus urls, or the two paths disagree');
        const dsrc = fs.readFileSync(path.join(root, 'ui/js/modules/downloads.js'), 'utf8');
        assert.ok(/opusWithUrl=\$\{/.test(dsrc), 'opusWithUrl must be visible in the device report');
        // The user-facing 403 hint used to tell people to mint a PO token. On the
        // one network we have data for that remedy cannot work — a Web token is
        // platform-bound and invalid on exactly the clients (ios/android_vr) that
        // hand out audio urls — and the capture that showed it also showed the
        // thing that actually fixed the download: rotating to a client with a muxed
        // url. Advice that names a dead remedy sends people down a false trail
        // during the one test that can prove it wrong.
        assert.ok(!/set youtube_po_token via BgUtil mint/.test(dsrc),
            'the 403 hint must not recommend minting a token as the remedy');
        // The stale claim must not come back in a comment.
        assert.ok(!/do NOT prefer webm\/opus/.test(ysrc),
            'the refuted claim must not remain as a comment — it is what caused the misranking');
    });

    it('an unreadable SDK_INT must fall back to the publish path that can work', () => {
        // android_downloads.rs: sdk_int() returned 26 on any failure to read
        // Build.VERSION.SDK_INT, which routes to publish_legacy. That path writes
        // through Environment + WRITE_EXTERNAL_STORAGE, and CI injects
        // WRITE_EXTERNAL_STORAGE with android:maxSdkVersion="29" — so on any
        // API 30+ device the fallback selected the one branch that cannot
        // succeed, and it failed silently because the only report is a warn! to a
        // logcat that release builds do not emit.
        //
        // This is a defect whether or not it is the current cause, so it is
        // pinned independently of the diagnosis. The constant is cfg(android) and
        // cannot be reached from a host test, hence the source assertion.
        const root = path.resolve(import.meta.dirname ?? path.dirname(new URL(import.meta.url).pathname), '../..');
        const asrc = fs.readFileSync(path.join(root, 'src/infrastructure/media/android_downloads.rs'), 'utf8');
        assert.ok(/const SDK_INT_FALLBACK: i32 = 29;/.test(asrc),
            'the assumed API level must be 29+, not 26 — 26 routes to a path that needs a permission capped at maxSdk 29');
        assert.ok(!/unwrap_or\(26\)/.test(asrc) && !/\n\s*26\n\s*\}/.test(asrc),
            'no remaining literal 26 fallback in sdk_int');

        // And the permission cap that makes this matter must stay where it is:
        // raising the fallback above 29 is only safe while the legacy path is
        // still permission-capped, so a silent change to maxSdkVersion would
        // invalidate the reasoning above.
        const ci = fs.readFileSync(path.join(root, '.github/workflows/build.yml'), 'utf8');
        assert.ok(/WRITE_EXTERNAL_STORAGE" android:maxSdkVersion="29"/.test(ci),
            'WRITE_EXTERNAL_STORAGE must remain capped at maxSdkVersion 29 for the fallback reasoning to hold');
    });

    it('a completed download must say where the file went, and say so when it is unfindable', () => {
        // Verified on the device from the filesystem, 2026-09-27: a download
        // reported `completed` while /storage/emulated/0/Download/Auralis/ was
        // empty and no audio file existed anywhere on the sdcard. The directory's
        // own mtime was 13 days stale, so nothing had been placed in it at all.
        //
        // Nothing reported that. The row rendered "completed • host • 100%" and
        // stopped — there was no path on screen, so a publish that failed and a
        // publish that worked were indistinguishable to the user. `output_path`
        // is already on the event (overwritten with the public location when
        // MediaStore publishing succeeds, left as the app-private path when it
        // does not), so the fix is to classify and render it rather than to
        // invent new plumbing.
        const repoRoot = path.resolve(import.meta.dirname ?? path.dirname(new URL(import.meta.url).pathname), '../..');
        const dsrc = fs.readFileSync(path.join(repoRoot, 'ui/js/modules/downloads.js'), 'utf8');
        assert.ok(/progress\.output_path/.test(dsrc), 'the row must read output_path from the event');
        assert.ok(/not visible in Files/.test(dsrc),
            'a completed download that is not in public storage must say so — that is the whole defect');
        // The public test: the two branches must be distinguished by the path
        // itself, not by a setting the user has to know to look for.
        assert.ok(/startsWith\('\/storage\/emulated\/'\)/.test(dsrc) && /startsWith\('content:\/\/'\)/.test(dsrc),
            'public storage must be recognised from the emitted path');
        assert.ok(/saved to /.test(dsrc) && /saved in app storage only/.test(dsrc),
            'both outcomes need distinct wording, or the row still cannot be read');
        // ...and the REASON, not just the fact. publish_to_downloads used to
        // collapse every distinct failure into one None, so a context failure and
        // a missing row id produced the same visible result and the same log
        // line. Four unrelated causes, one indistinguishable symptom.
        assert.ok(/progress\.publish_error/.test(dsrc),
            'the publish failure reason must reach the row, or the next run cannot diagnose itself');
        const dlsrc = fs.readFileSync(path.join(repoRoot, 'src/infrastructure/media/downloader.rs'), 'utf8');
        assert.ok(/note_publish_error\(/.test(dlsrc),
            'the downloader must record the reason instead of logging and dropping it');
        const dlmodel = fs.readFileSync(path.join(repoRoot, 'src/domain/models/download.rs'), 'utf8');
        assert.ok(/pub publish_error: Option<String>/.test(dlmodel) && /pub fn note_publish_error/.test(dlmodel),
            'DownloadProgress must carry the reason on its own field, separate from the download error');
    });

    it('the retry ladder is adaptive -> same-client muxed -> audio-only opus, and each rung is gated on the report', async () => {
        // The real per-client report from the device, 2026-09-27, track
        // yF9nmg_jHNs, which exhausted four attempts and still failed:
        //
        //   #1 ANDROID_VR itag=140 m4a audio-only -> 403 at byte 0
        //   #2 ANDROID_VR itag=18  mp4 MUXED      -> truncated 75s of 216s
        //   #3 ANDROID   itag=18  mp4 MUXED      -> truncated
        //   #4 IOS       itag=140 m4a audio-only -> 403 at byte 0
        //
        // and it recorded `ANDROID_VR ... opusWithUrl=2` — two audio-only opus
        // streams with usable urls that nothing ever requested, because
        // scoreFormat ranks itag 140 above opus. The owner had asked why opus
        // was deprioritised when we can decode it; this is the measurement that
        // finally lets us ask for it rather than guess.
        const report = [
            { client: 'MWEB', status: 'UNPLAYABLE', audioWithUrl: 0, progressiveWithUrl: 0, opusWithUrl: 0 },
            { client: 'WEB', status: 'UNPLAYABLE', audioWithUrl: 0, progressiveWithUrl: 0, opusWithUrl: 0 },
            { client: 'TV', status: 'UNPLAYABLE', audioWithUrl: 0, progressiveWithUrl: 0, opusWithUrl: 0 },
            { client: 'IOS', status: 'OK', audioWithUrl: 2, adaptiveWithUrl: 24, progressiveWithUrl: 0, audioOnlyWithUrl: 2, opusWithUrl: 0, sabrStreamingUrl: true },
            { client: 'ANDROID_VR', status: 'OK', audioWithUrl: 4, adaptiveWithUrl: 26, progressiveWithUrl: 1, audioOnlyWithUrl: 4, opusWithUrl: 2, sabrStreamingUrl: true },
            { client: 'ANDROID', status: 'OK', audioWithUrl: 0, adaptiveWithUrl: 0, progressiveWithUrl: 1, audioOnlyWithUrl: 0, opusWithUrl: 0, sabrStreamingUrl: true },
        ];
        const ordered = ['MWEB', 'WEB', 'ANDROID_VR', 'TV', 'IOS', 'ANDROID'];
        const err403 = 'HTTP 403 Forbidden [rr2---sn-ci5gup-cvhe7.googlevideo.com] body: (empty body), start_byte=0';
        const errTrunc = 'Truncated download: only 75s of 216s of audio is actually present (>66% missing).';

        // Walk the ladder the way the device did, feeding each rung the failure
        // the previous one produced, and assert the escalation rather than any
        // single step — the ordering is the whole point.
        const run = async (sel, errRaw, budget) => {
            const calls = [];
            const obj = {
                ...downloadMethods,
                _pendingDownloadContexts: new Map(),
                extractErrorMessage: (p) => p.error || '',
                showToast: () => {},
                getDownloadOptions: () => ({}),
                downloadResolvedTrack: async (r, f, o) => {
                    calls.push({
                        client: r.client,
                        muxed: !!o.forceLegacyProgressive,
                        opus: !!o.forceOpusAudio,
                    });
                    return { id: 'next' };
                },
            };
            obj._autoRetryBudget = new Map([['yF9nmg_jHNs', budget]]);
            obj._pendingDownloadContexts.set('dl1', {
                key: 'yF9nmg_jHNs',
                resolved: { client: sel.client, orderedClients: ordered, client_report: report, selection: sel },
                opts: {}, originalUrl: 'https://youtu.be/yF9nmg_jHNs', format: 'm4a', _retrying: false,
            });
            global.window = global.window || {};
            global.window.AuralisYouTube = {
                resolve: async (_u, o) => ({ kind: 'track', stream_url: 'https://x/', client: o.forceClient, client_report: report }),
            };
            await obj._handle403AutoRetry({ id: 'dl1', status: 'failed', error: errRaw });
            return calls;
        };

        const adaptive = { client: 'ANDROID_VR', itag: 140, ext: 'm4a', audioOnly: true, legacyProgressive: false };
        const muxed = { client: 'ANDROID_VR', itag: 18, ext: 'mp4', audioOnly: false, legacyProgressive: true };
        const opus = { client: 'ANDROID_VR', itag: 251, ext: 'webm', mime: 'audio/webm; codecs="opus"', audioOnly: true, legacyProgressive: false };

        // Rung 2 is OPUS, not the muxed fallback. This test previously asserted
        // the opposite, and the device run of 2026-09-27 is why that was wrong:
        // the muxed class has never succeeded (403 once, truncated twice) while
        // opus was never tried at all, so spending rung 2 on muxed guaranteed the
        // untried class would not be reached. See the class-ladder test below for
        // the full four-attempt replay.
        let calls = await run(adaptive, err403, { attempts: 0, triedClients: [], triedClasses: [] });
        assert.equal(calls.length, 1);
        assert.equal(calls[0].client, 'ANDROID_VR', 'ANDROID_VR is the only client offering opus, so it must be the one asked');
        assert.equal(calls[0].opus, true, 'the untried class must be rung 2');
        assert.equal(calls[0].muxed, false, 'muxed has never succeeded and must not outrank an untried class');

        // Rung 3: opus also refused, so the muxed class is the last one left.
        calls = await run({ ...opus, client: 'ANDROID_VR' }, err403, { attempts: 1, triedClients: ['ANDROID_VR'], triedClasses: ['adaptive', 'opus'] });
        assert.equal(calls.length, 1);
        assert.equal(calls[0].muxed, true, 'with adaptive and opus refused, muxed is what remains');
        assert.equal(calls[0].opus, false, 'must not re-ask for the class that just failed');

        // Gated: with no client offering opus the rung must not fire, or the
        // retry spends an attempt re-resolving a class already proven bad.
        // This is the real IOS case — progressiveWithUrl:0 and opusWithUrl:0.
        const noOpus = report.map((e) => ({ ...e, opusWithUrl: 0 }));
        const saved = report.splice(0, report.length, ...noOpus);
        calls = await run(adaptive, err403, { attempts: 0, triedClients: [], triedClasses: ['adaptive'] });
        assert.equal(calls.every((c) => !c.opus), true, 'a client set with no opus must not trigger the opus rung');
        assert.equal(calls[0].muxed, true, 'with no opus available the ladder must go to the muxed class instead');
        report.splice(0, report.length, ...saved);

        // ...and the resolver must honour the option, and say so when it cannot.
        const repoRoot = path.resolve(import.meta.dirname ?? path.dirname(new URL(import.meta.url).pathname), '../..');
        const ysrc = fs.readFileSync(path.join(repoRoot, 'ui/js/youtube.js'), 'utf8');
        assert.ok(/opts\.forceOpusAudio \|\| opts\.force_opus_audio/.test(ysrc), 'youtube.js must honour forceOpusAudio');
        assert.ok(/forceOpusAudio UNAVAILABLE/.test(ysrc), 'an unavailable opus rung must say so, since IOS reports opusWithUrl=0');
        // Audio-only and highest bitrate: the point of asking for opus is
        // quality, and it is also what avoids the 360p remux the muxed rung pays.
        assert.ok(/isAudioFormat\(f\)/.test(ysrc) && /!f\.has_video/.test(ysrc),
            'the opus selector must require an audio-only format');
        assert.ok(/\(b\.bitrate \|\| 0\) - \(a\.bitrate \|\| 0\)/.test(ysrc),
            'the opus selector must take the highest bitrate, not an arbitrary member');
        // The force modes are exclusive. Before this, `youtube.js` applied its
        // force blocks in sequence and each assigned `fmt`, so a stale
        // `forceOpusAudio` silently overwrote a muxed request — measured
        // 2026-09-29 on yF9nmg_jHNs, where opus was returned three times while
        // the ladder believed it had moved to muxed.
        //
        // This is a SOURCE-SHAPE assertion and says so: the resolver needs
        // InnerTube and the vendored bgutils, so its behaviour cannot be driven
        // here. What it buys is that deleting the guard fails the suite, which
        // is the difference between a guard and a comment. A mutation run
        // confirmed that removing it does NOT otherwise fail anything — the
        // reason this assertion exists.
        assert.ok(/if \(fmt && used_legacy_progressive\) \{/.test(ysrc),
            'the opus force block must decline to overwrite an honoured muxed request');
        assert.ok(/forceLegacyProgressive and forceOpusAudio are both set/.test(ysrc),
            'and the conflict must be reported rather than resolved silently');
    });

    it('the class ladder reaches audio-only opus instead of spending retries on classes already refused', async () => {
        // Replay of the real device run, 2026-09-27, track yF9nmg_jHNs, on the
        // build that shipped the opus rung. Four attempts, all failed, and opus
        // was never requested:
        //
        //   #1 ANDROID_VR itag=140 adaptive -> 403 @ byte 0
        //   #2 ANDROID_VR itag=18  MUXED    -> 403
        //   #3 IOS        itag=140 adaptive -> 403 @ byte 0
        //   #4 ANDROID    itag=18  MUXED    -> truncated 75s of 216s
        //
        // while the same report recorded `ANDROID_VR ... opusWithUrl=2`. The old
        // gate only reached for opus on an observed *truncation*, and the only
        // truncation landed on the last attempt the budget allows — so the one
        // untried class was structurally unreachable, and two of three retries
        // went to classes already proven bad.
        const report = [
            { client: 'MWEB', status: 'UNPLAYABLE', adaptiveWithUrl: 0, audioWithUrl: 0, progressiveWithUrl: 0, opusWithUrl: 0 },
            { client: 'WEB', status: 'UNPLAYABLE', adaptiveWithUrl: 0, audioWithUrl: 0, progressiveWithUrl: 0, opusWithUrl: 0 },
            // TV stopped being a hard zero on this run: `adaptive=28 progressive=1
            // progressiveWithUrl=1`, still no audio-only urls. Recorded because it
            // is the same "resolved empty is not a stable property" shape the
            // rotationRank split exists for.
            { client: 'TV', status: 'OK', adaptiveWithUrl: 0, audioWithUrl: 0, progressiveWithUrl: 1, opusWithUrl: 0, sabrStreamingUrl: true },
            { client: 'IOS', status: 'OK', adaptiveWithUrl: 24, audioWithUrl: 2, progressiveWithUrl: 0, opusWithUrl: 0, sabrStreamingUrl: true },
            { client: 'ANDROID_VR', status: 'OK', adaptiveWithUrl: 26, audioWithUrl: 4, progressiveWithUrl: 1, opusWithUrl: 2, sabrStreamingUrl: true },
            { client: 'ANDROID', status: 'OK', adaptiveWithUrl: 0, audioWithUrl: 0, progressiveWithUrl: 1, opusWithUrl: 0, sabrStreamingUrl: true },
        ];
        const ordered = ['MWEB', 'WEB', 'ANDROID_VR', 'TV', 'IOS', 'ANDROID'];
        const err403 = 'HTTP 403 Forbidden [rr1---sn-gwpa-cive7.googlevideo.com] body: (empty body), start_byte=0';
        const errTrunc = 'Truncated download: only 75s of 216s of audio is actually present (>66% missing).';

        // The adaptive class, as `selection` actually reports it.
        const adaptive = { itag: 140, ext: 'm4a', mime: 'audio/mp4; codecs="mp4a.40.2"', audioOnly: true, hasVideo: false, legacyProgressive: false };
        const muxed = { itag: 18, ext: 'mp4', mime: 'video/mp4; codecs="avc1.42001E, mp4a.40.2"', audioOnly: false, hasVideo: true, legacyProgressive: true };
        const opus = { itag: 251, ext: 'webm', mime: 'audio/webm; codecs="opus"', audioOnly: true, hasVideo: false, legacyProgressive: false };

        // One shared budget across a whole track's retry sequence, which is how
        // the real ladder is driven — `triedClients`/`triedClasses` accumulate.
        const budget = { attempts: 0, triedClients: ['ANDROID_VR'], triedClasses: [] };
        const calls = [];
        const obj = {
            ...downloadMethods,
            _pendingDownloadContexts: new Map(),
            extractErrorMessage: (p) => p.error || '',
            showToast: () => {},
            getDownloadOptions: () => ({}),
            downloadResolvedTrack: async (r, f, o) => {
                calls.push({
                    client: r.client,
                    muxed: !!o.forceLegacyProgressive,
                    opus: !!o.forceOpusAudio,
                });
                return { id: 'next' };
            },
        };
        obj._autoRetryBudget = new Map([['yF9nmg_jHNs', budget]]);
        global.window = global.window || {};
        global.window.AuralisYouTube = {
            resolve: async (_u, o) => ({
                kind: 'track', stream_url: 'https://x/', client: o.forceClient,
                client_report: report,
                selection: o.forceOpusAudio ? opus : (o.forceLegacyProgressive ? muxed : adaptive),
            }),
        };
        const feed = async (sel, errRaw) => {
            obj._pendingDownloadContexts.set('dl1', {
                key: 'yF9nmg_jHNs',
                resolved: { client: sel === muxed ? 'ANDROID' : 'ANDROID_VR', orderedClients: ordered, client_report: report, selection: sel },
                opts: {}, originalUrl: 'https://youtu.be/yF9nmg_jHNs', format: 'm4a', _retrying: false,
            });
            await obj._handle403AutoRetry({ id: 'dl1', status: 'failed', error: errRaw });
        };

        // Attempt #1 failed: adaptive refused at byte 0. The very next rung must
        // be the class nobody has tried, not the muxed fallback that the old
        // error-type gate jumped to.
        await feed(adaptive, err403);
        assert.equal(calls.length, 1, 'one retry per failure');
        assert.equal(calls[0].opus, true, 'the untried class (opus) must be rung 2, not the muxed fallback');
        assert.equal(calls[0].muxed, false, 'muxed has never succeeded and must not be tried before opus');
        assert.equal(calls[0].client, 'ANDROID_VR', 'ANDROID_VR is the only client reporting opusWithUrl > 0');

        // Attempt #2 (opus) also refused. The only class left is muxed, and it
        // must still be reached rather than burning the budget on a rotation.
        await feed(opus, err403);
        assert.equal(calls.length, 2, 'a second retry must happen');
        assert.equal(calls[1].muxed, true, 'with adaptive and opus both refused, muxed is the last class');
        assert.equal(calls[1].opus, false, 'must not re-ask the class that just failed');
        assert.deepEqual(budget.triedClasses, ['adaptive', 'opus'], 'each class is recorded once, in the order tried');

        // Attempt #3 (muxed) truncated. Every class has now been tried, so what
        // remains is a client re-ask — a second opinion, not progress, and the one
        // thing the old mid-ladder rotation was doing too early.
        await feed(muxed, errTrunc);
        assert.equal(calls.length, 3, 'one rotation is still allowed once the classes are exhausted');
        assert.equal(calls[2].opus, false, 'the rotation must not force a class');
        assert.equal(calls[2].muxed, false, 'the rotation must not re-force the class that just truncated');
        assert.deepEqual(budget.triedClasses, ['adaptive', 'opus', 'muxed'],
            'each class recorded once, in the order tried, and no class is re-added');
        // ...and the budget still bounds it: MAX_AUTO_RETRIES is 3, so the fourth
        // failure must stop rather than spin on a class known to truncate.
        await feed(muxed, errTrunc);
        assert.equal(calls.length, 3, 'the attempt budget must still terminate the ladder');
    });

    it('downloader tops a windowed (SABR) partial download up with explicit ranges', () => {
        const base = path.resolve(import.meta.dirname ?? path.dirname(new URL(import.meta.url).pathname), '../../src/infrastructure/media');
        const dsrc = fs.readFileSync(path.join(base, 'downloader.rs'), 'utf8');
        const rsrc = fs.readFileSync(path.join(base, 'range_topup.rs'), 'utf8');
        // The decoded-length gate is the only place that can see this failure
        // (all advertised bytes arrive, the media is still short), so the top-up
        // must hang off it rather than off the byte accounting gate.
        assert.ok(dsrc.includes('range_topup::top_up'), 'downloader must be able to request the bytes after a short file');
        // The top-up must be reachable ONLY from the not-accepted path. The
        // previous form of this assertion was a 4000-character proximity window
        // between `verify_decoded_duration` and `range_topup::top_up`, which is
        // exactly the kind of assertion that encodes an accident of file layout
        // rather than a requirement: when NEW-03 moved the verdict into
        // `gather_forensics` (a different function) the window broke with nothing
        // behavioural having changed, and it read as a real failure.
        //
        // What actually has to hold is structural, and is stable under refactor:
        // within `run_stream`, the accept-and-keep branch must be decided BEFORE
        // the top-up call, and the top-up must sit in the `else` of that decision.
        const rsStart = dsrc.indexOf('    async fn run_stream(');
        assert.ok(rsStart > 0, 'run_stream must exist');
        const runStream = dsrc.slice(rsStart, dsrc.indexOf('\n    async fn ', rsStart + 1) > 0
            ? dsrc.indexOf('\n    async fn ', rsStart + 1)
            : dsrc.length);
        const acceptAt = runStream.indexOf('== Acceptance::Accept');
        // Anchor on the call, not the bare path: a comment above the top-up
        // mentions `range_topup::top_up` by name, and matching that would
        // resolve to the explanation rather than to the call site.
        const topUpAt = runStream.indexOf('super::range_topup::top_up(');
        assert.ok(acceptAt > 0 && topUpAt > 0, 'run_stream must decide acceptance and be able to top up');
        assert.ok(acceptAt < topUpAt,
            'the top-up must be reached only from the not-accepted path, so acceptance is decided first');
        // ...and structurally, not merely by ordering: the top-up lives in the
        // `else` arm of that decision. Without the `else` the two would be
        // sequential and the ordering assertion above would pass while a
        // recovered file still fell through into the top-up.
        const between = runStream.slice(acceptAt, topUpAt);
        assert.ok(/\}\s*else\s*\{/.test(between),
            'the top-up must be gated by an else arm on the acceptance decision');
        // The decoder is not trusted: the container and a full decode decide.
        assert.ok(dsrc.includes('inspect_container') && dsrc.includes('inspect_content'),
            'a short verdict must be checked against the container and the decoded content');
        assert.ok(dsrc.includes('Verdict::Complete'),
            'the container verdict must gate acceptance');
        assert.ok(fs.existsSync(path.join(base, 'forensics.rs')),
            'the container forensics module must exist');
        assert.ok(dsrc.includes('itag='), 'the truncation error must report the itag/host/clen for diagnosis');
        // The top-up must try the mechanisms a googlevideo edge may honour, and
        // must name the status codes it got so a refusal is explainable in-app.
        assert.ok(rsrc.includes('TOPUP_CHUNK_BYTES') && rsrc.includes('MAX_TOPUP_ROUNDS'), 'top-up sizes must be named constants');
        assert.ok(rsrc.includes('with_range_param'), 'top-up must use the googlevideo range parameter');
        assert.ok(rsrc.includes('url+header') && rsrc.includes('"header"') && rsrc.includes('"url"'), 'all three range request shapes must be tried');
        assert.ok(rsrc.includes('RANGE'), 'the HTTP Range header must be used as well as the query param');
    });

    it('youtube.js flags the SABR-only legacy progressive fallback as partial-prone', () => {
        const src = fs.readFileSync(ytPath, 'utf8');
        assert.ok(src.includes('used_legacy_progressive'), 'must track legacy-progressive fallback usage');
        assert.ok(src.includes('sabrFallback: used_legacy_progressive'), 'resolved object must expose sabrFallback so retries can escalate');
        assert.ok(
            src.includes('allow_legacy_progressive') && src.includes('avoidLegacyProgressive'),
            'opts.avoidLegacyProgressive must be able to reject the legacy-progressive fallback'
        );
    });

    it('the asset protocol is enabled and scoped for local cover art', () => {
        // Every library card renders artwork with convertFileSrc(), which
        // produces an asset:// url. Tauri v2 only serves those when
        // app.security.assetProtocol is enabled *and* the path is in scope -
        // the CSP already allowed `asset:`, so without this every cover was a
        // broken image with the reason buried in an unreadable console.
        const conf = JSON.parse(fs.readFileSync(path.resolve(
            import.meta.dirname ?? path.dirname(new URL(import.meta.url).pathname),
            '../../tauri.conf.json'), 'utf8'));
        const security = conf.app.security || {};
        assert.ok(security.assetProtocol, 'tauri.conf.json must configure app.security.assetProtocol');
        assert.equal(security.assetProtocol.enable, true, 'the asset protocol must be enabled');
        const scope = security.assetProtocol.scope || [];
        assert.ok(scope.length > 0, 'the asset protocol needs a scope');
        assert.ok(
            scope.some((entry) => entry.includes('APPDATA') || entry.includes('APPLOCALDATA')),
            `the app data dir (where downloads and their .jpg sidecars live) must be in scope: ${JSON.stringify(scope)}`
        );
        assert.ok(
            security.csp.includes('asset:'),
            "the CSP must keep allowing asset: in img-src or cover art cannot load at all"
        );
        const uiSrc = fs.readFileSync(path.resolve(
            import.meta.dirname ?? path.dirname(new URL(import.meta.url).pathname),
            '../../ui/js/modules/ui.js'), 'utf8');
        assert.ok(uiSrc.includes('media_data_url'), 'cover art must fall back to media_data_url');
        assert.ok(uiSrc.includes('_artworkFailed'),
            'a failed cover must be recorded and replaced by a placeholder, not left broken');
    });

    it('ui/js/player.js coordinates smoothly with streaming audio element', () => {
        const src = fs.readFileSync(playerJsPath, 'utf8');
        assert.ok(src.includes('window._auralisStreamAudio'), 'player.js must check window._auralisStreamAudio');
        assert.ok(src.includes('window._auralisStreamAudio.play()') || src.includes('await window._auralisStreamAudio.play()'), 'play must resume streaming audio');
        assert.ok(src.includes('window._auralisStreamAudio.pause()'), 'pause must pause streaming audio');
    });
});

describe('HTMX #content navigation race (Download Audio bounced back to Home)', () => {
    // htmx's request queue is keyed per *owning element*, so the initial
    // `GET /partials/home.html` (owned by <main id="content">) and a nav click
    // (owned by the <a>/<button>) are not serialized against each other: both
    // run in parallel and whichever response lands last wins, so a stale home
    // response can clobber the freshly swapped download page. `hx-sync` puts
    // every writer of #content on one shared key with the `replace` strategy,
    // so a new navigation aborts whatever is already in flight. Nothing else
    // (no transition:true, no hx-boost) may reintroduce the race.
    const uiDir = path.resolve(import.meta.dirname ?? path.dirname(new URL(import.meta.url).pathname), '../../ui');

    // Blank out comments while preserving newlines so line numbers stay exact.
    function stripComments(html) {
        return html.replace(/<!--[\s\S]*?-->/g, (c) => c.replace(/[^\n]/g, ' '));
    }

    // Parse per opening tag, honouring quoted attribute values so a `>` inside
    // style="..." can never split a tag in half. Doctype/comments/closing tags
    // do not match, so each hit is exactly one element.
    function extractTags(html) {
        const src = stripComments(html);
        const tagRe = /<([a-zA-Z][^\s/>]*)((?:"[^"]*"|'[^']*'|[^>"'])*)>/g;
        const tags = [];
        let m;
        while ((m = tagRe.exec(src)) !== null) {
            tags.push({
                name: m[1].toLowerCase(),
                attrs: m[2] || '',
                line: src.slice(0, m.index).split('\n').length,
            });
        }
        return tags;
    }

    function collectHtml(dir) {
        const out = [];
        for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
            const full = path.join(dir, entry.name);
            if (entry.isDirectory()) {
                if (entry.name === 'vendor') continue; // third-party assets
                out.push(...collectHtml(full));
            } else if (entry.name.endsWith('.html')) {
                out.push(full);
            }
        }
        return out;
    }

    const htmlFiles = collectHtml(uiDir);
    const TARGET_RE = /hx-target\s*=\s*["']\s*#content\s*["']/;
    const SYNC_RE = /hx-sync\s*=\s*["']\s*#content\s*:\s*replace\s*["']/;

    it('scans the expected navigation markup (guards against a vacuous test)', () => {
        for (const rel of ['index.html', 'partials/nav.html', 'partials/home.html']) {
            const full = path.join(uiDir, rel);
            assert.ok(fs.existsSync(full), `expected ${rel} to exist`);
            const hits = extractTags(fs.readFileSync(full, 'utf8')).filter((t) => TARGET_RE.test(t.attrs));
            assert.ok(hits.length > 0, `expected ${rel} to contain at least one hx-target="#content" element`);
        }
    });

    it('every element targeting #content also carries hx-sync="#content:replace"', () => {
        const problems = [];
        let checked = 0;

        for (const file of htmlFiles) {
            const rel = path.relative(uiDir, file);
            for (const tag of extractTags(fs.readFileSync(file, 'utf8'))) {
                if (!TARGET_RE.test(tag.attrs)) continue;
                checked += 1;
                if (!SYNC_RE.test(tag.attrs)) {
                    problems.push(
                        `${rel}:${tag.line} <${tag.name}> has hx-target="#content" but no ` +
                        `hx-sync="#content:replace" — without a shared sync key its request runs ` +
                        `in parallel with the #content initial load and a late response overwrites it`
                    );
                }
            }
        }

        assert.ok(checked > 0, 'the scan found no hx-target="#content" elements at all — the parser is broken');
        assert.equal(
            problems.length,
            0,
            `every element that swaps #content must declare hx-sync="#content:replace":\n  - ${problems.join('\n  - ')}`
        );
    });

    it('the initial #content load is itself serialized against later navigation', () => {
        // <main id="content"> has no hx-target (it swaps itself), so it needs the
        // attribute even though the generic check above cannot see it.
        const rel = 'index.html';
        const tags = extractTags(fs.readFileSync(path.join(uiDir, rel), 'utf8'));
        const main = tags.find((t) => /id\s*=\s*["']\s*content\s*["']/.test(t.attrs));
        assert.ok(main, `${rel} must still contain <main id="content">`);
        assert.ok(
            /hx-get\s*=\s*["']\s*\/partials\/home\.html\s*["']/.test(main.attrs),
            `${rel}:${main.line} the initial load must still fetch /partials/home.html`
        );
        assert.ok(
            SYNC_RE.test(main.attrs),
            `${rel}:${main.line} <main id="content"> owns the initial request and must declare ` +
            `hx-sync="#content:replace", otherwise a nav click cannot abort it and the stale home ` +
            `response overwrites the page the user just opened`
        );
    });

    it('navigation does not reintroduce the race via view transitions or boosting', () => {
        const problems = [];
        for (const file of htmlFiles) {
            const rel = path.relative(uiDir, file);
            for (const tag of extractTags(fs.readFileSync(file, 'utf8'))) {
                if (!TARGET_RE.test(tag.attrs)) continue;
                if (/(?:^|\s)transition\s*:\s*true/.test(tag.attrs)) {
                    problems.push(`${rel}:${tag.line} <${tag.name}> sets transition:true`);
                }
                if (/(?:^|\s)hx-boost/.test(tag.attrs)) {
                    problems.push(`${rel}:${tag.line} <${tag.name}> uses hx-boost`);
                }
            }
        }
        assert.equal(
            problems.length,
            0,
            `transition:true / hx-boost caused superimposed views previously — keep them off #content:\n  - ${problems.join('\n  - ')}`
        );
    });
});



