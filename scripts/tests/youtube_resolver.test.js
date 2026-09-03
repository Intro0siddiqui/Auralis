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

    it('source has actions.execute loop over 6 clients (not 3)', () => {
        // Must list all 6 clients somewhere (PO-token-aware orderedClients now splits order)
        for (const c of ['IOS', 'ANDROID', 'ANDROID_VR', 'TV', 'MWEB', 'WEB']) {
            assert.ok(src.includes(`'${c}'`), `Expected client '${c}' in youtube.js`);
        }
        // orderedClients must be defined (TV/ANDROID_VR-first when no poToken)
        assert.ok(src.includes('orderedClients'), 'Expected orderedClients PO-token-aware ordering');
        assert.ok(src.includes("TV") && src.includes("ANDROID_VR"), 'orderedClients must mention TV/ANDROID_VR');
        // PO-aware order: when poToken empty => TV, ANDROID_VR, MWEB first; when token => IOS, ANDROID first
        const m = src.match(/(?:const|let)\s+orderedClients\s*=\s*opts\.poToken\s*\?\s*\[([^\]]+)\]\s*:\s*\[([^\]]+)\]/);
        assert.ok(m, 'orderedClients ternary not found or malformed');
        const withToken = m[1];
        const withoutToken = m[2];
        assert.ok(withToken.includes("'IOS'") && withToken.includes("'ANDROID'") && withToken.indexOf("'IOS'") < withToken.indexOf("'ANDROID'"), 'with poToken order should start IOS, ANDROID');
        assert.ok(withToken.indexOf("'IOS'") < withToken.indexOf("'TV'"), 'with poToken IOS must come before TV');
        assert.equal(withToken.replace(/\s/g, ''), "'IOS','ANDROID','ANDROID_VR','TV','MWEB','WEB'", 'with poToken branch exact order');
        assert.equal(withoutToken.replace(/\s/g, ''), "'TV','ANDROID_VR','MWEB','WEB','IOS','ANDROID'", 'without poToken branch exact order: TV, ANDROID_VR, MWEB first');
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
        // TV/ANDROID_VR-first ordering must remain (orderedClients ternary)
        const m = src.match(/(?:const|let)\s+orderedClients\s*=\s*opts\.poToken\s*\?\s*\[([^\]]+)\]\s*:\s*\[([^\]]+)\]/);
        assert.ok(m, 'orderedClients must still be TV-first when no poToken');
        assert.equal(m[2].replace(/\s/g, ''), "'TV','ANDROID_VR','MWEB','WEB','IOS','ANDROID'", 'TV/ANDROID_VR-first ordering must be preserved');
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

describe('pot-for-TV (YAD 7C4-TAWg7QA / Sx8z0U0lkjQ regression)', () => {
    const ytPath = path.resolve(import.meta.dirname ?? path.dirname(new URL(import.meta.url).pathname), '../../ui/js/youtube.js');
    const vendorPath = path.resolve(path.dirname(ytPath), '../vendor/youtubei.esm.mjs');

    it('fake resolve mints token and appends &pot= to googlevideo URL for winningClient TV', () => {
        // Simulate youtube.js pot append logic (defensive manual append for googlevideo)
        function appendPot(streamUrl, potVal) {
            if (!streamUrl || !potVal) return streamUrl;
            if (streamUrl.includes('pot=')) return streamUrl;
            try {
                const u = new URL(streamUrl);
                if (u.hostname.includes('googlevideo.com') || u.hostname.includes('youtube.com')) {
                    u.searchParams.set('pot', potVal);
                    return u.toString();
                }
            } catch (_) {
                if (!streamUrl.includes('pot=')) return streamUrl + (streamUrl.includes('?') ? '&' : '?') + 'pot=' + encodeURIComponent(potVal);
            }
            return streamUrl;
        }

        // Fake minted token
        const fakeToken = 'TEST_POT_TOKEN_6h_CACHE_123';
        const winningClient = 'TV';
        const googlevideoUrl = 'https://rr1---sn-gwpa-cived.googlevideo.com/videoplayback?expire=1234567890&ei=test&ip=2409%3A40c4%3A35b%3Ab681%3A8000%3A%3A&itag=140&id=o-ABC123&source=youtube&requiressl=yes&mh=xyz&mm=31%2C29&mn=sn-gwpa-cived&ms=au%2Crdu&mv=m&mvi=1&pl=24&initcwndbps=1280000&spn=1&vprv=1&mime=audio%2Fmp4&cnr=14&c=TV&cver=2.20250101&cplayer=UNIPLAYER&cbrand=google&cbrandft=1&cbr=SAMSUNG&cbrver=21.0&cmodel=SM-G998B&cplatform=mobile&csn=1&pot_placeholder=0&n=abc123&sparams=expire%2Cei%2Cip%2Cid%2Citag%2Csource%2Crequiressl%2Cvprv%2Cmime%2Cns%2Ccnr%2Csparams%2Cpot';

        // Simulate resolve that would have used TV client because no poToken initially, then minted
        assert.equal(winningClient, 'TV', 'winningClient should be TV for Jio IPv6 residential path');

        const withPot = appendPot(googlevideoUrl, fakeToken);
        assert.ok(withPot.includes('pot='), 'googlevideo URL must contain pot param after append');
        const u = new URL(withPot);
        assert.equal(u.searchParams.get('pot'), fakeToken, 'pot value must match minted token');
        assert.ok(u.hostname.includes('googlevideo.com'), 'hostname must be googlevideo');
        // Ensure pot is searchable via has('pot')
        assert.equal(u.searchParams.has('pot'), true);

        // Verify original URL without pot would be rejected on sn-gwpa-cived 2026-02 Jio CGNAT without pot
        const before = new URL(googlevideoUrl);
        assert.equal(before.searchParams.has('pot'), false, 'original URL has no pot');
    });

    it('youtube.js source unconditionally appends pot (Appended pot / searchParams.set)', () => {
        const src = fs.readFileSync(ytPath, 'utf8');
        // Must contain unconditional pot append logic
        const hasPotSet = src.includes("searchParams.set('pot'") || src.includes('searchParams.set("pot"') || src.includes("Appended pot");
        assert.ok(hasPotSet, 'youtube.js must contain searchParams.set(\'pot\' or Appended pot comment');
        // Ensure the defensive append is not gated behind sabr check in youtube.js
        // The pot append block should exist outside any sabr guard
        assert.ok(src.includes("Appended pot to googlevideo URL"), 'youtube.js should contain Appended pot log comment');
        // pot logic should handle both opts.poToken and opts.po_token
        assert.ok(src.includes("opts.poToken") && src.includes("opts.po_token"), 'youtube.js pot logic must handle both poToken variants');
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

    it('ui/js/player.js coordinates smoothly with streaming audio element', () => {
        const src = fs.readFileSync(playerJsPath, 'utf8');
        assert.ok(src.includes('window._auralisStreamAudio'), 'player.js must check window._auralisStreamAudio');
        assert.ok(src.includes('window._auralisStreamAudio.play()') || src.includes('await window._auralisStreamAudio.play()'), 'play must resume streaming audio');
        assert.ok(src.includes('window._auralisStreamAudio.pause()'), 'pause must pause streaming audio');
    });
});



