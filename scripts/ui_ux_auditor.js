#!/usr/bin/env node
/**
 * Automated UI/UX Live WebView Auditor for Auralis v2
 * Connects directly to the running application via DevAgentServer on http://127.0.0.1:8765
 */
const http = require('http');

function evalJs(code) {
    return new Promise((resolve, reject) => {
        const req = http.request({
            hostname: '127.0.0.1',
            port: 8765,
            path: '/eval',
            method: 'POST',
            headers: { 'Content-Type': 'text/plain' }
        }, (res) => {
            let data = '';
            res.on('data', chunk => data += chunk);
            res.on('end', () => {
                try {
                    const parsed = JSON.parse(data);
                    if (parsed.error) reject(new Error(parsed.error));
                    else resolve(parsed.result);
                } catch (e) {
                    reject(e);
                }
            });
        });
        req.on('error', reject);
        req.write(code);
        req.end();
    });
}

const sleep = ms => new Promise(r => setTimeout(r, ms));

async function main() {
    console.log('====================================================');
    console.log('🚀 AURALIS LIVE WEBVIEW UI/UX AUDITOR INITIALIZED');
    console.log('====================================================\n');

    // Setup global error collector in WebView
    await evalJs(`
        window.__capturedErrors = window.__capturedErrors || [];
        window.addEventListener('error', e => window.__capturedErrors.push({ type: 'error', msg: e.message, src: e.filename, line: e.lineno }));
        window.addEventListener('unhandledrejection', e => window.__capturedErrors.push({ type: 'unhandledrejection', msg: String(e.reason) }));
        return true;
    `);

    const allViews = ['home', 'library', 'albums', 'artists', 'playlists', 'search', 'download', 'sync', 'settings'];
    const views = process.argv[2] ? [process.argv[2]] : allViews;
    const auditResults = {
        viewsChecked: [],
        overflowIssues: [],
        touchTargetIssues: [],
        brokenImages: [],
        contrastIssues: [],
        modalIssues: [],
        runtimeErrors: []
    };

    for (const v of views) {
        process.stdout.write(`🔍 Auditing View: [${v}] ... `);

        // Run atomic navigation & inspection on target view
        const inspection = await evalJs(`
            try {
                await window.Auralis.router.navigate('${v}');
                await new Promise(r => setTimeout(r, 400));

                const pane = document.getElementById('view-${v}') || document.querySelector('.view-pane.active') || document.getElementById('content');
                if (!pane) return { error: 'Active view pane not found' };

                // 1. Check for text truncation / unintended horizontal scroll overflow
                const overflows = [];
                const textNodes = Array.from(pane.querySelectorAll('h1, h2, h3, h4, p, span, .track-title, .track-artist, .card-title, .btn')).slice(0, 50);
                for (const el of textNodes) {
                    if (el.scrollWidth > el.clientWidth + 2 && el.clientWidth > 0) {
                        const style = window.getComputedStyle(el);
                        if (style.textOverflow !== 'ellipsis' && style.overflow !== 'hidden' && style.whiteSpace !== 'nowrap') {
                            overflows.push({
                                tag: el.tagName,
                                className: String(el.className || ''),
                                text: (el.textContent || '').trim().slice(0, 35),
                                scrollWidth: el.scrollWidth,
                                clientWidth: el.clientWidth
                            });
                        }
                    }
                }

                // 2. Check small touch targets on interactive controls
                const smallTargets = [];
                const clickables = Array.from(pane.querySelectorAll('button, a, input, select, [data-nav], [data-nav-modal]')).slice(0, 50);
                for (const el of clickables) {
                    const rect = el.getBoundingClientRect();
                    if (rect.width === 0 || rect.height === 0) continue; // hidden
                    if (rect.width < 28 || rect.height < 28) {
                        smallTargets.push({
                            tag: el.tagName,
                            className: String(el.className || ''),
                            text: (el.textContent || el.getAttribute('title') || el.getAttribute('aria-label') || '').trim().slice(0, 30),
                            width: Math.round(rect.width),
                            height: Math.round(rect.height)
                        });
                    }
                }

                // 3. Check broken images (excluding embedded data: URIs)
                const brokenImgs = [];
                const imgs = Array.from(pane.querySelectorAll('img')).slice(0, 20);
                for (const img of imgs) {
                    const src = img.getAttribute('src') || '';
                    if (src.startsWith('data:')) continue;
                    if (img.complete && img.naturalWidth === 0) {
                        brokenImgs.push({
                            src: src.slice(0, 80),
                            className: String(img.className || '')
                        });
                    }
                }

                return {
                    view: '${v}',
                    visible: pane.style.display !== 'none',
                    elementCount: textNodes.length,
                    overflowCount: overflows.length,
                    overflows: overflows.slice(0, 5),
                    smallTargetCount: smallTargets.length,
                    smallTargets: smallTargets.slice(0, 5),
                    brokenImages: brokenImgs
                };
            } catch (err) {
                return {
                    view: '${v}',
                    error: err.message,
                    stack: err.stack
                };
            }
        `);

        console.log(`OK (Elements: ${inspection.elementCount}, Overflows: ${inspection.overflowCount}, SmallTargets: ${inspection.smallTargetCount})`);
        auditResults.viewsChecked.push(inspection);
        if (inspection.overflows && inspection.overflows.length > 0) {
            auditResults.overflowIssues.push({ view: v, items: inspection.overflows });
        }
        if (inspection.smallTargets && inspection.smallTargets.length > 0) {
            auditResults.touchTargetIssues.push({ view: v, items: inspection.smallTargets });
        }
        if (inspection.brokenImages && inspection.brokenImages.length > 0) {
            auditResults.brokenImages.push({ view: v, items: inspection.brokenImages });
        }
        await sleep(300);
    }

    // Audit Modals & FullScreen Player
    console.log('\n🔍 Auditing FullScreen Player Modal ...');
    const playerModalRes = await evalJs(`
        if (window.Auralis && window.Auralis.bridge && window.Auralis.bridge.openFullScreenPlayer) {
            window.Auralis.bridge.openFullScreenPlayer();
            const modal = document.getElementById('player-full');
            const overlay = document.getElementById('overlay-root');
            const rect = modal ? modal.getBoundingClientRect() : null;
            return {
                opened: Boolean(modal),
                overlayVisible: overlay ? window.getComputedStyle(overlay).display !== 'none' : false,
                modalWidth: rect ? rect.width : 0,
                modalHeight: rect ? rect.height : 0
            };
        }
        return { error: 'openFullScreenPlayer not found' };
    `);
    auditResults.modalIssues.push({ modal: 'player-full', result: playerModalRes });
    console.log(`FullScreen Player: Opened=${playerModalRes.opened}, OverlayVisible=${playerModalRes.overlayVisible}, Dimensions=${playerModalRes.modalWidth}x${playerModalRes.modalHeight}`);

    // Close modal
    await evalJs(`
        const overlay = document.getElementById('overlay-root');
        if (overlay) overlay.innerHTML = '';
        return true;
    `);

    // Check collected runtime errors
    const errors = await evalJs(`return window.__capturedErrors || [];`);
    auditResults.runtimeErrors = errors;

    console.log('\n====================================================');
    console.log('📊 AUDIT SUMMARY REPORT');
    console.log('====================================================');
    console.log(`Total Views Audited: ${auditResults.viewsChecked.length}`);
    console.log(`Runtime JS Exceptions: ${errors.length}`);
    if (errors.length > 0) {
        console.log('🚨 Runtime Errors:', JSON.stringify(errors, null, 2));
    }
    console.log(`Views with Text Overflows: ${auditResults.overflowIssues.length}`);
    for (const o of auditResults.overflowIssues) {
        console.log(`  - [${o.view}]: ${JSON.stringify(o.items)}`);
    }
    console.log(`Views with Small Interactive Targets (<28px): ${auditResults.touchTargetIssues.length}`);
    for (const t of auditResults.touchTargetIssues) {
        console.log(`  - [${t.view}]: ${t.items.map(i => i.text || i.className || i.tag).join(', ')}`);
    }
    console.log(`Broken Images: ${auditResults.brokenImages.length}`);
    for (const b of auditResults.brokenImages) {
        console.log(`  - [${b.view}]: ${JSON.stringify(b.items)}`);
    }

    console.log('\nAudit complete.');
}

main().catch(err => {
    console.error('Audit failed:', err);
    process.exit(1);
});
