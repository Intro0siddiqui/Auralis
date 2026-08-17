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
if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', () => {
        window.Auralis.bridge.init();
        if (window.Auralis.player && typeof window.Auralis.player.initBridgeListeners === 'function') {
            window.Auralis.player.initBridgeListeners();
        }
    });
} else {
    window.Auralis.bridge.init();
    if (window.Auralis.player && typeof window.Auralis.player.initBridgeListeners === 'function') {
        window.Auralis.player.initBridgeListeners();
    }
}

export { Bridge };
export default window.Auralis.bridge;
