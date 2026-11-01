#!/usr/bin/env node
/**
 * stream_cancellation.test.js — JS unit tests for YouTube streaming race condition prevention
 * and audio overlap cancellation.
 */
import { describe, it, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { coreMethods } from '../../ui/js/modules/core.js';
import { uiMethods } from '../../ui/js/modules/ui.js';
import { downloadMethods } from '../../ui/js/modules/downloads.js';
import { playerMethods } from '../../ui/js/modules/player.js';

describe('Stream cancellation and multi-audio overlap prevention', () => {
    beforeEach(() => {
        // Reset mock window state
        global.window = {
            _auralisStreamAudio: null,
            _auralisStreamVideoId: null,
            __auralisStreamRequestId: 0,
            __auralisStreamInFlight: null,
            Auralis: {
                player: {
                    isPlaying: false,
                    currentTrack: null,
                    progress: 0,
                    duration: 0,
                    updatePlayButton: () => {},
                    updateProgressUI: () => {},
                    updateTrackInfo: () => {},
                    updatePositionState: () => {},
                },
                bridge: {},
            },
            lucide: { createIcons: () => {} },
        };
        global.document = {
            getElementById: () => null,
            querySelector: () => null,
            querySelectorAll: () => [],
        };
    });

    it('downloadMethods._stopStreamAudio cleanly stops, removes src, and clears audio instance', () => {
        let paused = false;
        let loaded = false;
        let removedSrc = false;
        const mockAudio = {
            currentTime: 42,
            pause: () => { paused = true; },
            removeAttribute: (attr) => { if (attr === 'src') removedSrc = true; },
            load: () => { loaded = true; },
        };

        window._auralisStreamAudio = mockAudio;
        window._auralisStreamVideoId = 'vid_123';

        const ctx = {
            _stopStreamAudio: downloadMethods._stopStreamAudio,
            invoke: async () => {},
        };

        ctx._stopStreamAudio();

        assert.equal(paused, true, 'Audio must be paused');
        assert.equal(removedSrc, true, 'src attribute must be removed');
        assert.equal(loaded, true, 'load() must be called to release decoder');
        assert.equal(mockAudio.currentTime, 0, 'currentTime must be reset to 0');
        assert.equal(window._auralisStreamAudio, null, 'window._auralisStreamAudio must be set to null');
        assert.equal(window._auralisStreamVideoId, null, 'window._auralisStreamVideoId must be set to null');
    });

    it('playTrack cancels pending stream requests and calls _stopStreamAudio', async () => {
        window.__auralisStreamRequestId = 5;
        let stopCalled = false;
        const ctx = {
            ...playerMethods,
            tracks: [{ id: 'track_1', title: 'Local Song' }],
            _stopStreamAudio: () => { stopCalled = true; },
            setQueue: async () => {},
            invoke: async () => ({ track: { id: 'track_1' } }),
            updatePlayerBar: () => {},
        };

        await ctx.playTrack('track_1');

        assert.equal(window.__auralisStreamRequestId, 6, 'Request ID must increment to invalidate pending streams');
        assert.equal(stopCalled, true, '_stopStreamAudio must be called on playTrack');
    });

    it('superseded stream requests do not start audio or overwrite player state', async () => {
        let createdAudios = [];
        class MockAudio {
            constructor() {
                this.src = '';
                this.paused = true;
                this.currentTime = 0;
                this.events = {};
                createdAudios.push(this);
            }
            addEventListener(ev, fn) { this.events[ev] = fn; }
            removeAttribute() {}
            load() {}
            pause() { this.paused = true; }
            async play() { this.paused = false; }
        }
        global.Audio = MockAudio;

        const results = [
            { id: 'video_1', title: 'Video 1', url: 'https://youtube.com/watch?v=1' },
            { id: 'video_2', title: 'Video 2', url: 'https://youtube.com/watch?v=2' },
        ];

        let resolve1Finished = false;
        window.AuralisYouTube = {
            resolve: async (url) => {
                if (url.includes('v=1')) {
                    // Simulate slower network response for video 1
                    await new Promise(r => setTimeout(r, 50));
                    resolve1Finished = true;
                    return { kind: 'track', stream_url: 'http://cdn/1', duration: 180 };
                }
                // Faster response for video 2
                return { kind: 'track', stream_url: 'http://cdn/2', duration: 200 };
            }
        };

        const ctx = {
            ...coreMethods,
            ...uiMethods,
            ...downloadMethods,
            _lastSearchResults: results,
            getDownloadOptions: () => ({}),
            showToast: () => {},
            invoke: async (cmd, args) => {
                if (cmd === 'register_stream_url') return `http://127.0.0.1:8888/stream/${args.streamUrl.slice(-1)}`;
                return null;
            },
        };

        // Trigger video 1, then immediately video 2
        const p1 = ctx.streamYouTubeSearchResult(0);
        const p2 = ctx.streamYouTubeSearchResult(1);

        await Promise.all([p1, p2]);

        // Only video 2 should have become the active stream
        assert.equal(window._auralisStreamVideoId, 'video_2', 'Active video must be video_2');
        assert.equal(window.Auralis.player.currentTrack.title, 'Video 2', 'Current track must be Video 2');
        // Only one audio should be playing
        const playingAudios = createdAudios.filter(a => !a.paused);
        assert.equal(playingAudios.length, 1, 'Exactly one audio element must be playing');
        assert.ok(createdAudios[createdAudios.length - 1].src.includes('/stream/2'), 'Playing audio must be stream 2');
    });

    it('events from an orphaned audio element are ignored and do not affect player state', async () => {
        let createdAudios = [];
        class MockAudio {
            constructor() {
                this.src = '';
                this.paused = true;
                this.currentTime = 0;
                this.events = {};
                createdAudios.push(this);
            }
            addEventListener(ev, fn) { this.events[ev] = fn; }
            removeAttribute() {}
            load() {}
            pause() { this.paused = true; }
            async play() { this.paused = false; }
        }
        global.Audio = MockAudio;

        const results = [
            { id: 'video_1', title: 'Video 1', url: 'https://youtube.com/watch?v=1' },
            { id: 'video_2', title: 'Video 2', url: 'https://youtube.com/watch?v=2' },
        ];

        window.AuralisYouTube = {
            resolve: async (url) => ({
                kind: 'track',
                stream_url: `http://cdn/${url.slice(-1)}`,
                duration: 200,
            }),
        };

        const ctx = {
            ...coreMethods,
            ...uiMethods,
            ...downloadMethods,
            _lastSearchResults: results,
            getDownloadOptions: () => ({}),
            showToast: () => {},
            invoke: async (cmd, args) => {
                if (cmd === 'register_stream_url') return `http://127.0.0.1:8888/stream/${args.streamUrl.slice(-1)}`;
                return null;
            },
        };

        // First play video 1 completely
        await ctx.streamYouTubeSearchResult(0);
        const audio1 = createdAudios[0];
        assert.ok(audio1, 'audio1 must exist');
        assert.equal(window.Auralis.player.currentTrack.title, 'Video 1');

        // Now play video 2
        await ctx.streamYouTubeSearchResult(1);
        const audio2 = createdAudios[1];
        assert.ok(audio2, 'audio2 must exist');
        assert.equal(window.Auralis.player.currentTrack.title, 'Video 2');

        // Now fire timeupdate and ended on audio1 (the orphaned element)
        audio1.currentTime = 99;
        if (audio1.events.timeupdate) audio1.events.timeupdate();
        assert.notEqual(window.Auralis.player.progress, 99, 'Orphaned audio timeupdate must NOT overwrite player.progress');

        // Fire ended on audio1
        if (audio1.events.ended) audio1.events.ended();
        assert.equal(window.Auralis.player.isPlaying, true, 'Orphaned audio ended event must NOT set player.isPlaying to false');
    });

    it('duplicate stream taps while in flight are ignored', async () => {
        let resolveCount = 0;
        class MockAudio {
            constructor() {
                this.src = '';
                this.paused = true;
                this.currentTime = 0;
                this.events = {};
            }
            addEventListener(ev, fn) { this.events[ev] = fn; }
            removeAttribute() {}
            load() {}
            pause() { this.paused = true; }
            async play() { this.paused = false; }
        }
        global.Audio = MockAudio;

        const results = [{ id: 'video_1', title: 'Video 1', url: 'https://youtube.com/watch?v=1' }];

        window.AuralisYouTube = {
            resolve: async () => {
                resolveCount++;
                await new Promise(r => setTimeout(r, 40));
                return { kind: 'track', stream_url: 'http://cdn/1', duration: 180 };
            },
        };

        const ctx = {
            ...coreMethods,
            ...uiMethods,
            ...downloadMethods,
            _lastSearchResults: results,
            getDownloadOptions: () => ({}),
            showToast: () => {},
            invoke: async () => 'http://127.0.0.1:8888/stream/1',
        };

        // Tap video 1 twice in rapid succession
        const p1 = ctx.streamYouTubeSearchResult(0);
        const p2 = ctx.streamYouTubeSearchResult(0);

        await Promise.all([p1, p2]);

        assert.equal(resolveCount, 1, 'Only one resolve must execute; duplicate in-flight tap must be ignored');
    });

    it('tapping the same already loaded video toggles play and pause without reloading', async () => {
        let playCount = 0;
        let pauseCount = 0;
        let resolveCount = 0;

        const mockAudio = {
            paused: false,
            currentTime: 10,
            play: async () => { playCount++; mockAudio.paused = false; },
            pause: () => { pauseCount++; mockAudio.paused = true; },
        };
        window._auralisStreamAudio = mockAudio;
        window._auralisStreamVideoId = 'video_1';
        window.Auralis.player.currentTrack = { title: 'Video 1' };

        window.AuralisYouTube = {
            resolve: async () => {
                resolveCount++;
                return { kind: 'track', stream_url: 'http://cdn/1' };
            },
        };

        const ctx = {
            ...coreMethods,
            ...uiMethods,
            ...downloadMethods,
            _lastSearchResults: [{ id: 'video_1', title: 'Video 1', url: 'https://youtube.com/watch?v=1' }],
            getDownloadOptions: () => ({}),
            showToast: () => {},
            invoke: async () => {},
        };

        // First tap: was playing, so should pause
        await ctx.streamYouTubeSearchResult(0);
        assert.equal(pauseCount, 1, 'Should pause audio');
        assert.equal(mockAudio.paused, true);
        assert.equal(resolveCount, 0, 'Should NOT re-resolve URL');

        // Second tap: was paused, so should play
        await ctx.streamYouTubeSearchResult(0);
        assert.equal(playCount, 1, 'Should resume audio');
        assert.equal(mockAudio.paused, false);
        assert.equal(resolveCount, 0, 'Should NOT re-resolve URL');
    });
});
