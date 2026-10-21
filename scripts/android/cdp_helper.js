const http = require('http');
const crypto = require('crypto');
const { EventEmitter } = require('events');

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
            if (head && head.length > 0) this._buffer = Buffer.concat([this._buffer, head]);
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
            const isFinal = (firstByte & 0x80) !== 0;
            const opcode = firstByte & 0x0f;
            const isMasked = (secondByte & 0x80) !== 0;
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
            let maskKey = null;
            if (isMasked) {
                if (this._buffer.length < offset + 4) return;
                maskKey = this._buffer.subarray(offset, offset + 4);
                offset += 4;
            }
            if (this._buffer.length < offset + payloadLen) return;
            let payload = this._buffer.subarray(offset, offset + payloadLen);
            this._buffer = this._buffer.subarray(offset + payloadLen);
            if (isMasked && maskKey) {
                const unmasked = Buffer.alloc(payload.length);
                for (let i = 0; i < payload.length; i++) unmasked[i] = payload[i] ^ maskKey[i % 4];
                payload = unmasked;
            }
            if (opcode === 0x0) {
                if (this._fragBuffer) {
                    this._fragBuffer = Buffer.concat([this._fragBuffer, payload]);
                    if (isFinal) {
                        const complete = this._fragBuffer;
                        this._fragBuffer = null;
                        this.emit('message', complete.toString('utf8'));
                    }
                }
            } else if (opcode === 0x1 || opcode === 0x2) {
                if (!isFinal) {
                    this._fragBuffer = payload;
                } else {
                    this.emit('message', payload.toString('utf8'));
                }
            } else if (opcode === 0x8) {
                this.close();
            } else if (opcode === 0x9) {
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
            header[0] = 0x81;
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
        for (let i = 0; i < payload.length; i++) masked[i] = payload[i] ^ mask[i % 4];
        this.socket.write(Buffer.concat([header, masked]));
    }
    _sendPong(_payload) {
        if (!this.socket) return;
        this.socket.write(Buffer.from([0x8a, 0x00]));
    }
    close() {
        if (this.socket) {
            try { this.socket.write(Buffer.from([0x88, 0x00])); this.socket.end(); } catch (_) {}
        }
    }
}

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
            try { msg = JSON.parse(raw); } catch (_) { return; }
            if (msg.method === 'Runtime.consoleAPICalled' && msg.params) {
                const text = (msg.params.args || [])
                    .map(a => a.value !== undefined ? (typeof a.value === 'object' ? JSON.stringify(a.value) : a.value) : (a.description || ''))
                    .join(' ');
                console.log(`[WebView Console] ${text}`);
            }
            if (msg.id && this.pendingRequests.has(msg.id)) {
                const { resolve, reject } = this.pendingRequests.get(msg.id);
                this.pendingRequests.delete(msg.id);
                if (msg.error) reject(new Error(`CDP error: ${JSON.stringify(msg.error)}`));
                else resolve(msg.result);
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
            throw new Error(`Evaluation Exception: ${desc}`);
        }
        return result.result?.value;
    }
}

function fetchJson(url) {
    return new Promise((resolve, reject) => {
        const req = http.get(url, { timeout: 3000 }, (res) => {
            if (res.statusCode !== 200) {
                res.resume();
                return reject(new Error(`HTTP ${res.statusCode} from ${url}`));
            }
            let data = '';
            res.setEncoding('utf8');
            res.on('data', (c) => data += c);
            res.on('end', () => {
                try { resolve(JSON.parse(data)); } catch (e) { reject(e); }
            });
        });
        req.on('error', reject);
    });
}

async function connectToCdp(host = '127.0.0.1', port = 9222) {
    const targets = await fetchJson(`http://${host}:${port}/json`);
    const page = targets.find(t => t.webSocketDebuggerUrl);
    if (!page) throw new Error('No WebView page target found on port ' + port);
    const ws = new SimpleWebSocketClient(page.webSocketDebuggerUrl);
    await new Promise((resolve, reject) => {
        ws.on('open', resolve);
        ws.on('error', reject);
    });
    const cdp = new CdpSession(ws);
    await cdp.send('Runtime.enable');
    await cdp.send('Console.enable');
    return { cdp, ws };
}

module.exports = {
    SimpleWebSocketClient,
    CdpSession,
    fetchJson,
    connectToCdp
};
