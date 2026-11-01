const { connectToCdp } = require('./cdp_helper.js');
const { execSync } = require('child_process');
const fs = require('fs');

function execAdb(cmd) {
    try {
        return execSync(`source ./env.sh && adb ${cmd}`, { shell: '/bin/bash', encoding: 'utf8' }).trim();
    } catch (e) {
        return (e.stdout || e.message || '').trim();
    }
}

async function sleep(ms) {
    return new Promise(r => setTimeout(r, ms));
}

async function main() {
    console.log('===============================================================');
    console.log('  LIVE VERIFICATION: Multi-Stream Cancellation & Zero Overlap');
    console.log('===============================================================');

    // Wake up device screen
    execAdb('shell input keyevent 224'); // WAKEUP
    execAdb('shell wm dismiss-keyguard');

    console.log('[1/5] Connecting to Chrome DevTools Protocol...');
    const { cdp } = await connectToCdp();

    // Install runtime audio tracker on the page
    console.log('[2/5] Installing live Audio concurrency monitor in WebView...');
    await cdp.evaluate(`
        (() => {
            window.__audioMonitor = {
                activeAudios: new Set(),
                maxConcurrentPlaying: 0,
                currentlyPlayingCount: 0,
                playEvents: [],
                supersededAudiosStopped: 0,
            };

            const origPlay = HTMLAudioElement.prototype.play;
            const origPause = HTMLAudioElement.prototype.pause;

            HTMLAudioElement.prototype.play = async function(...args) {
                const audio = this;
                window.__audioMonitor.activeAudios.add(audio);
                const res = await origPlay.apply(this, args);
                
                // Recount currently playing audios
                let playing = 0;
                for (const a of window.__audioMonitor.activeAudios) {
                    if (!a.paused && a.readyState > 0) playing++;
                }
                window.__audioMonitor.currentlyPlayingCount = playing;
                if (playing > window.__audioMonitor.maxConcurrentPlaying) {
                    window.__audioMonitor.maxConcurrentPlaying = playing;
                }
                window.__audioMonitor.playEvents.push({
                    src: audio.src,
                    at: Date.now(),
                    concurrentPlaying: playing,
                });
                return res;
            };

            HTMLAudioElement.prototype.pause = function(...args) {
                const res = origPause.apply(this, args);
                let playing = 0;
                for (const a of window.__audioMonitor.activeAudios) {
                    if (!a.paused && a.readyState > 0) playing++;
                }
                window.__audioMonitor.currentlyPlayingCount = playing;
                return res;
            };
            return true;
        })()
    `);

    // Switch to search view
    console.log('[3/5] Navigating to Download / YouTube Search view...');
    await cdp.evaluate(`
        (async () => {
            if (window.Auralis?.bridge?.switchView) {
                await window.Auralis.bridge.switchView('download');
            } else {
                htmx.ajax('GET', '/partials/download', { target: '#content', swap: 'innerHTML' });
            }
        })()
    `);
    await sleep(1500);

    // Perform YouTube Search
    const query = 'NCS Cartoon On and On';
    console.log(`[4/5] Searching YouTube for: "${query}"...`);
    const searchRes = await cdp.evaluate(`
        (async () => {
            const searchInput = document.querySelector('#youtube-search-form input[name="q"]');
            if (searchInput) searchInput.value = '${query}';
            const form = document.getElementById('download-form');
            const opts = window.Auralis?.bridge?.getDownloadOptions ? window.Auralis.bridge.getDownloadOptions(form) : {};
            await window.Auralis.bridge.performYouTubeSearch('${query}', opts);
            const results = window.Auralis?.bridge?._lastSearchResults || [];
            return {
                count: results.length,
                titles: results.slice(0, 3).map(r => r.title)
            };
        })()
    `);

    console.log(`  -> Found ${searchRes.count} results:`);
    (searchRes.titles || []).forEach((t, i) => console.log(`     [${i}] ${t}`));

    if (!searchRes.count || searchRes.count < 3) {
        throw new Error('Need at least 3 search results for rapid-fire test');
    }

    // Rapid-fire test: trigger stream 0, then stream 1 after 100ms, then stream 2 after 100ms
    console.log('\n[5/5] Rapid-fire clicking 3 YouTube streams in rapid succession...');
    console.log('  -> Triggering streamYouTubeSearchResult(0)...');
    cdp.evaluate(`window.Auralis.bridge.streamYouTubeSearchResult(0);`);
    await sleep(100);

    console.log('  -> Triggering streamYouTubeSearchResult(1) (while 0 is in flight)...');
    cdp.evaluate(`window.Auralis.bridge.streamYouTubeSearchResult(1);`);
    await sleep(100);

    console.log('  -> Triggering streamYouTubeSearchResult(2) (while 1 is in flight)...');
    cdp.evaluate(`window.Auralis.bridge.streamYouTubeSearchResult(2);`);

    console.log('\n  -> Monitoring stream resolution and playback over 12 seconds...');
    let lastReport = null;
    let maxConcurrent = 0;
    for (let sec = 1; sec <= 12; sec++) {
        await sleep(1000);
        const mon = await cdp.evaluate(`
            (() => {
                const m = window.__audioMonitor || {};
                let playing = 0;
                for (const a of (m.activeAudios || [])) {
                    if (!a.paused) playing++;
                }
                const curAudio = window._auralisStreamAudio;
                return {
                    sec: ${sec},
                    currentlyPlaying: playing,
                    maxConcurrent: m.maxConcurrentPlaying || 0,
                    activeAudiosTotal: (m.activeAudios ? m.activeAudios.size : 0),
                    playEventsCount: (m.playEvents ? m.playEvents.length : 0),
                    curAudioExists: Boolean(curAudio),
                    curAudioPaused: curAudio ? curAudio.paused : null,
                    curAudioTime: curAudio ? curAudio.currentTime : null,
                    curAudioDuration: curAudio ? curAudio.duration : null,
                    curVideoId: window._auralisStreamVideoId,
                    playerTrack: window.Auralis?.player?.currentTrack?.title,
                    playerIsPlaying: window.Auralis?.player?.isPlaying
                };
            })()
        `);
        lastReport = mon;
        if (mon.maxConcurrent > maxConcurrent) maxConcurrent = mon.maxConcurrent;
        console.log(`  [t=${sec}s] Playing: ${mon.currentlyPlaying} | Max Concurrent: ${mon.maxConcurrent} | Audio Time: ${mon.curAudioTime ? mon.curAudioTime.toFixed(1) : 'none'}s | Track: "${mon.playerTrack || 'none'}"`);
    }

    console.log('\n===============================================================');
    console.log('  VERIFICATION RESULTS:');
    console.log(`  Max Concurrent Playing Audios: ${maxConcurrent} (MUST BE <= 1)`);
    console.log(`  Currently Playing: ${lastReport.currentlyPlaying}`);
    console.log(`  Current Track Title: "${lastReport.playerTrack}"`);
    console.log(`  Audio Current Time: ${lastReport.curAudioTime ? lastReport.curAudioTime.toFixed(1) : 0}s`);
    console.log('===============================================================');

    if (maxConcurrent > 1) {
        throw new Error(`FAILURE: Audio overlap detected! Max concurrent playing audios was ${maxConcurrent} (> 1)`);
    }

    console.log('  ✓ PASSED: ZERO audio overlap observed! Max concurrent playing audios never exceeded 1.');

    // Now test library transition: playing local track must immediately stop stream
    console.log('\n[Follow-up Check] Switching to local library track while stream is active...');
    const libRes = await cdp.evaluate(`
        (async () => {
            const tracks = await window.Auralis.bridge.invoke('get_tracks', { filter: { limit: 1 } });
            const track = tracks?.tracks?.[0];
            if (!track) return { success: false, reason: 'No library tracks' };
            await window.Auralis.bridge.playTrack(track.id);
            await new Promise(r => setTimeout(r, 600));
            return {
                success: true,
                streamAudioExists: Boolean(window._auralisStreamAudio),
                playerTrack: window.Auralis?.player?.currentTrack?.title,
                rustPlaying: window.Auralis?.player?.isPlaying
            };
        })()
    `);
    console.log('  -> Library playback result:', libRes);

    if (libRes.streamAudioExists) {
        throw new Error('FAILURE: stream audio was NOT destroyed after library track play!');
    }
    console.log('  ✓ PASSED: YouTube stream was cleanly stopped and detached upon playing library track.');

    // Take screenshot of phone screen for proof
    const screenshotPath = '/home/intro/.gemini/antigravity-cli/brain/add18ab6-b636-42c1-8216-f64070fe3af4/stream_no_overlap_verified.png';
    execAdb(`shell screencap -p /sdcard/stream_no_overlap.png`);
    execAdb(`pull /sdcard/stream_no_overlap.png ${screenshotPath}`);
    execAdb(`shell rm /sdcard/stream_no_overlap.png`);
    console.log(`  ✓ Screenshot saved to: ${screenshotPath}`);

    console.log('\n===============================================================');
    console.log('  ALL CHECKS PASSED: 100% verified on real Android 15 phone!');
    console.log('===============================================================');
    process.exit(0);
}

main().catch(err => {
    console.error('\n❌ TEST FAILED:', err);
    process.exit(1);
});
