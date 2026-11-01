import { test, describe, beforeEach, afterEach } from 'node:test';
import assert from 'node:assert/strict';

import {
    isAndroid,
    isMobile,
    isDesktop,
    getStorageAdapter,
    setStorageAdapterForTesting,
    resetStorageAdapterForTesting,
    desktopStorageAdapter,
    androidStorageAdapter
} from '../../ui/js/modules/storage/index.js';
import { libraryMethods } from '../../ui/js/modules/library.js';

function setNavigatorUserAgent(ua) {
    Object.defineProperty(globalThis, 'navigator', {
        value: { userAgent: ua },
        configurable: true,
        writable: true
    });
}

describe('Storage Platform Detection & Adapter Selection', () => {
    const originalNavigatorDesc = Object.getOwnPropertyDescriptor(globalThis, 'navigator');
    const originalWindow = globalThis.window;
    const originalDocument = globalThis.document;

    afterEach(() => {
        resetStorageAdapterForTesting();
        if (originalNavigatorDesc) {
            Object.defineProperty(globalThis, 'navigator', originalNavigatorDesc);
        } else {
            delete globalThis.navigator;
        }

        if (originalWindow === undefined) delete globalThis.window;
        else globalThis.window = originalWindow;

        if (originalDocument === undefined) delete globalThis.document;
        else globalThis.document = originalDocument;
    });

    test('isAndroid returns true on Android userAgent', () => {
        setNavigatorUserAgent('Mozilla/5.0 (Linux; Android 15; Pixel 8) AppleWebKit/537.36');
        assert.equal(isAndroid(), true);
        assert.equal(isMobile(), true);
        assert.equal(isDesktop(), false);
        assert.equal(getStorageAdapter(), androidStorageAdapter);
    });

    test('isAndroid returns true on Tauri mobile window metadata without window label', () => {
        setNavigatorUserAgent('Mozilla/5.0 (Linux; U; Android)');
        globalThis.window = {
            __TAURI_METADATA__: {
                __currentWindow: { label: '' }
            }
        };
        assert.equal(isAndroid(), true);
        assert.equal(isMobile(), true);
        assert.equal(isDesktop(), false);
    });

    test('isDesktop returns true on Linux/macOS/Windows desktop userAgent', () => {
        setNavigatorUserAgent('Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36');
        globalThis.window = {
            __TAURI_METADATA__: {
                __currentWindow: { label: 'main' }
            }
        };
        assert.equal(isAndroid(), false);
        assert.equal(isMobile(), false);
        assert.equal(isDesktop(), true);
        assert.equal(getStorageAdapter(), desktopStorageAdapter);
    });

    test('setStorageAdapterForTesting overrides active adapter', () => {
        const mockAdapter = { name: 'mock' };
        setStorageAdapterForTesting(mockAdapter);
        assert.equal(getStorageAdapter(), mockAdapter);
        resetStorageAdapterForTesting();
        assert.notEqual(getStorageAdapter(), mockAdapter);
    });
});

describe('Desktop Storage Adapter', () => {
    test('triggerFolderScan calls pick_folder_and_scan and updates view', async () => {
        const calls = [];
        const mockCtx = {
            appendScanLog: (msg) => calls.push({ type: 'log', msg }),
            showToast: (msg, level) => calls.push({ type: 'toast', msg, level }),
            loadLibraryView: async () => calls.push({ type: 'loadLibraryView' }),
            refreshCurrentView: () => calls.push({ type: 'refreshCurrentView' }),
            invoke: async (cmd, args) => {
                calls.push({ type: 'invoke', cmd, args });
                if (cmd === 'pick_folder_and_scan') {
                    return { tracks_added: 5, tracks_updated: 2, errors: [] };
                }
                return null;
            }
        };

        const res = await desktopStorageAdapter.triggerFolderScan(mockCtx);
        assert.deepEqual(res, { tracks_added: 5, tracks_updated: 2, errors: [] });
        assert.ok(calls.some(c => c.type === 'invoke' && c.cmd === 'pick_folder_and_scan'));
        assert.ok(calls.some(c => c.type === 'toast' && c.level === 'success' && c.msg.includes('5 added')));
        assert.ok(calls.some(c => c.type === 'loadLibraryView'));
    });

    test('triggerFolderScan handles user cancel gracefully', async () => {
        const logs = [];
        const mockCtx = {
            appendScanLog: (msg) => logs.push(msg),
            showToast: () => {},
            invoke: async (cmd) => {
                if (cmd === 'pick_folder_and_scan') return null; // user cancelled
                return null;
            }
        };

        const res = await desktopStorageAdapter.triggerFolderScan(mockCtx);
        assert.equal(res, null);
        assert.ok(logs.some(l => l.includes('cancelled')));
    });

    test('triggerFolderScan falls back to scanLibrary if native dialog throws', async () => {
        let libraryScanned = false;
        const mockCtx = {
            appendScanLog: () => {},
            showToast: () => {},
            updateScanProgressUI: () => {},
            invoke: async (cmd) => {
                if (cmd === 'pick_folder_and_scan') {
                    throw new Error('dialog error');
                }
                if (cmd === 'scan_library_paths') {
                    libraryScanned = true;
                    return { tracks_added: 1, tracks_updated: 0, errors: [] };
                }
                return null;
            }
        };

        const res = await desktopStorageAdapter.triggerFolderScan(mockCtx);
        assert.equal(libraryScanned, true);
        assert.equal(res.tracks_added, 1);
    });

    test('triggerAudioImport calls pick_audio_files_and_import', async () => {
        let picked = false;
        const mockCtx = {
            appendScanLog: () => {},
            showToast: () => {},
            loadLibraryView: async () => {},
            refreshCurrentView: () => {},
            invoke: async (cmd) => {
                if (cmd === 'pick_audio_files_and_import') {
                    picked = true;
                    return { tracks_added: 3, tracks_updated: 0, errors: [] };
                }
                return null;
            }
        };

        const res = await desktopStorageAdapter.triggerAudioImport(mockCtx);
        assert.equal(picked, true);
        assert.equal(res.tracks_added, 3);
    });

    test('scanLibrary outputs clean desktop-specific messaging on 0 tracks found', async () => {
        const toasts = [];
        const logs = [];
        const mockCtx = {
            appendScanLog: (msg) => logs.push(msg),
            showToast: (msg, level) => toasts.push({ msg, level }),
            updateScanProgressUI: () => {},
            loadLibraryView: async () => {},
            refreshCurrentView: () => {},
            invoke: async (cmd) => {
                if (cmd === 'scan_library_paths') {
                    return { tracks_added: 0, tracks_updated: 0, errors: [] };
                }
                return null;
            }
        };

        await desktopStorageAdapter.scanLibrary(mockCtx);
        assert.ok(toasts.some(t => t.msg.includes('library is up to date')));
        assert.ok(logs.some(l => l.includes('Desktop library scan completed')));
        // Never output mobile/Scoped Storage text on desktop!
        assert.ok(!toasts.some(t => t.msg.includes('Scoped Storage')));
        assert.ok(!logs.some(l => l.includes('Scoped Storage')));
    });
});

describe('Android Storage Adapter', () => {
    test('scanLibrary outputs Android Scoped Storage guidance on 0 tracks found', async () => {
        const toasts = [];
        const logs = [];
        const mockCtx = {
            appendScanLog: (msg) => logs.push(msg),
            showToast: (msg, level) => toasts.push({ msg, level }),
            updateScanProgressUI: () => {},
            loadLibraryView: async () => {},
            refreshCurrentView: () => {},
            invoke: async (cmd) => {
                if (cmd === 'scan_library_paths') {
                    return { tracks_added: 0, tracks_updated: 0, errors: [] };
                }
                return null;
            }
        };

        await androidStorageAdapter.scanLibrary(mockCtx);
        assert.ok(toasts.some(t => t.msg.includes('No audio files in app sandbox')));
        assert.ok(logs.some(l => l.includes('Android Scoped Storage: Tap "Import Audio"')));
    });

    test('triggerAudioImport clicks global-audio-import-input', async () => {
        let clicked = false;
        let valueReset = false;
        const inputElem = {
            dataset: {},
            get value() { return 'old'; },
            set value(v) { if (v === '') valueReset = true; },
            click: () => { clicked = true; }
        };
        globalThis.document = {
            getElementById: (id) => (id === 'global-audio-import-input' ? inputElem : null),
            createElement: () => ({ setAttribute() {}, style: {}, dataset: {} }),
            body: { dataset: {}, appendChild() {} }
        };

        const mockCtx = {
            appendScanLog: () => {},
            showToast: () => {}
        };

        await androidStorageAdapter.triggerAudioImport(mockCtx);
        assert.equal(clicked, true);
        assert.equal(valueReset, true);
    });

    test('triggerFolderScan initiates scanLibrary and does not hijack folder input', async () => {
        let scanned = false;
        let folderClicked = false;
        const folderElem = {
            dataset: {},
            set value(_) {},
            click: () => { folderClicked = true; }
        };
        globalThis.document = {
            getElementById: (id) => (id === 'global-folder-scan-input' ? folderElem : null),
            createElement: () => ({ setAttribute() {}, style: {}, dataset: {} }),
            body: { dataset: {}, appendChild() {} }
        };

        const mockCtx = {
            appendScanLog: () => {},
            showToast: () => {},
            updateScanProgressUI: () => {},
            loadLibraryView: async () => {},
            refreshCurrentView: () => {},
            invoke: async (cmd) => {
                if (cmd === 'scan_library_paths') {
                    scanned = true;
                    return { tracks_added: 2, tracks_updated: 0, errors: [] };
                }
                return null;
            }
        };

        await androidStorageAdapter.triggerFolderScan(mockCtx);
        assert.equal(scanned, true);
        assert.equal(folderClicked, false);
    });

    test('handleFolderScan ignores input when no files selected', async () => {
        const logs = [];
        const mockCtx = {
            appendScanLog: (msg) => logs.push(msg)
        };
        await androidStorageAdapter.handleFolderScan(mockCtx, { files: [] });
        assert.ok(logs.some(l => l.includes('closed without selection')));
    });

    test('handleFolderScan warns when directory has no supported audio files', async () => {
        const logs = [];
        const toasts = [];
        const mockCtx = {
            appendScanLog: (msg) => logs.push(msg),
            showToast: (msg, level) => toasts.push({ msg, level })
        };
        const input = {
            files: [{ name: 'notes.txt', type: 'text/plain', size: 100 }],
            value: 'fake'
        };
        await androidStorageAdapter.handleFolderScan(mockCtx, input);
        assert.ok(logs.some(l => l.includes('No supported audio format files')));
        assert.ok(toasts.some(t => t.msg.includes('No audio files found')));
        assert.equal(input.value, '');
    });

    test('handleAudioImport skips files over 64MB to protect memory', async () => {
        const logs = [];
        const mockCtx = {
            appendScanLog: (msg) => logs.push(msg),
            showToast: () => {},
            updateScanProgressUI: () => {},
            finishScanProgressUI: () => {},
            loadLibraryView: async () => {},
            refreshCurrentView: () => {}
        };
        const input = {
            files: [{ name: 'huge_song.wav', type: 'audio/wav', size: 70 * 1024 * 1024 }],
            value: 'fake'
        };
        await androidStorageAdapter.handleAudioImport(mockCtx, input);
        assert.ok(logs.some(l => l.includes('File too large') && l.includes('skipped to avoid memory issues')));
        assert.equal(input.value, '');
    });

    test('handleAudioImport ingests audio file and notifies track imported', async () => {
        const originalFileReader = globalThis.FileReader;
        globalThis.FileReader = class MockFileReader {
            readAsDataURL() {
                setTimeout(() => {
                    this.result = 'data:audio/mp3;base64,SUQzBAAAAAAA';
                    if (this.onload) this.onload();
                }, 2);
            }
        };

        try {
            let invokedCmd = null;
            let invokedArgs = null;
            let importedTrack = null;

            const mockCtx = {
                appendScanLog: () => {},
                showToast: () => {},
                updateScanProgressUI: () => {},
                finishScanProgressUI: () => {},
                loadLibraryView: async () => {},
                refreshCurrentView: () => {},
                handleTrackImported: (t) => { importedTrack = t; },
                invoke: async (cmd, args) => {
                    invokedCmd = cmd;
                    invokedArgs = args;
                    return { id: 'track-uuid-1', title: 'Test Song', artist: 'Artist' };
                }
            };

            const input = {
                files: [{ name: 'song.mp3', type: 'audio/mp3', size: 1024 }],
                value: 'fake'
            };

            await androidStorageAdapter.handleAudioImport(mockCtx, input);
            assert.equal(invokedCmd, 'import_audio_file');
            assert.equal(invokedArgs.name, 'song.mp3');
            assert.equal(invokedArgs.data_base64, 'SUQzBAAAAAAA');
            assert.equal(importedTrack.id, 'track-uuid-1');
            assert.equal(input.value, '');
        } finally {
            if (originalFileReader) globalThis.FileReader = originalFileReader;
            else delete globalThis.FileReader;
        }
    });
});

describe('Library Module Delegation & Backward Compatibility', () => {
    test('libraryMethods delegate cleanly to active adapter', async () => {
        let scanCalled = false;
        let importAudioCalled = false;
        let folderScanCalled = false;

        const mockAdapter = {
            scanLibrary: async (ctx, paths) => { scanCalled = true; return { tracks_added: 1 }; },
            triggerAudioImport: async (ctx) => { importAudioCalled = true; },
            triggerFolderScan: async (ctx) => { folderScanCalled = true; },
            handleFolderScan: async (ctx, input) => {},
            handleAudioImport: async (ctx, input) => {},
            ensureFileInput: (ctx) => {}
        };
        setStorageAdapterForTesting(mockAdapter);

        const bridge = Object.create(libraryMethods);
        await bridge.scanLibrary(['/custom/path']);
        assert.equal(scanCalled, true);

        await bridge.triggerAudioImport();
        assert.equal(importAudioCalled, true);

        await bridge.triggerFolderScan();
        assert.equal(folderScanCalled, true);

        resetStorageAdapterForTesting();
    });

    test('handleTrackImported updates in-memory collection and triggers refresh', () => {
        let rendered = false;
        const bridge = Object.assign(Object.create(libraryMethods), {
            tracks: [{ id: '1', title: 'Old' }],
            activeView: 'library',
            renderLibraryTracks: () => { rendered = true; },
            refreshCurrentView: () => {}
        });

        bridge.handleTrackImported({ id: '2', title: 'New' });
        assert.equal(bridge.tracks.length, 2);
        assert.equal(bridge.tracks[0].id, '2');
        assert.equal(rendered, true);

        // Update existing track in-place
        bridge.handleTrackImported({ id: '1', title: 'Old Updated' });
        assert.equal(bridge.tracks.length, 2);
        assert.equal(bridge.tracks[1].title, 'Old Updated');
    });
});
