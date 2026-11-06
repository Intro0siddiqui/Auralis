#!/usr/bin/env node
/**
 * router.test.js — unit tests for ui/js/modules/router.js (Vanilla JS ViewRouter)
 *
 *   node --test scripts/tests/router.test.js
 *
 * Covers:
 *  - View name normalization and route aliases
 *  - Route registration and custom loaders
 *  - View container creation, wrapping, and persistent mounting (.view-pane)
 *  - Navigation state transitions (active pane display, inactive pane hiding)
 *  - Scroll position preservation across view transitions
 *  - History pushState / replaceState and popstate / backbutton navigation
 *  - Delegated click handling on [data-nav], [data-nav-modal], and player bar
 *  - Navigation link synchronization across desktop and mobile nav items
 *  - Lifecycle hook emissions ('view:enter', 'view:leave', 'modal:open')
 *  - Static claims verifying removal of HTMX and clean Vanilla JS router architecture
 */

import { describe, it, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { ViewRouter, router as singletonRouter } from '../../ui/js/modules/router.js';

const here = path.dirname(fileURLToPath(import.meta.url));
const uiDir = path.join(here, '..', '..', 'ui');

// ── Minimal Mock DOM Harness ──────────────────────────────────────────────────

class MockClassList {
    constructor(initial = []) {
        this._classes = new Set(initial);
    }
    add(...cls) {
        for (const c of cls) if (c) this._classes.add(c);
    }
    remove(...cls) {
        for (const c of cls) if (c) this._classes.delete(c);
    }
    contains(c) {
        return this._classes.has(c);
    }
    toggle(c, force) {
        if (force !== undefined) {
            if (force) {
                this._classes.add(c);
                return true;
            } else {
                this._classes.delete(c);
                return false;
            }
        }
        if (this._classes.has(c)) {
            this._classes.delete(c);
            return false;
        }
        this._classes.add(c);
        return true;
    }
    toArray() {
        return Array.from(this._classes);
    }
    toString() {
        return Array.from(this._classes).join(' ');
    }
}

class MockElement {
    constructor(tagName = 'div', attrs = {}) {
        this.tagName = tagName.toUpperCase();
        this.attributes = new Map();
        this.dataset = {};
        this.classList = new MockClassList();
        this.style = {};
        this.children = [];
        this.parentElement = null;
        this.id = '';
        this.scrollTop = 0;
        this._innerHTML = '';

        for (const [k, v] of Object.entries(attrs)) {
            this.setAttribute(k, v);
        }
    }

    get className() {
        return this.classList.toString();
    }
    set className(val) {
        this.classList = new MockClassList(val ? val.split(/\s+/).filter(Boolean) : []);
    }

    get firstChild() {
        return this.children[0] || null;
    }

    get innerHTML() {
        return this._innerHTML;
    }
    set innerHTML(val) {
        this._innerHTML = String(val);
        // Minimal child clearing when innerHTML = ''
        if (this._innerHTML === '') {
            for (const child of this.children) {
                child.parentElement = null;
            }
            this.children = [];
        }
    }

    setAttribute(name, value) {
        const valStr = String(value);
        this.attributes.set(name, valStr);
        if (name === 'id') this.id = valStr;
        if (name === 'class') this.className = valStr;
        if (name.startsWith('data-')) {
            const camel = name.slice(5).replace(/-([a-z])/g, (_, g) => g.toUpperCase());
            this.dataset[camel] = valStr;
        }
    }

    getAttribute(name) {
        return this.attributes.has(name) ? this.attributes.get(name) : null;
    }

    hasAttribute(name) {
        return this.attributes.has(name);
    }

    removeAttribute(name) {
        this.attributes.delete(name);
        if (name === 'id') this.id = '';
        if (name === 'class') this.className = '';
        if (name.startsWith('data-')) {
            const camel = name.slice(5).replace(/-([a-z])/g, (_, g) => g.toUpperCase());
            delete this.dataset[camel];
        }
    }

    appendChild(child) {
        if (!child) return child;
        if (child.parentElement) {
            child.parentElement.removeChild(child);
        }
        child.parentElement = this;
        this.children.push(child);
        return child;
    }

    removeChild(child) {
        const idx = this.children.indexOf(child);
        if (idx !== -1) {
            this.children.splice(idx, 1);
            child.parentElement = null;
        }
        return child;
    }

    matches(selector) {
        if (!selector) return false;
        const parts = selector.split(',').map((s) => s.trim());
        return parts.some((part) => this._matchesSingle(part));
    }

    _matchesSingle(sel) {
        // Tag + attribute e.g. .nav-item[data-nav="home"] or [data-nav]
        const tagMatch = sel.match(/^([a-zA-Z0-9_-]+)/);
        if (tagMatch && this.tagName !== tagMatch[1].toUpperCase()) {
            return false;
        }

        const classMatches = sel.match(/\.([a-zA-Z0-9_-]+)/g);
        if (classMatches) {
            for (const cm of classMatches) {
                if (!this.classList.contains(cm.slice(1))) return false;
            }
        }

        const idMatch = sel.match(/#([a-zA-Z0-9_-]+)/);
        if (idMatch && this.id !== idMatch[1]) {
            return false;
        }

        const attrMatches = sel.match(/\[([a-zA-Z0-9_-]+)(?:=([^\],]+))?\]/g);
        if (attrMatches) {
            for (const am of attrMatches) {
                const parsed = am.match(/\[([a-zA-Z0-9_-]+)(?:=(?:"([^"]*)"|'([^']*)'|([^\]]*)))?\]/);
                if (parsed) {
                    const attrName = parsed[1];
                    const attrVal = parsed[2] ?? parsed[3] ?? parsed[4];
                    if (!this.hasAttribute(attrName)) return false;
                    if (attrVal !== undefined && this.getAttribute(attrName) !== attrVal) return false;
                }
            }
        }

        return true;
    }

    closest(selector) {
        let el = this;
        while (el) {
            if (el instanceof MockElement && el.matches(selector)) {
                return el;
            }
            el = el.parentElement;
        }
        return null;
    }

    querySelector(selector) {
        return this.querySelectorAll(selector)[0] || null;
    }

    querySelectorAll(selector) {
        const results = [];
        const walk = (node) => {
            for (const child of node.children) {
                if (child.matches(selector)) {
                    results.push(child);
                }
                walk(child);
            }
        };
        walk(this);
        return results;
    }
}

class MockEvent {
    constructor(type, init = {}) {
        this.type = type;
        this.target = init.target || null;
        this.defaultPrevented = false;
        this.detail = init.detail || null;
        Object.assign(this, init);
    }
    preventDefault() {
        this.defaultPrevented = true;
    }
}

function createMockEnvironment() {
    const docListeners = new Map();
    const winListeners = new Map();

    const historyStack = [];
    let historyIndex = -1;

    const documentMock = {
        createElement: (tag, attrs) => new MockElement(tag, attrs),
        getElementById: (id) => {
            if (id === 'content') return contentEl;
            if (id === 'overlay-root') return overlayRootEl;
            if (id === 'sidebar') return sidebarEl;
            return rootEl.querySelector(`#${id}`);
        },
        querySelector: (sel) => rootEl.querySelector(sel),
        querySelectorAll: (sel) => rootEl.querySelectorAll(sel),
        addEventListener: (type, fn) => {
            if (!docListeners.has(type)) docListeners.set(type, []);
            docListeners.get(type).push(fn);
        },
        removeEventListener: (type, fn) => {
            if (!docListeners.has(type)) return;
            docListeners.set(type, docListeners.get(type).filter((f) => f !== fn));
        },
        dispatchEvent: (evt) => {
            const list = docListeners.get(evt.type) || [];
            for (const fn of list) fn(evt);
            return !evt.defaultPrevented;
        },
    };

    const windowMock = {
        location: { hash: '' },
        history: {
            pushState: (state, title, url) => {
                historyStack.splice(historyIndex + 1);
                historyStack.push({ state, title, url });
                historyIndex = historyStack.length - 1;
                windowMock.location.hash = url;
            },
            replaceState: (state, title, url) => {
                if (historyIndex >= 0) {
                    historyStack[historyIndex] = { state, title, url };
                } else {
                    historyStack.push({ state, title, url });
                    historyIndex = 0;
                }
                windowMock.location.hash = url;
            },
            get stack() { return historyStack; },
            get currentIndex() { return historyIndex; },
        },
        addEventListener: (type, fn) => {
            if (!winListeners.has(type)) winListeners.set(type, []);
            winListeners.get(type).push(fn);
        },
        removeEventListener: (type, fn) => {
            if (!winListeners.has(type)) return;
            winListeners.get(type).push(winListeners.get(type).filter((f) => f !== fn));
        },
        dispatchEvent: (evt) => {
            const list = winListeners.get(evt.type) || [];
            for (const fn of list) fn(evt);
            return !evt.defaultPrevented;
        },
        CustomEvent: MockEvent,
        Auralis: {
            bridge: {
                activeView: 'home',
                refreshCurrentView: () => {},
                emit: () => {},
            },
        },
        lucide: {
            createIcons: () => {},
        },
    };

    const rootEl = new MockElement('body');
    const contentEl = new MockElement('main', { id: 'content', class: 'content' });
    const overlayRootEl = new MockElement('div', { id: 'overlay-root' });
    const sidebarEl = new MockElement('aside', { id: 'sidebar', class: 'glass sidebar' });

    rootEl.appendChild(sidebarEl);
    rootEl.appendChild(contentEl);
    rootEl.appendChild(overlayRootEl);

    return {
        documentMock,
        windowMock,
        contentEl,
        overlayRootEl,
        sidebarEl,
        rootEl,
        docListeners,
        winListeners,
        fireDoc: (type, init = {}) => {
            const evt = new MockEvent(type, init);
            const list = docListeners.get(type) || [];
            for (const fn of list) fn(evt);
            return evt;
        },
        fireWin: (type, init = {}) => {
            const evt = new MockEvent(type, init);
            const list = winListeners.get(type) || [];
            for (const fn of list) fn(evt);
            return evt;
        },
    };
}

// ── Test Harness Runner ───────────────────────────────────────────────────────

function withRouterHarness(fn) {
    const env = createMockEnvironment();
    const prevDoc = globalThis.document;
    const prevWin = globalThis.window;
    const prevElement = globalThis.Element;
    const prevCustomEvent = globalThis.CustomEvent;

    globalThis.document = env.documentMock;
    globalThis.window = env.windowMock;
    globalThis.Element = MockElement;
    globalThis.CustomEvent = MockEvent;

    const router = new ViewRouter();
    router.fetchViewTemplate = async (name) => `<div class="page-${name}">Mock ${name}</div>`;

    return Promise.resolve()
        .then(() => fn({ router, ...env }))
        .finally(() => {
            if (prevDoc === undefined) delete globalThis.document; else globalThis.document = prevDoc;
            if (prevWin === undefined) delete globalThis.window; else globalThis.window = prevWin;
            if (prevElement === undefined) delete globalThis.Element; else globalThis.Element = prevElement;
            if (prevCustomEvent === undefined) delete globalThis.CustomEvent; else globalThis.CustomEvent = prevCustomEvent;
        });
}

// ── Test Suites ───────────────────────────────────────────────────────────────

describe('ViewRouter Unit Tests', () => {

    describe('View Name Normalization', () => {
        it('normalizes view names correctly across hashes, paths, and extensions', () => {
            const r = new ViewRouter();
            assert.equal(r.normalizeViewName('library'), 'library');
            assert.equal(r.normalizeViewName('#library'), 'library');
            assert.equal(r.normalizeViewName('/partials/library.html'), 'library');
            assert.equal(r.normalizeViewName('partials/library.html'), 'library');
            assert.equal(r.normalizeViewName('download.html'), 'download');
            assert.equal(r.normalizeViewName('///search///'), 'search');
            assert.equal(r.normalizeViewName(''), 'home');
            assert.equal(r.normalizeViewName(null), 'home');
            assert.equal(r.normalizeViewName(undefined), 'home');
        });
    });

    describe('Route Registration & Custom Loaders', () => {
        it('registers custom view loaders and invokes them during navigation', async () => {
            await withRouterHarness(async ({ router, contentEl }) => {
                let loaderCalled = false;
                let passedParams = null;

                await router.init(contentEl, { defaultView: 'home' });

                router.registerView('custom-view', async (params) => {
                    loaderCalled = true;
                    passedParams = params;
                    return '<section class="custom-page"><h2>Custom View</h2></section>';
                });

                assert.ok(router.views.has('custom-view'), 'view must be registered in router.views');

                const ok = await router.navigate('custom-view', { filter: 'favorites' });
                assert.equal(ok, true, 'navigation must succeed');
                assert.equal(loaderCalled, true, 'custom loader must have been invoked');
                assert.deepEqual(passedParams, { filter: 'favorites' }, 'params must be forwarded to loader');

                const mounted = contentEl.querySelector('#view-custom-view');
                assert.ok(mounted, 'pane #view-custom-view must be mounted in #content');
                assert.equal(mounted.dataset.view, 'custom-view');
                assert.ok(mounted.classList.contains('active'), 'mounted pane must have .active class');
                assert.ok(mounted.innerHTML.includes('Custom View'), 'pane must contain rendered HTML');
            });
        });

        it('supports loaders returning an HTMLElement directly', async () => {
            await withRouterHarness(async ({ router, contentEl }) => {
                await router.init(contentEl);

                const customEl = new MockElement('section', { id: 'custom-element' });
                customEl.className = 'page-element';
                router.registerView('element-view', () => customEl);

                await router.navigate('element-view');
                assert.equal(router.currentView, 'element-view');
                assert.ok(contentEl.children.includes(customEl), 'custom element must be appended to #content');
            });
        });
    });

    describe('View Container Creation & Persistent Mounting (.view-pane)', () => {
        it('creates persistent view-panes and reuses them on return visits', async () => {
            await withRouterHarness(async ({ router, contentEl }) => {
                let libraryLoadCount = 0;
                let homeLoadCount = 0;

                router.registerView('home', () => {
                    homeLoadCount++;
                    return '<div class="page-home">Home Content</div>';
                });
                router.registerView('library', () => {
                    libraryLoadCount++;
                    return '<div class="page-library">Library Content</div>';
                });

                await router.init(contentEl);

                // Initial navigation to home loads it once
                await router.navigate('home');
                assert.equal(homeLoadCount, 1);
                const homePane = contentEl.querySelector('#view-home');
                assert.ok(homePane, 'home pane mounted');

                await router.navigate('library');
                assert.equal(libraryLoadCount, 1);
                const libPane = contentEl.querySelector('#view-library');
                assert.ok(libPane, 'library pane mounted');
                assert.equal(libPane.style.display, '', 'library pane is visible');
                assert.equal(homePane.style.display, 'none', 'home pane is hidden');

                // Return to home — loader must NOT be called again (cached!)
                await router.navigate('home');
                assert.equal(homeLoadCount, 1, 'home loader must not be called again; view is cached');
                assert.equal(homePane.style.display, '', 'home pane is visible again');
                assert.equal(libPane.style.display, 'none', 'library pane is hidden');
            });
        });

        it('discovers and wraps existing content in #content on init', async () => {
            await withRouterHarness(async ({ router, contentEl }) => {
                const preExisting = new MockElement('div', { class: 'welcome-card' });
                contentEl.appendChild(preExisting);

                await router.init(contentEl, { defaultView: 'home' });

                const homePane = contentEl.querySelector('#view-home');
                assert.ok(homePane, '#content initial markup must be wrapped in #view-home');
                assert.ok(homePane.children.includes(preExisting), 'initial child must be moved inside pane');
            });
        });
    });

    describe('Navigation Transitions & Active Panes', () => {
        it('toggles .active and style.display correctly across multiple views', async () => {
            await withRouterHarness(async ({ router, contentEl }) => {
                await router.init(contentEl);
                router.registerView('v1', () => '<div>View 1</div>');
                router.registerView('v2', () => '<div>View 2</div>');
                router.registerView('v3', () => '<div>View 3</div>');

                await router.navigate('v1');
                const p1 = contentEl.querySelector('#view-v1');
                assert.ok(p1.classList.contains('active'));
                assert.equal(p1.style.display, '');

                await router.navigate('v2');
                const p2 = contentEl.querySelector('#view-v2');
                assert.ok(!p1.classList.contains('active'));
                assert.equal(p1.style.display, 'none');
                assert.ok(p2.classList.contains('active'));
                assert.equal(p2.style.display, '');

                await router.navigate('v3');
                const p3 = contentEl.querySelector('#view-v3');
                assert.ok(!p2.classList.contains('active'));
                assert.equal(p2.style.display, 'none');
                assert.ok(p3.classList.contains('active'));
                assert.equal(p3.style.display, '');
            });
        });

        it('eliminates out-of-order race conditions when fast switching between views', async () => {
            await withRouterHarness(async ({ router, contentEl, navItem1, navItem2 }) => {
                await router.init(contentEl);

                // Register a slow view (takes 40ms) and a fast view (takes 5ms)
                router.registerView('sync', async () => {
                    await new Promise(r => setTimeout(r, 40));
                    return '<div class="page-sync">Syncing Device</div>';
                });
                router.registerView('library', async () => {
                    await new Promise(r => setTimeout(r, 5));
                    return '<div class="page-library">Library Songs</div>';
                });

                // Simulate rapid switching: tap sync, then immediately tap library
                const pSync = router.navigate('sync');
                const pLibrary = router.navigate('library');

                await Promise.all([pSync, pLibrary]);

                const syncPane = contentEl.querySelector('#view-sync');
                const libPane = contentEl.querySelector('#view-library');

                assert.equal(router.currentView, 'library', 'currentView must be the latest navigated target (library)');
                assert.ok(libPane, 'library pane must exist');
                assert.equal(libPane.style.display, '', 'library pane must be visible');
                assert.ok(libPane.classList.contains('active'), 'library pane must have active class');

                if (syncPane) {
                    assert.equal(syncPane.style.display, 'none', 'stale sync pane must be hidden');
                    assert.ok(!syncPane.classList.contains('active'), 'stale sync pane must not be active');
                }
            });
        });
    });

    describe('Scroll Position Preservation', () => {
        it('preserves and restores scroll positions when navigating back and forth', async () => {
            await withRouterHarness(async ({ router, contentEl }) => {
                await router.init(contentEl);
                router.registerView('library', () => '<div style="height:2000px">Library Tracks</div>');
                router.registerView('settings', () => '<div style="height:1000px">Settings</div>');

                await router.navigate('library');
                const libPane = contentEl.querySelector('#view-library');

                // Simulate user scrolling down in Library
                contentEl.scrollTop = 850;

                // Navigate away to Settings
                await router.navigate('settings');
                assert.equal(libPane._savedScrollTop, 850, 'scroll position must be saved on view leave');

                // Simulate scrolling in Settings
                contentEl.scrollTop = 200;

                // Navigate back to Library
                await router.navigate('library');
                assert.equal(contentEl.scrollTop, 850, 'saved scroll position must be restored');
            });
        });
    });

    describe('Lifecycle Hooks (view:enter, view:leave, modal:open)', () => {
        it('emits view:enter and view:leave in the correct lifecycle sequence', async () => {
            await withRouterHarness(async ({ router, contentEl }) => {
                const events = [];

                router.on('view:leave', (prev) => {
                    events.push({ event: 'view:leave', view: prev });
                });

                router.on('view:enter', (curr, params) => {
                    events.push({ event: 'view:enter', view: curr, params });
                });

                router.registerView('home', () => '<div>Home</div>');
                router.registerView('search', () => '<div>Search</div>');
                await router.init(contentEl, { defaultView: 'home' });

                // Navigate to search
                await router.navigate('search', { q: 'Mozart' });

                assert.deepEqual(events, [
                    { event: 'view:enter', view: 'home', params: {} },
                    { event: 'view:leave', view: 'home' },
                    { event: 'view:enter', view: 'search', params: { q: 'Mozart' } },
                ]);
            });
        });

        it('allows unbinding of lifecycle listeners via returned off function', async () => {
            await withRouterHarness(async ({ router, contentEl }) => {
                let callCount = 0;
                const unbind = router.on('view:enter', () => {
                    callCount++;
                });

                router.registerView('home', () => '<div>Home</div>');
                router.registerView('sync', () => '<div>Sync</div>');
                await router.init(contentEl);

                // Initial navigation to home triggered view:enter once
                assert.equal(callCount, 1);

                unbind(); // Unbind listener

                await router.navigate('sync');
                assert.equal(callCount, 1, 'listener must not be called after unbinding');
            });
        });
    });

    describe('History Navigation (pushState, popstate & Android Back Button)', () => {
        it('pushes state to history on navigate with addToHistory=true', async () => {
            await withRouterHarness(async ({ router, contentEl, windowMock }) => {
                await router.init(contentEl);
                router.registerView('home', () => '<div>Home</div>');
                router.registerView('library', () => '<div>Library</div>');

                await router.navigate('library', { tab: 'artists' }, true);

                const lastHistory = windowMock.history.stack[windowMock.history.currentIndex];
                assert.ok(lastHistory, 'history entry must exist');
                assert.deepEqual(lastHistory.state, { view: 'library', params: { tab: 'artists' } });
                assert.equal(lastHistory.url, '#library');
            });
        });

        it('handles popstate event by switching back to previous view', async () => {
            await withRouterHarness(async ({ router, contentEl, fireWin }) => {
                await router.init(contentEl);
                router.registerView('home', () => '<div>Home</div>');
                router.registerView('library', () => '<div>Library</div>');

                await router.navigate('home');
                await router.navigate('library');
                assert.equal(router.currentView, 'library');

                // Simulate Android Back Button popstate event
                fireWin('popstate', { state: { view: 'home' } });
                // Allow microtasks to complete
                await Promise.resolve();

                assert.equal(router.currentView, 'home', 'popstate must switch back to home view');
            });
        });

        it('skips popstate view navigation if an overlay or modal is active', async () => {
            await withRouterHarness(async ({ router, contentEl, overlayRootEl, fireWin }) => {
                await router.init(contentEl);
                router.registerView('home', () => '<div>Home</div>');
                router.registerView('library', () => '<div>Library</div>');

                await router.navigate('library');
                assert.equal(router.currentView, 'library');

                // Open modal inside overlayRoot
                const modal = new MockElement('div', { class: 'modal' });
                overlayRootEl.appendChild(modal);

                // Popstate arrives while modal is open
                fireWin('popstate', { state: { view: 'home' } });
                await Promise.resolve();

                assert.equal(router.currentView, 'library', 'navigation must be suppressed while modal is open');
            });
        });
    });

    describe('Delegated Click Handling ([data-nav], [data-nav-modal])', () => {
        it('intercepts click on [data-nav] elements and navigates to target view', async () => {
            await withRouterHarness(async ({ router, contentEl, rootEl, fireDoc }) => {
                await router.init(contentEl);
                router.registerView('home', () => '<div>Home</div>');
                router.registerView('settings', () => '<div>Settings</div>');

                const navLink = new MockElement('a', { 'data-nav': 'settings' });
                const icon = new MockElement('i', { class: 'lucide-settings' });
                const text = new MockElement('span');
                text.innerHTML = 'Settings';
                navLink.appendChild(icon);
                navLink.appendChild(text);
                rootEl.appendChild(navLink);

                // Click directly on nested child <span>
                const evt = fireDoc('click', { target: text });
                await new Promise((r) => setTimeout(r, 10));

                assert.equal(evt.defaultPrevented, true, 'event default must be prevented');
                assert.equal(router.currentView, 'settings', 'router must navigate to settings');
            });
        });

        it('intercepts click on [data-nav-modal] elements and opens modal', async () => {
            await withRouterHarness(async ({ router, contentEl, rootEl, overlayRootEl, fireDoc }) => {
                await router.init(contentEl);
                let modalOpened = null;
                router.on('modal:open', (m) => { modalOpened = m; });

                // Stub fetchViewTemplate for test
                router.fetchViewTemplate = async (name) => `<div class="modal-${name}">Modal Content</div>`;

                const modalTrigger = new MockElement('button', { 'data-nav-modal': 'player-full' });
                rootEl.appendChild(modalTrigger);

                fireDoc('click', { target: modalTrigger });
                await Promise.resolve();

                assert.equal(modalOpened, 'player-full', 'modal:open hook must be emitted');
                assert.ok(overlayRootEl.innerHTML.includes('Modal Content'), 'modal HTML injected into overlay');
            });
        });
    });

    describe('Navigation Link Synchronization (syncNavLinks)', () => {
        it('toggles .active on matching desktop and mobile navigation links', async () => {
            await withRouterHarness(async ({ router, contentEl, rootEl }) => {
                const homeLink = new MockElement('a', { class: 'nav-item', 'data-nav': 'home' });
                const libLink = new MockElement('a', { class: 'nav-item', 'data-nav': 'library' });
                const mobHome = new MockElement('a', { class: 'mobile-nav-item', 'data-nav': 'home' });
                const mobLib = new MockElement('a', { class: 'mobile-nav-item', 'data-nav': 'library' });

                rootEl.appendChild(homeLink);
                rootEl.appendChild(libLink);
                rootEl.appendChild(mobHome);
                rootEl.appendChild(mobLib);

                await router.init(contentEl);
                router.registerView('home', () => '<div>Home</div>');
                router.registerView('library', () => '<div>Library</div>');

                await router.navigate('home');
                assert.ok(homeLink.classList.contains('active'), 'desktop home must be active');
                assert.ok(mobHome.classList.contains('active'), 'mobile home must be active');
                assert.ok(!libLink.classList.contains('active'), 'desktop library must be inactive');
                assert.ok(!mobLib.classList.contains('active'), 'mobile library must be inactive');

                await router.navigate('library');
                assert.ok(!homeLink.classList.contains('active'), 'desktop home must be inactive');
                assert.ok(!mobHome.classList.contains('active'), 'mobile home must be inactive');
                assert.ok(libLink.classList.contains('active'), 'desktop library must be active');
                assert.ok(mobLib.classList.contains('active'), 'mobile library must be active');
            });
        });
    });
});

// ── Static Architecture Claims ────────────────────────────────────────────────

describe('Static Claims: HTMX Removal & Vanilla JS Router Migration', () => {
    it('ui/index.html does not include htmx.min.js script tag', () => {
        const indexHtml = fs.readFileSync(path.join(uiDir, 'index.html'), 'utf8');
        assert.ok(!indexHtml.includes('htmx.min.js'), 'index.html must not include htmx.min.js');
    });

    it('ui/vendor/htmx.min.js is permanently deleted', () => {
        const htmxPath = path.join(uiDir, 'vendor', 'htmx.min.js');
        assert.ok(!fs.existsSync(htmxPath), 'ui/vendor/htmx.min.js must not exist in repository');
    });

    it('ui/index.html main#content is clean without hx- attributes', () => {
        const indexHtml = fs.readFileSync(path.join(uiDir, 'index.html'), 'utf8');
        assert.match(indexHtml, /<main\s+id=["']content["']\s+class=["']content["']>\s*<\/main>/,
            'index.html main#content must be clean and unencumbered by hx- attributes');
        assert.ok(!indexHtml.includes('hx-sync='), 'hx-sync must be removed from index.html');
        assert.ok(!indexHtml.includes('hx-get='), 'hx-get must be removed from index.html');
    });

    it('ui/js/bridge.js exports router and wires bridge.navigateTo', () => {
        const bridgeJs = fs.readFileSync(path.join(uiDir, 'js', 'bridge.js'), 'utf8');
        assert.match(bridgeJs, /import\s+.*router.*from\s+['"]\.\/modules\/router\.js['"]/,
            'bridge.js must import router');
        assert.match(bridgeJs, /window\.Auralis\.router\s*=\s*router/,
            'bridge.js must expose window.Auralis.router');
        assert.match(bridgeJs, /bridge\.navigateTo\s*=.*router\.navigate/,
            'bridge.js must wire bridge.navigateTo to router.navigate');
    });

    it('ui/js/modules/router.js exists and exports ViewRouter and router singleton', () => {
        const routerPath = path.join(uiDir, 'js', 'modules', 'router.js');
        assert.ok(fs.existsSync(routerPath), 'ui/js/modules/router.js must exist');
        assert.ok(typeof ViewRouter === 'function', 'ViewRouter class must be exported');
        assert.ok(singletonRouter instanceof ViewRouter, 'router singleton must be exported');
    });
});
