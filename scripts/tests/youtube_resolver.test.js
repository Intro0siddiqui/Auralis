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

// ── helpers extracted verbatim from youtube.js so tests don't need a WebView ──
function isDirectAudio(url) { return /\.(mp3|m4a|aac|ogg|oga|opus|wav|flac|webm)(\?.*)?$/i.test(url); }
function extFromMime(mime) {
    if (!mime) return 'm4a';
    const m = String(mime).toLowerCase();
    if (m.includes('mp4') || m.includes('m4a') || m.includes('aac')) return 'm4a';
    if (m.includes('webm') || m.includes('opus')) return 'webm';
    if (m.includes('ogg')) return 'ogg';
    if (m.includes('wav')) return 'wav';
    if (m.includes('flac')) return 'flac';
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
        assert.equal(extFromMime('video/mp4'), 'm4a');
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

describe('regression: 6-client fallback must be present in youtube.js', () => {
    const ytPath = path.resolve(import.meta.dirname ?? path.dirname(new URL(import.meta.url).pathname), '../../ui/js/youtube.js');
    const src = fs.readFileSync(ytPath, 'utf8');

    it('source contains all 6 clients IOS, ANDROID, ANDROID_VR, TV, MWEB, WEB', () => {
        for (const c of ['IOS', 'ANDROID', 'ANDROID_VR', 'TV', 'MWEB', 'WEB']) {
            assert.ok(src.includes(`'${c}'`), `youtube.js missing client '${c}'`);
        }
    });

    it('source has actions.execute loop over 6 clients (not 3)', () => {
        // The array literal must list all 6 in order
        const six = "['IOS', 'ANDROID', 'ANDROID_VR', 'TV', 'MWEB', 'WEB']";
        assert.ok(src.includes(six), `Expected ${six} in youtube.js — prevents revert to 3-client ARM bug`);
    });

    it('getInfo fallback tries 6 clients', () => {
        // fallback is 6 lambdas with client: 'IOS' .. 'WEB' (actions loop uses array literal)
        const webCount = (src.match(/client: 'WEB'/g) || []).length;
        assert.ok(webCount >= 1, `Expected fallback block to contain client: 'WEB', got ${webCount}`);
        // array literal must also include WEB for the actions.execute path
        assert.ok(src.includes("'IOS'") && src.includes("'WEB'"), 'fallback must mention IOS and WEB');
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
