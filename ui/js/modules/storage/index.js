/**
 * Storage Adapter Selector & Platform Detection
 * Dispatches storage operations to platform-specific adapters (desktop vs android).
 */

import { desktopStorageAdapter } from './desktop.js';
import { androidStorageAdapter } from './android.js';

/**
 * Detect if running under an Android environment (Android WebView, Mobile userAgent, or Tauri Android window).
 * @returns {boolean}
 */
export function isAndroid() {
    if (typeof navigator !== 'undefined' && navigator.userAgent) {
        if (/Android/i.test(navigator.userAgent)) return true;
    }
    // Mobile Tauri window detection: on Android WebView, window label is often absent or empty
    if (
        typeof window !== 'undefined' &&
        window.__TAURI_METADATA__ &&
        window.__TAURI_METADATA__.__currentWindow &&
        !window.__TAURI_METADATA__.__currentWindow.label
    ) {
        return true;
    }
    return false;
}

/**
 * Detect if running on any mobile device (Android, iOS).
 * @returns {boolean}
 */
export function isMobile() {
    if (typeof navigator !== 'undefined' && navigator.userAgent) {
        if (/Android|iPhone|iPad|iPod/i.test(navigator.userAgent)) return true;
    }
    return isAndroid();
}

/**
 * Detect if running on desktop (Linux, Windows, macOS).
 * @returns {boolean}
 */
export function isDesktop() {
    return !isMobile();
}

let _testAdapter = null;

/**
 * Return the active platform storage adapter.
 * @returns {typeof desktopStorageAdapter | typeof androidStorageAdapter}
 */
export function getStorageAdapter() {
    if (_testAdapter) {
        return _testAdapter;
    }
    if (isAndroid() || isMobile()) {
        return androidStorageAdapter;
    }
    return desktopStorageAdapter;
}

/**
 * Set an adapter override for testing purposes.
 * @param {object|null} adapter
 */
export function setStorageAdapterForTesting(adapter) {
    _testAdapter = adapter;
}

/**
 * Reset test adapter override.
 */
export function resetStorageAdapterForTesting() {
    _testAdapter = null;
}

export { desktopStorageAdapter, androidStorageAdapter };
