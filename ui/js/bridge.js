/**
 * Auralis Bridge Entry Point
 * Composes ES modules onto the Bridge prototype and exposes window.Auralis.bridge.
 */

import { Bridge, coreMethods } from './modules/core.js';
import { libraryMethods } from './modules/library.js';
import { scanUiMethods } from './modules/scan-ui.js';
import { viewMethods } from './modules/views.js';
import { playerMethods } from './modules/player.js';
import { downloadMethods } from './modules/downloads.js';
import { syncMethods } from './modules/sync.js';
import { uiMethods } from './modules/ui.js';
import { copyText, copyWithToast } from './modules/clipboard.js';
import { router } from './modules/router.js';

// Compose all module methods onto Bridge prototype
Object.assign(
    Bridge.prototype,
    { copyText, copyWithToast },
    coreMethods,
    libraryMethods,
    scanUiMethods,
    viewMethods,
    playerMethods,
    downloadMethods,
    syncMethods,
    uiMethods
);

// Preserve backward-compatible globals and instance immediately
window.Auralis = window.Auralis || {};
window.Auralis.Bridge = Bridge;
window.Auralis.bridge = new Bridge();
window.Auralis.router = router;
window.Auralis.bridge.navigateTo = (viewName) => router.navigate(viewName);
Bridge.prototype.navigateTo = (viewName) => router.navigate(viewName);
window.Auralis.assetUrl = (path) => window.Auralis.bridge.assetUrl(path);

// Auto-initialize
function _wirePlayer() {
    const p = window.Auralis && window.Auralis.player;
    if (p && typeof p.initBridgeListeners === 'function') p.initBridgeListeners();
    if (p && typeof p.hydrateState === 'function') p.hydrateState().catch(()=>{});
}

function _initApp() {
    window.Auralis.bridge.init();
    router.init();
    _wirePlayer();
    // Retry shortly for case where player.js loaded after this module
    setTimeout(_wirePlayer, 300);
}

if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', () => {
        _initApp();
    });
} else {
    _initApp();
}

export { Bridge, router };
export default window.Auralis.bridge;
