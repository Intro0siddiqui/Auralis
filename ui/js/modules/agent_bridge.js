// Lightweight Dev Agent Bridge for local UI/UX inspection and automated control
(function() {
    if (typeof window === 'undefined') return;

    function sendLog(msg) {
        fetch('http://127.0.0.1:8765/log', {
            method: 'POST',
            body: String(msg)
        }).catch(() => {});
    }

    window.__devLog = sendLog;

    let active = true;
    let failCount = 0;
    async function pollLoop() {
        while (active) {
            try {
                const res = await fetch('http://127.0.0.1:8765/poll');
                failCount = 0;
                if (res.ok) {
                    const data = await res.json();
                    if (data && data.cmdId && data.code) {
                        sendLog('Received cmd: ' + data.cmdId);
                        let result = null;
                        let error = null;
                        try {
                            const fn = new Function('window', 'document', `return (async () => { ${data.code} })()`);
                            result = await fn(window, document);
                            sendLog('Finished evaluating cmd: ' + data.cmdId);
                        } catch (err) {
                            error = (err && err.message) ? `${err.name || 'Error'}: ${err.message}` : (String(err) || 'Unknown error');
                            sendLog('Error evaluating cmd: ' + data.cmdId + ' -> ' + error);
                        }

                        // Bulletproof serialization to avoid cyclic/DOM node crashes
                        let safeBody;
                        try {
                            safeBody = JSON.stringify({ cmdId: data.cmdId, result, error });
                        } catch (serErr) {
                            safeBody = JSON.stringify({
                                cmdId: data.cmdId,
                                result: String(result),
                                error: 'SerializationError: ' + serErr.message
                            });
                        }

                        sendLog('Sending reply for cmd: ' + data.cmdId);
                        await fetch('http://127.0.0.1:8765/reply', {
                            method: 'POST',
                            headers: { 'Content-Type': 'application/json' },
                            body: safeBody
                        }).catch((e) => {
                            sendLog('Failed sending reply for cmd: ' + data.cmdId + ' -> ' + e.message);
                        });
                        sendLog('Reply sent for cmd: ' + data.cmdId);
                    }
                }
            } catch (e) {
                // Server offline or not listening, stop after 3 failed attempts
                failCount++;
                if (failCount >= 3) {
                    active = false;
                    break;
                }
                await new Promise(r => setTimeout(r, 2000));
            }
        }
    }

    pollLoop();
})();
