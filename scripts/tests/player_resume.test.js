#!/usr/bin/env node
/**
 * player_resume.test.js — regression tests for the "pause then play is dead" bug.
 *
 * Runs with Node's built-in runner (no npm deps):
 *   node --test scripts/tests/player_resume.test.js
 *
 * Bug: pressing pause left the app unable to play anything again until it was
 * reopened. Two halves:
 *  1. `AudioPlayer::resume()` returned `Ok(())` when there was nothing to
 *     resume (no sink, or a drained rodio `Player` whose `play()` is a no-op),
 *     so the failure was invisible to the caller.
 *  2. `PlayerController.play()` treated a resolved `invoke('resume')` as proof
 *     that sound started, optimistically set `isPlaying = true`, and returned —
 *     so every later press took the same dead path.
 *
 * The controller is loaded from source into a `node:vm` context with a minimal
 * fake DOM/bridge, so the real `play()`/timeout/fallback logic runs here
 * without a WebView. The Rust half is guarded by source assertions.
 */
import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import vm from 'node:vm';

const here = import.meta.dirname ?? path.dirname(new URL(import.meta.url).pathname);
const playerJsPath = path.resolve(here, '../../ui/js/player.js');
const playerRsPath = path.resolve(here, '../../src/infrastructure/media/player.rs');
const playerJsSrc = fs.readFileSync(playerJsPath, 'utf8');
const playerRsSrc = fs.readFileSync(playerRsPath, 'utf8');

const delay = (ms) => new Promise((r) => setTimeout(r, ms));

/**
 * Build an isolated environment containing the real PlayerController plus a
 * fake bridge that records what the controller asked the backend to do.
 */
function makeEnv() {
    const listeners = {};
    const record = { playTrackCalls: [], toasts: [], invokes: [] };

    const bridge = {
        on(event, cb) {
            if (!listeners[event]) listeners[event] = [];
            listeners[event].push(cb);
        },
        emit(event, data) {
            (listeners[event] || []).forEach((cb) => cb(data));
        },
        async invoke(command) {
            record.invokes.push(command);
            return null;
        },
        showToast(msg, kind) {
            record.toasts.push({ msg: String(msg), kind: kind || 'info' });
        },
        playTrack(trackId) {
            record.playTrackCalls.push(trackId);
        },
    };

    const noop = () => {};
    const documentStub = {
        addEventListener: noop,
        removeEventListener: noop,
        getElementById: () => null,
        querySelector: () => null,
        querySelectorAll: () => [],
        createElement: () => ({ style: {}, classList: { toggle: noop, contains: () => false, add: noop, remove: noop } }),
        body: { addEventListener: noop, classList: { contains: () => false } },
    };

    const sandbox = {
        console: { log: noop, warn: noop, error: noop, info: noop, debug: noop, groupCollapsed: noop, groupEnd: noop },
        document: documentStub,
        navigator: {},
        setTimeout,
        clearTimeout,
    };
    sandbox.window = sandbox;
    sandbox.window.Auralis = { bridge };
    sandbox.globalThis = sandbox;

    const context = vm.createContext(sandbox);
    // Expose the class: a top-level `class` binding is lexical, not a global.
    vm.runInContext(`${playerJsSrc}\n;globalThis.__PlayerController = PlayerController;`, context);
    const PlayerController = sandbox.__PlayerController;

    return {
        bridge,
        record,
        create() {
            const ctrl = new PlayerController();
            // Keep the windows short so the tests stay fast and deterministic.
            ctrl.RESUME_VERIFY_MS = 20;
            return ctrl;
        },
    };
}

describe('play() resume path: a successful resume reply is not proof of sound', () => {
    it('replays the track when nothing started within the window (the reported bug)', async () => {
        const env = makeEnv();
        const ctrl = env.create();
        ctrl.currentTrack = { id: 't1', title: 'A', duration_secs: 100 };

        // `resume` answers Ok but never emits `playback:state` — exactly what
        // AudioPlayer::resume did when the sink was missing or drained.
        await ctrl.play();

        assert.ok(env.record.invokes.includes('resume'), 'play() should try resume first');
        assert.deepEqual(env.record.playTrackCalls, ['t1'], 'the track must be replayed from the start');
        assert.equal(ctrl.isPlaying, false, 'isPlaying must not stay stuck at true');
        assert.ok(
            env.record.toasts.some((t) => /replay/i.test(t.msg)),
            `expected a replay toast, got ${JSON.stringify(env.record.toasts)}`
        );
    });

    it('does not replay when playback:state confirms is_playing', async () => {
        const env = makeEnv();
        const ctrl = env.create();
        ctrl.currentTrack = { id: 't1', title: 'A', duration_secs: 100 };
        // Rust emits playback:state_changed from inside the resume command, so
        // the event can beat the invoke's reply back to us.
        env.bridge.invoke = async (command) => {
            env.record.invokes.push(command);
            if (command === 'resume') env.bridge.emit('playback:state', { is_playing: true });
            return null;
        };

        await ctrl.play();
        await delay(40);

        assert.deepEqual(env.record.playTrackCalls, [], 'a confirmed resume must not replay the track');
        assert.equal(ctrl.isPlaying, true, 'isPlaying must follow the backend');
    });

    it('accepts a playback:progress tick as proof of life', async () => {
        const env = makeEnv();
        const ctrl = env.create();
        ctrl.currentTrack = { id: 't1', title: 'A', duration_secs: 100 };
        env.bridge.invoke = async (command) => {
            env.record.invokes.push(command);
            if (command === 'resume') {
                setTimeout(() => env.bridge.emit('playback:progress', { position: 12.5, duration: 100 }), 1);
            }
            return null;
        };

        await ctrl.play();
        await delay(40);

        assert.deepEqual(env.record.playTrackCalls, [], 'a progress tick means the sink is running');
    });

    it('falls back immediately (once) when the resume invoke itself fails', async () => {
        const env = makeEnv();
        const ctrl = env.create();
        ctrl.currentTrack = { id: 't1', title: 'A', duration_secs: 100 };
        env.bridge.invoke = async (command) => {
            env.record.invokes.push(command);
            throw 'Resume error: State error: nothing to resume: playback is not active';
        };

        await ctrl.play();
        await delay(40);

        // The invariant this suite exists to protect is that a failed resume
        // never leaves the user with *nothing happening*. That is carried by
        // the replay, which must still fire.
        assert.deepEqual(env.record.playTrackCalls, ['t1'], 'the error path must replay the track');

        // ...and NOT by a red toast for this specific error. "nothing to resume"
        // is the case Rust deliberately reports so we replay — pressing play
        // after a song has finished is normal intent, and announcing the
        // recovery as a failure is what the owner reported as a bug
        // ("Resume failed: ... the track already finished"). The visible-guard
        // half of the original fix moves to the unexpected-error test below.
        assert.equal(
            env.record.toasts.filter((t) => /Resume failed/i.test(t.msg)).length,
            0,
            'a self-recovering end-of-track resume must not raise an error toast'
        );
    });

    it('still surfaces a resume failure that is NOT the expected drained-sink case', async () => {
        const env = makeEnv();
        const ctrl = env.create();
        ctrl.currentTrack = { id: 't1', title: 'A', duration_secs: 100 };
        env.bridge.invoke = async (command) => {
            env.record.invokes.push(command);
            throw 'Resume error: something we have never seen before';
        };

        await ctrl.play();
        await delay(40);

        assert.deepEqual(env.record.playTrackCalls, ['t1'], 'the error path must replay the track');
        assert.ok(
            env.record.toasts.some((t) => /Resume failed/i.test(t.msg)),
            'an unrecognised resume failure must still be surfaced'
        );
    });

    it('does nothing on timeout when the track changed in the meantime', async () => {
        const env = makeEnv();
        const ctrl = env.create();
        ctrl.currentTrack = { id: 't1', title: 'A', duration_secs: 100 };
        env.bridge.invoke = async (command) => {
            env.record.invokes.push(command);
            if (command === 'resume') {
                // User hits Next while the check is pending.
                setTimeout(() => { ctrl.currentTrack = { id: 't2', title: 'B', duration_secs: 100 }; }, 2);
            }
            return null;
        };

        await ctrl.play();
        await delay(40);

        assert.deepEqual(env.record.playTrackCalls, [], 'a stale resume must not hijack the new track');
        assert.equal(
            env.record.toasts.filter((t) => /replay/i.test(t.msg)).length,
            0,
            'no replay toast for a stale resume'
        );
    });

    it('pause() cancels the pending check instead of being undone by the timer', async () => {
        const env = makeEnv();
        const ctrl = env.create();
        ctrl.currentTrack = { id: 't1', title: 'A', duration_secs: 100 };

        const playing = ctrl.play();
        ctrl.pause();
        await playing;
        await delay(40);

        assert.deepEqual(env.record.playTrackCalls, [], 'the timeout must not restart playback after a pause');
        assert.equal(ctrl.isPlaying, false);
    });
});

describe('togglePlay(): a second tap must not cancel an in-flight resume', () => {
    // The device bug, in full. play() sets isPlaying = true before its invoke
    // returns, so a fast second tap used to be dispatched as pause() and cancel
    // the resume that was still starting. The resume agent localised this by
    // elimination: only PlayerController.play() produces a `resume` command,
    // `replay=` is absent from every log line so the replay path never ran, and
    // therefore the two extra `resume` commands were the owner's own taps.
    //
    // Both observed verdicts follow from that one cause:
    //   probe=superseded                  -> tap 3 replaced tap 1's armed probe
    //   never_playing polls=7 progress=0  -> tap 2 paused a sink still queued
    //                                         (empty=0 proves nothing drained)
    it('ignores a tap that lands while the resume round-trip is still open', async () => {
        const env = makeEnv();
        const ctrl = env.create();
        ctrl.currentTrack = { id: 't1', title: 'A', duration_secs: 100 };

        // Hold the `resume` invoke open so the tap provably lands mid-round-trip.
        let releaseResume;
        env.bridge.invoke = async (command) => {
            env.record.invokes.push(command);
            if (command === 'resume') {
                await new Promise((r) => { releaseResume = r; });
                env.bridge.emit('playback:state', { is_playing: true });
            }
            return null;
        };

        const playing = ctrl.play();
        await delay(10);
        assert.equal(ctrl.resumeInFlight, true, 'the resume should be in flight');

        // The second tap. Without the guard this dispatches `pause`.
        ctrl.togglePlay();
        assert.ok(
            !env.record.invokes.includes('pause'),
            `a tap during an in-flight resume must not dispatch pause, got ${JSON.stringify(env.record.invokes)}`
        );
        assert.equal(ctrl.isPlaying, true, 'the resume must still be treated as playing');

        releaseResume();
        await playing;
        await delay(20);
        assert.equal(ctrl.resumeInFlight, false, 'the guard must be released afterwards');
    });

    it('still pauses normally once the resume has settled', async () => {
        const env = makeEnv();
        const ctrl = env.create();
        ctrl.currentTrack = { id: 't1', title: 'A', duration_secs: 100 };

        env.bridge.invoke = async (command) => {
            env.record.invokes.push(command);
            if (command === 'resume') env.bridge.emit('playback:state', { is_playing: true });
            return null;
        };

        await ctrl.play();
        await delay(20);
        assert.equal(ctrl.resumeInFlight, false);

        ctrl.togglePlay();
        assert.ok(
            env.record.invokes.includes('pause'),
            'a settled player must still pause on tap, or the button is simply broken'
        );
        assert.equal(ctrl.isPlaying, false);
    });

    it('does not swallow a real pause() from the notification or MediaSession', async () => {
        // The guard lives in togglePlay(), not pause(), precisely so that the
        // hardware/notification stop path still works mid-resume. If this ever
        // moves into pause(), the user cannot stop playback until a round-trip
        // completes.
        const env = makeEnv();
        const ctrl = env.create();
        ctrl.currentTrack = { id: 't1', title: 'A', duration_secs: 100 };

        let releaseResume;
        env.bridge.invoke = async (command) => {
            env.record.invokes.push(command);
            if (command === 'resume') await new Promise((r) => { releaseResume = r; });
            return null;
        };

        const playing = ctrl.play();
        await delay(10);
        ctrl.pause();               // what MediaPlaybackService.kt calls via JNI
        assert.ok(
            env.record.invokes.includes('pause'),
            'a direct pause() during an in-flight resume must reach the backend'
        );
        assert.equal(ctrl.isPlaying, false);

        releaseResume();
        await playing;
        await delay(20);
    });

    it('releases the guard even when the resume invoke throws', async () => {
        // A guard left set by an exception would disable the play button for
        // the rest of the session — a worse bug than the one being fixed.
        const env = makeEnv();
        const ctrl = env.create();
        ctrl.currentTrack = { id: 't1', title: 'A', duration_secs: 100 };

        env.bridge.invoke = async (command) => {
            env.record.invokes.push(command);
            if (command === 'resume') throw new Error('boom');
            return null;
        };

        await ctrl.play();
        assert.equal(ctrl.resumeInFlight, false, 'the guard must not survive a failed resume');
        assert.deepEqual(env.record.playTrackCalls, ['t1'], 'the fallback replay still happens');

        // And the button works again.
        env.record.invokes.length = 0;
        ctrl.isPlaying = false;
        ctrl.togglePlay();
        assert.ok(env.record.invokes.includes('resume'), 'play must work again after a failed resume');
    });
});

describe('seek gate: a committed seek outranks in-flight progress ticks', () => {
    // The recorded defect: scrub to 0:18, lift, and the handle rubber-banded
    // back to 2:17 and sat there for 3.75s with no fingers on the glass.
    // `endSeek` clears `isSeeking` synchronously, but `commitSeek` is a
    // fire-and-forget round trip — so every tick arriving in between still
    // reports the *pre-seek* coordinate and was being painted straight over
    // the scrub target.
    it('blocks ticks until Rust acknowledges the seek, then lets them through', async () => {
        const env = makeEnv();
        const ctrl = env.create();
        ctrl.duration = 198;
        ctrl.progress = 18;

        ctrl.commitSeek();
        assert.equal(ctrl.seekInFlight, true, 'the gate must be armed before dispatch');

        // Still at 2:17 on the Rust side: the rebuild has not finished.
        env.bridge.emit('playback:progress', { position_secs: 137, duration_secs: 198 });
        assert.equal(ctrl.progress, 18, 'an in-flight tick must not rubber-band the scrub target');

        await delay(10);
        assert.equal(ctrl.seekInFlight, false, 'the gate must open once the seek settles');

        env.bridge.emit('playback:progress', { position_secs: 20, duration_secs: 198 });
        assert.equal(ctrl.progress, 20, 'ticks must flow again once the seek has landed');
    });

    it('only the newest of two rapid seeks may open the gate', async () => {
        const env = makeEnv();
        const ctrl = env.create();
        ctrl.duration = 198;
        const pending = [];
        env.bridge.invoke = (command) => {
            env.record.invokes.push(command);
            return new Promise((resolve) => pending.push(resolve));
        };

        ctrl.progress = 137;
        ctrl.commitSeek();          // generation 1
        ctrl.progress = 18;
        ctrl.commitSeek();          // generation 2
        assert.equal(ctrl.seekInFlight, true);

        pending[0]();               // the first seek lands while the second is still out
        await delay(5);
        assert.equal(
            ctrl.seekInFlight,
            true,
            'the first seek settling must not open the gate while the second is in flight'
        );

        pending[1]();
        await delay(5);
        assert.equal(ctrl.seekInFlight, false, 'the newest seek settling must open the gate');
    });

    it('opens the gate when the seek fails, so a rejected invoke cannot freeze the bar', async () => {
        const env = makeEnv();
        const ctrl = env.create();
        ctrl.duration = 198;
        env.bridge.invoke = async () => {
            env.record.invokes.push('seek');
            throw 'seek unavailable';
        };

        ctrl.progress = 18;
        ctrl.commitSeek();
        assert.equal(ctrl.seekInFlight, true);

        await delay(20);
        assert.equal(ctrl.seekInFlight, false, 'a failed seek must not leave progress gated forever');
        assert.ok(
            env.record.toasts.some((t) => /Seek failed/i.test(t.msg)),
            'the failure must still be reported'
        );
    });

    it('does not arm the gate when commitSeek bails out before dispatching', () => {
        const env = makeEnv();
        const ctrl = env.create();
        // Invalid duration -> the guard returns before any invoke. A gate armed
        // here could never be released, which would freeze progress permanently.
        ctrl.duration = 0;
        ctrl.progress = 18;
        ctrl.commitSeek();
        assert.equal(ctrl.seekInFlight, false, 'an early return must not arm the gate');
        // `init()` already dispatched `get_now_playing`, so count seeks only.
        assert.equal(
            env.record.invokes.filter((c) => c === 'seek').length,
            0,
            'nothing should have been dispatched'
        );
    });
});

describe('source guards: the fix must not be silently reverted', () => {
    it('player.js no longer treats a bare resume as success', () => {
        // The old shape was: invoke('resume') … return;  with no verification.
        assert.ok(
            !/await window\.Auralis\.bridge\.invoke\('resume'\);\s*\}\s*catch[\s\S]{0,400}?\breturn;\s*\}/.test(playerJsSrc),
            'the resume branch must verify that playback started before returning'
        );
        assert.ok(playerJsSrc.includes('this.awaitPlaybackStart('), 'play() must arm a proof-of-life check');
        assert.ok(playerJsSrc.includes('await started;'), 'play() must wait for the check');
        assert.ok(playerJsSrc.includes('this.renewResumeWatch('), 'the check must be re-armed after a slow invoke');
        assert.ok(playerJsSrc.includes('bridge.playTrack(watch.trackId)'), 'the timeout must replay the track');
        assert.ok(
            playerJsSrc.includes('Resume did not start'),
            'the timeout must tell the user what happened'
        );
    });

    it('player.js confirms playback from the existing event listeners', () => {
        assert.ok(
            playerJsSrc.includes('if (state.is_playing) this.settleResumeWatch(true);'),
            'playback:state with is_playing must settle the pending check'
        );
        assert.ok(
            playerJsSrc.includes("on('playback:progress', (data) => {\n            // The Rust watcher only emits progress"),
            'playback:progress must also settle the pending check'
        );
        assert.ok(
            playerJsSrc.includes('this.settleResumeWatch(false);'),
            'the resume failure path must disarm the check so it cannot fire later'
        );
    });

    it('AudioPlayer::resume reports failure instead of a silent no-op', () => {
        assert.ok(
            playerRsSrc.includes('"nothing to resume: playback is not active"'),
            'a missing sink must be a StateError'
        );
        assert.ok(
            playerRsSrc.includes('"nothing to resume: the track already finished"'),
            'a drained rodio player must be a StateError (play() cannot revive it)'
        );
        // The old body ended the `if let Some(s)` with a bare `Ok(())`, which is
        // what made the failure invisible.
        assert.ok(
            !/if let Some\(s\) = sink_guard\.as_ref\(\) \{[\s\S]{0,400}?\n {8}Ok\(\(\)\)\n {4}\}/.test(playerRsSrc),
            'resume() must not keep the "no sink -> Ok(())" shape'
        );
        assert.ok(
            playerRsSrc.includes('if s.empty() {'),
            'resume() must check that the sink still has audio queued'
        );
    });

    it('the resume toast is suppressed ONLY for the drained-sink case', () => {
        // Suppressing the toast is scoped deliberately. Widen it and a real
        // resume failure becomes invisible again — the exact defect this suite
        // was written to prevent. This pins the shape so it cannot drift.
        const idx = playerJsSrc.indexOf("msg.includes('nothing to resume')");
        assert.ok(idx > -1, 'the suppression must be keyed on the drained-sink message');
        const tail = playerJsSrc.slice(idx, idx + 300);
        assert.ok(tail.includes('} else {'), 'the toast must live in an else branch');
        assert.ok(
            tail.includes('showToast(`Resume failed:'),
            'every other resume failure must still be toasted'
        );
    });

    it('both seek controls release isSeeking on touchcancel', () => {
        // Android delivers touchcancel (edge swipe, gesture nav, incoming call)
        // instead of touchend. `isSeeking` gates every playback:progress event,
        // so a missed release froze the progress bar for 17s in a device
        // recording while audio kept playing in the background.
        assert.equal(
            (playerJsSrc.match(/addEventListener\('touchcancel'/g) || []).length,
            2,
            'progressTrack and fullProgress must both register touchcancel'
        );
    });

    it('the three cosmetic player fixes stay in place', () => {
        // Neither styling nor an HTML fallback string has a behavioural
        // harness, so pin the shapes rather than trust them to survive.
        const css = fs.readFileSync(path.resolve(here, '../../ui/styles/components.css'), 'utf8');
        const mod = fs.readFileSync(path.resolve(here, '../../ui/js/modules/player.js'), 'utf8');

        assert.match(
            css,
            /\.player-full-play\.is-playing \{[^}]*box-shadow: 0 0 24px/,
            'the cyan glow must be scoped to the playing state'
        );
        assert.ok(
            !/\.player-full-play \{[^}]*box-shadow: 0 0 24px/.test(css),
            'a paused button must not glow'
        );
        assert.match(
            css,
            /@media \(hover: none\), \(pointer: coarse\) \{\s*\.progress-handle \{\s*opacity: 1;/,
            'touch devices must keep the scrub handle visible after release'
        );
        assert.ok(
            mod.includes("actualTrack.artist || 'Unknown Artist'"),
            'a playing track with no artist tag must not read "Select a song"'
        );
        assert.ok(
            !mod.includes("|| 'Select a song'"),
            'the placeholder must not be reachable while a track is playing'
        );
    });
});
