#!/usr/bin/env node

/**
 * End-to-End Android YouTube Download Test
 * -----------------------------------------
 * Connects to Chrome DevTools Protocol (CDP) forwarded from Android WebView (localhost:9222).
 *
 * Test steps evaluated in the WebView:
 *  1. Resolves YouTube audio metadata via `window.AuralisYouTube.resolve(...)`.
 *  2. Initiates native audio download via `window.__TAURI__.core.invoke('download_audio', ...)`.
 *  3. Listens for `download:completed` event (with a 90-second timeout).
 *
 * Exit code 0 on success, exit code 1 on failure or timeout.
 */

const http = require('http');
const crypto = require('crypto');
const { EventEmitter } = require('events');

const CDP_HOST = process.env.CDP_HOST || '127.0.0.1';
const CDP_PORT = parseInt(process.env.CDP_PORT || '9222', 10);
const TEST_YOUTUBE_URL = process.env.TEST_YOUTUBE_URL || process.argv[2] || 'https://www.youtube.com/watch?v=ZYEz2EKwrQ4';
const OVERALL_TIMEOUT_MS = parseInt(process.env.TEST_TIMEOUT_MS || '120000', 10); // 120s overall timeout

/**
 * Minimal zero-dependency RFC 6455 WebSocket client for Node environments
 * where globalThis.WebSocket is unavailable.
 */
class SimpleWebSocketClient extends EventEmitter {
    constructor(wsUrl) {
        super();
        this.url = new URL(wsUrl);
        this.socket = null;
        this._buffer = Buffer.alloc(0);
        this._connect();
    }

    _connect() {
        const key = crypto.randomBytes(16).toString('base64');
        const req = http.request({
            hostname: this.url.hostname,
            port: this.url.port || 80,
            path: this.url.pathname + this.url.search,
            headers: {
                'Upgrade': 'websocket',
                'Connection': 'Upgrade',
                'Sec-WebSocket-Key': key,
                'Sec-WebSocket-Version': '13',
                'Host': this.url.host
            }
        });

        req.on('upgrade', (_res, socket, head) => {
            this.socket = socket;
            if (head && head.length > 0) {
                this._buffer = Buffer.concat([this._buffer, head]);
            }
            socket.on('data', (chunk) => this._onData(chunk));
            socket.on('close', () => this.emit('close'));
            socket.on('error', (err) => this.emit('error', err));
            this.emit('open');
        });

        req.on('error', (err) => this.emit('error', err));
        req.end();
    }

    _onData(chunk) {
        this._buffer = Buffer.concat([this._buffer, chunk]);
        while (this._buffer.length >= 2) {
            const firstByte = this._buffer[0];
            const secondByte = this._buffer[1];
            const opcode = firstByte & 0x0f;
            let payloadLen = secondByte & 0x7f;
            let offset = 2;

            if (payloadLen === 126) {
                if (this._buffer.length < 4) return;
                payloadLen = this._buffer.readUInt16BE(2);
                offset = 4;
            } else if (payloadLen === 127) {
                if (this._buffer.length < 10) return;
                const high = this._buffer.readUInt32BE(2);
                const low = this._buffer.readUInt32BE(6);
                payloadLen = high * 4294967296 + low;
                offset = 10;
            }

            if (this._buffer.length < offset + payloadLen) return;

            const payload = this._buffer.subarray(offset, offset + payloadLen);
            this._buffer = this._buffer.subarray(offset + payloadLen);

            if (opcode === 0x1) {
                // Text frame
                this.emit('message', payload.toString('utf8'));
            } else if (opcode === 0x8) {
                // Close frame
                this.close();
            } else if (opcode === 0x9) {
                // Ping -> respond with Pong
                this._sendPong(payload);
            }
        }
    }

    send(data) {
        if (!this.socket) throw new Error('Socket not connected');
        const payload = Buffer.from(data, 'utf8');
        const mask = crypto.randomBytes(4);
        let header;
        if (payload.length < 126) {
            header = Buffer.alloc(6);
            header[0] = 0x81; // FIN + text
            header[1] = 0x80 | payload.length;
            mask.copy(header, 2);
        } else if (payload.length <= 65535) {
            header = Buffer.alloc(8);
            header[0] = 0x81;
            header[1] = 0x80 | 126;
            header.writeUInt16BE(payload.length, 2);
            mask.copy(header, 4);
        } else {
            header = Buffer.alloc(14);
            header[0] = 0x81;
            header[1] = 0x80 | 127;
            header.writeUInt32BE(0, 2);
            header.writeUInt32BE(payload.length, 6);
            mask.copy(header, 10);
        }
        const masked = Buffer.alloc(payload.length);
        for (let i = 0; i < payload.length; i++) {
            masked[i] = payload[i] ^ mask[i % 4];
        }
        this.socket.write(Buffer.concat([header, masked]));
    }

    _sendPong(_payload) {
        if (!this.socket) return;
        const header = Buffer.from([0x8a, 0x00]);
        this.socket.write(header);
    }

    close() {
        if (this.socket) {
            try {
                this.socket.write(Buffer.from([0x88, 0x00]));
                this.socket.end();
            } catch (_) {}
        }
    }
}

/**
 * Creates a WebSocket connection using global WebSocket or the fallback client.
 */
function createWebSocket(wsUrl) {
    if (typeof globalThis.WebSocket === 'function') {
        const ws = new globalThis.WebSocket(wsUrl);
        const emitter = new EventEmitter();
        ws.onopen = () => emitter.emit('open');
        ws.onmessage = (event) => emitter.emit('message', event.data);
        ws.onerror = (err) => emitter.emit('error', err);
        ws.onclose = () => emitter.emit('close');
        emitter.send = (data) => ws.send(data);
        emitter.close = () => ws.close();
        return emitter;
    }
    return new SimpleWebSocketClient(wsUrl);
}

/**
 * Perform an HTTP GET request and parse JSON response.
 */
function fetchJson(url) {
    return new Promise((resolve, reject) => {
        const req = http.get(url, { timeout: 3000 }, (res) => {
            if (res.statusCode !== 200) {
                res.resume();
                return reject(new Error(`HTTP ${res.statusCode} from ${url}`));
            }
            let data = '';
            res.setEncoding('utf8');
            res.on('data', (chunk) => { data += chunk; });
            res.on('end', () => {
                try {
                    resolve(JSON.parse(data));
                } catch (e) {
                    reject(new Error(`Invalid JSON from ${url}: ${e.message}`));
                }
            });
        });
        req.on('error', reject);
        req.on('timeout', () => {
            req.destroy();
            reject(new Error(`Timeout fetching ${url}`));
        });
    });
}

/**
 * Poll http://127.0.0.1:9222/json until a debuggable WebView page target is found.
 */
async function discoverTarget(host, port, maxRetries = 45, intervalMs = 1000) {
    const listUrl = `http://${host}:${port}/json`;
    console.log(`[CDP] Discovering WebView targets at ${listUrl}...`);

    for (let attempt = 1; attempt <= maxRetries; attempt++) {
        try {
            const targets = await fetchJson(listUrl);
            if (Array.isArray(targets) && targets.length > 0) {
                // Find page target with webSocketDebuggerUrl
                const pageTarget = targets.find(t => t.webSocketDebuggerUrl && (t.type === 'page' || !t.type))
                    || targets.find(t => t.webSocketDebuggerUrl);

                if (pageTarget && pageTarget.webSocketDebuggerUrl) {
                    let wsUrl = pageTarget.webSocketDebuggerUrl;
                    // Normalize host if DevTools returned an abstract or local URL
                    try {
                        const parsed = new URL(wsUrl);
                        parsed.hostname = host;
                        parsed.port = String(port);
                        wsUrl = parsed.toString();
                    } catch (_) {}

                    console.log(`[CDP] Found target "${pageTarget.title || pageTarget.url}" (id: ${pageTarget.id})`);
                    console.log(`[CDP] WebSocket URL: ${wsUrl}`);
                    return { target: pageTarget, wsUrl };
                }
            }
        } catch (err) {
            // Target not ready yet
        }
        await new Promise(r => setTimeout(r, intervalMs));
    }

    throw new Error(`Failed to discover WebView target on ${listUrl} after ${maxRetries} seconds`);
}

/**
 * Chrome DevTools Protocol Client helper.
 */
class CdpSession {
    constructor(ws) {
        this.ws = ws;
        this.nextId = 1;
        this.pendingRequests = new Map();
        this._setupListeners();
    }

    _setupListeners() {
        this.ws.on('message', (raw) => {
            let msg;
            try {
                msg = JSON.parse(raw);
            } catch (_) {
                return;
            }

            // Stream console logs from WebView
            if (msg.method === 'Runtime.consoleAPICalled' && msg.params) {
                const type = msg.params.type || 'log';
                const text = (msg.params.args || [])
                    .map(a => a.value !== undefined ? (typeof a.value === 'object' ? JSON.stringify(a.value) : a.value) : (a.description || ''))
                    .join(' ');
                console.log(`[WebView Console ${type.toUpperCase()}] ${text}`);
            } else if (msg.method === 'Console.messageAdded' && msg.params?.message) {
                const m = msg.params.message;
                console.log(`[WebView Console ${m.level?.toUpperCase() || 'LOG'}] ${m.text}`);
            }

            // Handle command responses
            if (msg.id && this.pendingRequests.has(msg.id)) {
                const { resolve, reject } = this.pendingRequests.get(msg.id);
                this.pendingRequests.delete(msg.id);

                if (msg.error) {
                    reject(new Error(`CDP error (${msg.id}): ${msg.error.message || JSON.stringify(msg.error)}`));
                } else {
                    resolve(msg.result);
                }
            }
        });
    }

    send(method, params = {}) {
        const id = this.nextId++;
        const payload = JSON.stringify({ id, method, params });
        return new Promise((resolve, reject) => {
            this.pendingRequests.set(id, { resolve, reject });
            this.ws.send(payload);
        });
    }

    async evaluate(expression) {
        const result = await this.send('Runtime.evaluate', {
            expression,
            awaitPromise: true,
            returnByValue: true,
            userGesture: true
        });

        if (result.exceptionDetails) {
            const desc = result.exceptionDetails.exception?.description ||
                         result.exceptionDetails.text ||
                         JSON.stringify(result.exceptionDetails);
            throw new Error(`WebView Evaluation Exception: ${desc}`);
        }

        return result.result?.value;
    }
}

/**
 * Builds the JavaScript expression that will execute in the WebView.
 */
function buildInPageTestExpression(testUrl) {
    return `
    (async () => {
        const log = (...args) => console.log('[E2E-InPage]', ...args);
        const warn = (...args) => console.warn('[E2E-InPage]', ...args);
        const errLog = (...args) => console.error('[E2E-InPage]', ...args);

        log('Starting Android YouTube download E2E test in WebView...');
        log('Target URL:', ${JSON.stringify(testUrl)});

        // 1. Wait for window.AuralisYouTube and Tauri bridge to be initialized
        const waitStart = Date.now();
        while (Date.now() - waitStart < 30000) {
            const hasResolver = Boolean(window.AuralisYouTube && typeof window.AuralisYouTube.resolve === 'function');
            const hasInvoke = Boolean(
                (window.__TAURI__?.core?.invoke) ||
                (window.__TAURI_INTERNALS__?.invoke) ||
                (window.Auralis?.bridge?.invoke)
            );
            if (hasResolver && hasInvoke) break;
            await new Promise(r => setTimeout(r, 250));
        }

        if (!window.AuralisYouTube || typeof window.AuralisYouTube.resolve !== 'function') {
            throw new Error('window.AuralisYouTube resolver not available after 30 seconds');
        }

        const invoke = (window.__TAURI__?.core?.invoke)
            ? window.__TAURI__.core.invoke.bind(window.__TAURI__.core)
            : (window.Auralis?.bridge?.invoke)
            ? window.Auralis.bridge.invoke.bind(window.Auralis.bridge)
            : (window.__TAURI_INTERNALS__?.invoke);

        if (!invoke) {
            throw new Error('Tauri invoke function not available in WebView window context');
        }

        const tauriListen = (window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.tauri && window.__TAURI_INTERNALS__.tauri.listen)
            || (window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.event && window.__TAURI_INTERNALS__.event.listen)
            || (window.__TAURI__ && window.__TAURI__.event && window.__TAURI__.event.listen)
            || (window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.listen);

        // 2. Resolve YouTube stream metadata
        log('Step 1/3: Resolving YouTube video stream...');
        const resolved = await window.AuralisYouTube.resolve(${JSON.stringify(testUrl)});
        log('Resolved track details:', JSON.stringify({
            kind: resolved.kind,
            title: resolved.title,
            ext: resolved.ext,
            platform: resolved.platform,
            total_bytes: resolved.total_bytes,
            has_stream_url: Boolean(resolved.stream_url)
        }));

        if (!resolved || !resolved.stream_url) {
            throw new Error('Failed to resolve audio stream URL from YouTube');
        }

        // 3. Set up event listener for download:completed before starting download
        log('Step 2/3: Registering download:completed event listener...');
        let unlistenFn = null;
        let completedPayload = null;

        const downloadCompletionPromise = new Promise((resolve, reject) => {
            const timeoutTimer = setTimeout(() => {
                if (typeof unlistenFn === 'function') unlistenFn();
                reject(new Error('Download timeout exceeded (90 seconds waiting for download:completed event)'));
            }, 90000);

            const handleCompleted = (eventData) => {
                const payload = (eventData && eventData.payload) ? eventData.payload : eventData;
                log('download:completed event received:', JSON.stringify(payload));
                completedPayload = payload;

                if (!payload || payload.status === 'completed') {
                    clearTimeout(timeoutTimer);
                    if (typeof unlistenFn === 'function') unlistenFn();
                    resolve(payload || { status: 'completed' });
                } else if (payload.status === 'failed') {
                    clearTimeout(timeoutTimer);
                    if (typeof unlistenFn === 'function') unlistenFn();
                    reject(new Error('Download failed: ' + (payload.error_message || 'unknown error')));
                }
            };

            // Register with Auralis bridge if available
            if (window.Auralis?.bridge && typeof window.Auralis.bridge.on === 'function') {
                window.Auralis.bridge.on('download:completed', handleCompleted);
            }

            // Register with Tauri event system
            if (tauriListen) {
                tauriListen('download:completed', handleCompleted)
                    .then(u => { unlistenFn = u; })
                    .catch(err => warn('Failed to attach Tauri event listener:', err));
            }
        });

        // 4. Invoke download_audio command
        log('Step 3/3: Invoking download_audio command in Rust backend...');
        const downloadRequest = {
            url: resolved.stream_url,
            title: resolved.title || 'YouTube Test Track',
            platform: resolved.platform || 'youtube',
            format: resolved.ext || 'm4a',
            ext: resolved.ext || 'm4a',
            thumbnail: resolved.thumbnail || null
        };

        const startResult = await invoke('download_audio', { request: downloadRequest });
        log('download_audio invocation returned:', JSON.stringify(startResult));
        const downloadId = startResult?.id;

        // Periodic polling safety check in case event was emitted before listener was bound
        const pollTimer = setInterval(async () => {
            if (!downloadId) return;
            try {
                const progress = await invoke('get_download_progress', { id: downloadId });
                if (progress) {
                    log('Polled download progress:', progress.status, progress.bytes_downloaded, '/', progress.total_bytes);
                    if (progress.status === 'completed' && !completedPayload) {
                        clearInterval(pollTimer);
                    }
                }
            } catch (_) {}
        }, 3000);

        try {
            const finalResult = await downloadCompletionPromise;
            clearInterval(pollTimer);
            log('E2E download test succeeded in WebView context!');
            return {
                success: true,
                title: resolved.title,
                ext: resolved.ext,
                downloadId: downloadId,
                finalResult
            };
        } catch (err) {
            clearInterval(pollTimer);
            errLog('E2E download test failed in WebView:', err?.message || err);
            throw err;
        }
    })()
    `;
}

/**
 * Main execution flow.
 */
async function run() {
    console.log('====================================================');
    console.log('  Auralis Android YouTube Download E2E Test (CDP)   ');
    console.log('====================================================');
    console.log(`CDP Endpoint: http://${CDP_HOST}:${CDP_PORT}`);
    console.log(`Test YouTube URL: ${TEST_YOUTUBE_URL}`);
    console.log(`Overall Timeout: ${OVERALL_TIMEOUT_MS / 1000}s`);
    console.log('----------------------------------------------------');

    const overallTimeout = setTimeout(() => {
        console.error(`\n[FATAL] Overall test execution timeout exceeded (${OVERALL_TIMEOUT_MS / 1000}s)`);
        process.exit(1);
    }, OVERALL_TIMEOUT_MS);

    try {
        // 1. Discover DevTools target
        const { target, wsUrl } = await discoverTarget(CDP_HOST, CDP_PORT);

        // 2. Connect WebSocket
        console.log(`[CDP] Connecting WebSocket to ${wsUrl}...`);
        const ws = createWebSocket(wsUrl);

        await new Promise((resolve, reject) => {
            const connectTimer = setTimeout(() => reject(new Error('WebSocket connection timeout')), 10000);
            ws.on('open', () => {
                clearTimeout(connectTimer);
                resolve();
            });
            ws.on('error', (err) => {
                clearTimeout(connectTimer);
                reject(err);
            });
        });

        console.log('[CDP] WebSocket connected successfully.');
        const session = new CdpSession(ws);

        // 3. Enable CDP domains
        console.log('[CDP] Enabling Runtime, Page, and Console domains...');
        await session.send('Runtime.enable');
        await session.send('Page.enable');
        await session.send('Console.enable');

        // 4. Evaluate E2E test in WebView
        console.log('[CDP] Evaluating E2E download test expression in WebView...');
        const expr = buildInPageTestExpression(TEST_YOUTUBE_URL);
        const result = await session.evaluate(expr);

        console.log('----------------------------------------------------');
        console.log('[CDP] Test execution returned value:', JSON.stringify(result, null, 2));
        console.log('====================================================');
        console.log('  ✓ E2E Android YouTube Download Test PASSED!       ');
        console.log('====================================================');

        clearTimeout(overallTimeout);
        ws.close();
        process.exit(0);
    } catch (err) {
        console.error('----------------------------------------------------');
        console.error('[FATAL] E2E Download Test FAILED:', err.message || err);
        console.error('====================================================');
        console.error('  ✗ E2E Android YouTube Download Test FAILED        ');
        console.error('====================================================');

        clearTimeout(overallTimeout);
        process.exit(1);
    }
}

run();
