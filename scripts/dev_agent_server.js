#!/usr/bin/env node
const http = require('http');

let pendingResolve = null;
const cmdQueue = [];
let replyPromises = new Map();
let cmdCounter = 1;

const server = http.createServer((req, res) => {
    // CORS headers & connection management
    res.setHeader('Access-Control-Allow-Origin', '*');
    res.setHeader('Access-Control-Allow-Methods', 'GET, POST, OPTIONS');
    res.setHeader('Access-Control-Allow-Headers', '*');
    res.setHeader('Access-Control-Allow-Private-Network', 'true');
    res.setHeader('Connection', 'close'); // Prevent socket pool exhaustion in WebKit

    if (req.method === 'OPTIONS') {
        res.writeHead(204);
        res.end();
        return;
    }

    if (req.url === '/poll' && req.method === 'GET') {
        if (cmdQueue.length > 0) {
            const cmd = cmdQueue.shift();
            console.log(`[DevAgentServer] Dispatching queued ${cmd.cmdId} to /poll`);
            res.writeHead(200, { 'Content-Type': 'application/json' });
            res.end(JSON.stringify(cmd));
        } else {
            pendingResolve = res;
            // Clean server-side timeout after 25s
            const pollTimeout = setTimeout(() => {
                if (pendingResolve === res) {
                    pendingResolve = null;
                    res.writeHead(200, { 'Content-Type': 'application/json' });
                    res.end(JSON.stringify({ empty: true }));
                }
            }, 25000);

            req.on('close', () => {
                clearTimeout(pollTimeout);
                if (pendingResolve === res) pendingResolve = null;
            });
        }
    } else if (req.url === '/reply' && req.method === 'POST') {
        let body = '';
        req.on('data', chunk => body += chunk);
        req.on('end', () => {
            try {
                const data = JSON.parse(body);
                console.log(`[DevAgentServer] Received reply for ${data.cmdId}`);
                res.writeHead(200, { 'Content-Type': 'application/json' });
                res.end(JSON.stringify({ ok: true }));

                const p = replyPromises.get(data.cmdId);
                if (p) {
                    replyPromises.delete(data.cmdId);
                    p(data);
                }
            } catch (err) {
                console.error('[DevAgentServer] Reply parse error:', err);
                res.writeHead(400);
                res.end('invalid json');
            }
        });
    } else if (req.url === '/log' && req.method === 'POST') {
        let body = '';
        req.on('data', chunk => body += chunk);
        req.on('end', () => {
            console.log('[BrowserLog]', body);
            res.writeHead(200);
            res.end('ok');
        });
    } else if (req.url === '/eval' && req.method === 'POST') {
        let body = '';
        req.on('data', chunk => body += chunk);
        req.on('end', () => {
            const code = body.trim();
            const cmdId = 'cmd_' + (cmdCounter++);
            const cmd = { cmdId, code };
            console.log(`[DevAgentServer] Queuing/dispatching eval ${cmdId}`);

            const timeout = setTimeout(() => {
                if (replyPromises.has(cmdId)) {
                    replyPromises.delete(cmdId);
                    const qIdx = cmdQueue.findIndex(c => c.cmdId === cmdId);
                    if (qIdx !== -1) cmdQueue.splice(qIdx, 1);
                    console.log(`[DevAgentServer] ${cmdId} timed out`);
                    res.writeHead(504, { 'Content-Type': 'application/json' });
                    res.end(JSON.stringify({ error: 'Timeout waiting for WebView reply' }));
                }
            }, 45000);

            replyPromises.set(cmdId, (reply) => {
                clearTimeout(timeout);
                res.writeHead(200, { 'Content-Type': 'application/json' });
                res.end(JSON.stringify(reply));
            });

            if (pendingResolve) {
                console.log(`[DevAgentServer] Immediately dispatching ${cmdId} to pending /poll`);
                const r = pendingResolve;
                pendingResolve = null;
                r.writeHead(200, { 'Content-Type': 'application/json' });
                r.end(JSON.stringify(cmd));
            } else {
                console.log(`[DevAgentServer] Pushing ${cmdId} to queue (len was ${cmdQueue.length})`);
                cmdQueue.push(cmd);
            }
        });
    } else if (req.url === '/clear' && req.method === 'POST') {
        cmdQueue.length = 0;
        replyPromises.clear();
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ ok: true }));
    } else if (req.url === '/status') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({
            active: true,
            connected: pendingResolve !== null,
            queueLen: cmdQueue.length,
            pendingReplies: replyPromises.size
        }));
    } else {
        res.writeHead(404);
        res.end();
    }
});

server.listen(8765, '127.0.0.1', () => {
    console.log('[DevAgentServer] Listening on http://127.0.0.1:8765');
});
