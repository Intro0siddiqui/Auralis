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
import { uiMethods } from './modules/ui.js';

// Compose all module methods onto Bridge prototype
Object.assign(
    Bridge.prototype,
    coreMethods,
    libraryMethods,
    scanUiMethods,
    viewMethods,
    playerMethods,
    downloadMethods,
    uiMethods
);

// Preserve backward-compatible globals and instance immediately
window.Auralis = window.Auralis || {};
window.Auralis.Bridge = Bridge;
window.Auralis.bridge = new Bridge();
window.Auralis.assetUrl = (path) => window.Auralis.bridge.assetUrl(path);

// Auto-initialize
function _wirePlayer() {
    const p = window.Auralis && window.Auralis.player;
    if (p && typeof p.initBridgeListeners === 'function') p.initBridgeListeners();
    if (p && typeof p.hydrateState === 'function') p.hydrateState().catch(()=>{});
}

function _wireRamSyncListeners() {
    window.Auralis.bridge.on('sync:track_received_in_ram', (event) => {
        const payload = event.payload || event;
        console.log('[Auralis] Track received in RAM:', payload);
        if (window.Auralis.showToast) {
            window.Auralis.showToast(`Received track "${payload.title}" in RAM. Instant playback ready.`);
        }
    });
}

if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', () => {
        window.Auralis.bridge.init();
        _wirePlayer();
        _wireRamSyncListeners();
        // Retry shortly for case where player.js loaded after this module
        setTimeout(_wirePlayer, 300);
    });
} else {
    window.Auralis.bridge.init();
    _wirePlayer();
    _wireRamSyncListeners();
    setTimeout(_wirePlayer, 300);
}

export { Bridge };
export default window.Auralis.bridge;
