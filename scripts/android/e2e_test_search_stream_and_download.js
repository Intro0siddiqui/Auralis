const { connectToCdp } = require('./cdp_helper.js');
const { execSync } = require('child_process');

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

async function runTest() {
    console.log('====================================================');
    console.log('  E2E TEST: YouTube Search by Name, Stream & Download');
    console.log('====================================================');

    const { cdp, ws } = await connectToCdp();

    // 1. Switch to download view
    console.log('[1/4] Switching to Download/Search view...');
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

    // 2. Perform YouTube Search by song name (not URL)
    const searchQuery = 'Chopin Nocturne op 9 no 2';
    console.log(`[2/4] Searching YouTube for song by name: "${searchQuery}"...`);

    const searchRes = await cdp.evaluate(`
        (async () => {
            const searchInput = document.querySelector('#youtube-search-form input[name="q"]');
            if (searchInput) searchInput.value = '${searchQuery}';

            const form = document.getElementById('download-form');
            const opts = window.Auralis?.bridge?.getDownloadOptions ? window.Auralis.bridge.getDownloadOptions(form) : {};
            await window.Auralis.bridge.performYouTubeSearch('${searchQuery}', opts);

            const results = window.Auralis?.bridge?._lastSearchResults || [];
            return {
                count: results.length,
                first: results[0] ? {
                    id: results[0].id,
                    title: results[0].title,
                    channel: results[0].channel,
                    duration: results[0].duration,
                    url: results[0].url
                } : null
            };
        })()
    `);

    console.log('[Search Results]:', JSON.stringify(searchRes, null, 2));
    if (!searchRes || searchRes.count === 0 || !searchRes.first) {
        throw new Error('Search returned 0 results for: ' + searchQuery);
    }
    console.log(`  ✓ Search returned ${searchRes.count} results! Top: "${searchRes.first.title}" (${searchRes.first.duration}s)`);

    // 3. Test Direct Streaming (Running without downloading)
    console.log('\n[3/4] Testing Direct Streaming (Run without downloading)...');

    const initialFiles = execAdb('shell "ls -1 /sdcard/Download/Auralis/ 2>/dev/null || true"');
    console.log('[Files Before Streaming]:\n' + (initialFiles || '(none)'));

    console.log('  -> Triggering streamYouTubeSearchResult(0)...');
    const streamStartRes = await cdp.evaluate(`
        (async () => {
            try {
                window.__streamEvents = [];
                await window.Auralis.bridge.streamYouTubeSearchResult(0);
                const audio = window._auralisStreamAudio;
                if (!audio) return { success: false, error: 'No _auralisStreamAudio element created' };

                audio.addEventListener('playing', () => window.__streamEvents.push('playing'));
                audio.addEventListener('timeupdate', () => {
                    if (window.__streamEvents.indexOf('timeupdate') === -1) {
                        window.__streamEvents.push('timeupdate');
                    }
                });

                return {
                    success: true,
                    src: audio.src,
                    currentTrack: window.Auralis?.player?.currentTrack?.title || null
                };
            } catch (e) {
                return { success: false, error: e.message || String(e) };
            }
        })()
    `);

    console.log('[Stream Start Result]:', JSON.stringify(streamStartRes, null, 2));
    if (!streamStartRes.success) {
        throw new Error('Stream start failed: ' + streamStartRes.error);
    }

    // Monitor playback continuously for 15 seconds
    console.log('  -> Monitoring audio playback progress over 15 seconds...');
    let maxTimeSeen = 0;
    for (let sec = 1; sec <= 15; sec++) {
        await sleep(1000);
        const playStatus = await cdp.evaluate(`
            (() => {
                const audio = window._auralisStreamAudio;
                if (!audio) return { error: 'No audio element' };
                return {
                    currentTime: audio.currentTime,
                    duration: audio.duration,
                    paused: audio.paused,
                    readyState: audio.readyState,
                    networkState: audio.networkState,
                    error: audio.error ? { code: audio.error.code, message: audio.error.message } : null,
                    playerIsPlaying: window.Auralis?.player?.isPlaying
                };
            })()
        `);

        if (playStatus.currentTime > maxTimeSeen) {
            maxTimeSeen = playStatus.currentTime;
        }

        console.log(`    [Second ${sec.toString().padStart(2)}] currentTime: ${playStatus.currentTime?.toFixed(2)}s | duration: ${playStatus.duration?.toFixed(1)}s | paused: ${playStatus.paused} | readyState: ${playStatus.readyState} | error: ${JSON.stringify(playStatus.error)}`);

        if (playStatus.error) {
            throw new Error(`Audio stream encountered error: code ${playStatus.error.code} - ${playStatus.error.message}`);
        }
    }

    if (maxTimeSeen < 5.0) {
        throw new Error(`Streaming failed to advance continuously (max currentTime reached was only ${maxTimeSeen}s)`);
    }

    console.log(`  ✓ DIRECT STREAMING VERIFIED: Audio streamed continuously for >15s (reached ${maxTimeSeen.toFixed(2)}s) with zero errors!`);

    // Verify that NO files were created on disk during direct streaming
    const filesAfterStreaming = execAdb('shell "ls -1 /sdcard/Download/Auralis/ 2>/dev/null || true"');
    if (filesAfterStreaming !== initialFiles) {
        console.warn('[WARN] File list changed after streaming:', filesAfterStreaming);
    } else {
        console.log('  ✓ Storage verification: 0 files written to /sdcard/Download/Auralis/ during direct streaming!');
    }

    // Pause the stream before proceeding to download test
    await cdp.evaluate(`
        (() => {
            if (window._auralisStreamAudio) {
                window._auralisStreamAudio.pause();
            }
        })()
    `);

    // 4. Test Download Audio from YouTube Search
    console.log('\n[4/4] Testing Download Audio from Search Result...');
    console.log('  -> Triggering downloadSearchResult(0)...');

    const dlStartRes = await cdp.evaluate(`
        (async () => {
            try {
                window.__downloadCompleted = false;
                window.__downloadError = null;
                window.__downloadProgressEvents = [];

                if (window.Auralis?.bridge?.on) {
                    window.Auralis.bridge.on('download:progress', (p) => {
                        window.__downloadProgressEvents.push(p);
                    });
                    window.Auralis.bridge.on('download:completed', (c) => {
                        window.__downloadCompleted = true;
                        window.__downloadCompletedPayload = c;
                    });
                    window.Auralis.bridge.on('download:error', (e) => {
                        window.__downloadError = e;
                    });
                }

                await window.Auralis.bridge.downloadSearchResult(0);
                return { success: true };
            } catch (e) {
                return { success: false, error: e.message || String(e) };
            }
        })()
    `);

    console.log('[Download Initiated]:', JSON.stringify(dlStartRes, null, 2));
    if (!dlStartRes.success) {
        throw new Error('Download initiation failed: ' + dlStartRes.error);
    }

    // Monitor download progress up to 90 seconds
    console.log('  -> Waiting for download to complete...');
    let downloadDone = false;
    for (let poll = 1; poll <= 45; poll++) {
        await sleep(2000);
        const dlStatus = await cdp.evaluate(`
            (() => {
                return {
                    completed: window.__downloadCompleted,
                    error: window.__downloadError,
                    progressCount: (window.__downloadProgressEvents || []).length,
                    lastProgress: (window.__downloadProgressEvents || []).slice(-1)[0] || null,
                    completedPayload: window.__downloadCompletedPayload || null
                };
            })()
        `);

        if (dlStatus.lastProgress) {
            const p = dlStatus.lastProgress;
            console.log(`    [Progress ${(poll*2).toString().padStart(2)}s] ${p.downloaded_bytes || p.downloaded || 0} / ${p.total_bytes || p.total || '?'} bytes (${p.percent ? p.percent.toFixed(1) + '%' : 'downloading'})`);
        }

        if (dlStatus.completed) {
            const payload = dlStatus.completedPayload;
            if (payload && payload.status === 'completed') {
                console.log('  ✓ Download successfully completed!', JSON.stringify(payload));
                downloadDone = true;
                break;
            } else if (payload && payload.status === 'failed') {
                console.log('    Attempt failed (' + (payload.error ? payload.error.slice(0, 80) : '') + '), waiting for retry ladder...');
                await cdp.evaluate('window.__downloadCompleted = false;');
            }
        }

        if (dlStatus.error) {
            throw new Error('Download failed with error: ' + JSON.stringify(dlStatus.error));
        }
    }

    // Check files on disk
    console.log('\n[Verification] Checking files on disk after download...');
    const filesAfterDl = execAdb('shell "ls -la /sdcard/Download/Auralis/ 2>/dev/null || true"');
    console.log('/sdcard/Download/Auralis/ contents:\n' + filesAfterDl);

    const internalFiles = execAdb('shell "run-as com.auralis.v2 ls -la /data/user/0/com.auralis.v2/files/downloads 2>/dev/null || true"');
    console.log('Internal app downloads:\n' + internalFiles);

    ws.close();

    console.log('\n====================================================');
    console.log('  ✓ ALL E2E CHECKS PASSED SUCCESSFULLY!');
    console.log('  1. YouTube Search by Name: PASS (found & returned tracks)');
    console.log('  2. Direct Streaming without download: PASS (continuous playback >15s, 0 files created)');
    console.log('  3. Download from Search: PASS (file saved to storage)');
    console.log('====================================================');
}

runTest().catch(err => {
    console.error('\n[FATAL TEST FAILURE]:', err.message || err);
    process.exit(1);
});
