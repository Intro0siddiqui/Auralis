/**
 * ViewRouter Module — Auralis v2
 * Rock-solid, zero-dependency Vanilla JS SPA ViewRouter replacing HTMX.
 * Provides persistent view-pane caching, browser & Android hardware back button
 * navigation, lifecycle hooks, and navigation link synchronization.
 */

export class ViewRouter {
    constructor() {
        this.contentContainer = null;
        this.currentView = null;
        this.previousView = null;
        this.views = new Map(); // name -> loaderFn
        this.panes = new Map(); // name -> HTMLElement
        this.listeners = {};
        this.initialized = false;
        this.options = {
            defaultView: 'home',
            activeClass: 'active',
            paneClass: 'view-pane',
        };
        this._historyBound = false;
        this._delegationBound = false;
        this._initialEntered = false;
        this._navId = 0;
    }

    /**
     * Normalize view name by stripping hashes, paths, and .html extensions.
     * Maps 'downloads' <-> 'download' aliases.
     * @param {string} name
     * @returns {string}
     */
    normalizeViewName(name) {
        if (!name || typeof name !== 'string') return this.options.defaultView || 'home';
        let clean = name.trim();
        if (clean.startsWith('#')) clean = clean.slice(1);
        if (clean.startsWith('/partials/')) clean = clean.slice('/partials/'.length);
        if (clean.startsWith('partials/')) clean = clean.slice('partials/'.length);
        if (clean.endsWith('.html')) clean = clean.slice(0, -5);
        clean = clean.replace(/^\/+|\/+$/g, '');
        return clean || this.options.defaultView || 'home';
    }

    /**
     * Register an event listener for router lifecycle events.
     * Supported events: 'view:enter', 'view:leave', 'modal:open', 'modal:close'.
     * @param {string} event
     * @param {Function} callback
     * @returns {Function} unbind function
     */
    on(event, callback) {
        if (!this.listeners[event]) this.listeners[event] = [];
        this.listeners[event].push(callback);
        return () => this.off(event, callback);
    }

    /**
     * Remove an event listener.
     * @param {string} event
     * @param {Function} callback
     */
    off(event, callback) {
        if (!this.listeners[event]) return;
        this.listeners[event] = this.listeners[event].filter(cb => cb !== callback);
    }

    /**
     * Emit lifecycle events to internal listeners, window CustomEvents,
     * and the Auralis bridge if present.
     * @param {string} event
     * @param  {...any} args
     */
    emit(event, ...args) {
        if (this.listeners[event]) {
            for (const cb of [...this.listeners[event]]) {
                try {
                    cb(...args);
                } catch (e) {
                    console.error(`[ViewRouter] Listener error on '${event}':`, e);
                }
            }
        }

        if (typeof window !== 'undefined') {
            try {
                if (window.Auralis && window.Auralis.bridge && typeof window.Auralis.bridge.emit === 'function') {
                    window.Auralis.bridge.emit(event, ...args);
                }
                const detail = args.length === 1 ? args[0] : args;
                window.dispatchEvent(new CustomEvent(event, { detail }));
            } catch (_) {}
        }
    }

    /**
     * Register a custom loader function for a given view name.
     * Loader function can return an HTML string or an HTMLElement.
     * @param {string} name
     * @param {Function} loaderFn
     * @returns {ViewRouter}
     */
    registerView(name, loaderFn) {
        if (!name || typeof loaderFn !== 'function') return this;
        const normalized = this.normalizeViewName(name);
        this.views.set(normalized, loaderFn);
        if (this.panes.has(normalized)) {
            const old = this.panes.get(normalized);
            if (old && old.parentElement) old.parentElement.removeChild(old);
            this.panes.delete(normalized);
        }
        return this;
    }

    /**
     * Initialize the ViewRouter on the target container with options.
     * @param {HTMLElement|string} contentContainer
     * @param {Object} options
     * @returns {ViewRouter}
     */
    init(contentContainer, options = {}) {
        if (this.initialized && this.contentContainer) {
            return this;
        }

        this.options = Object.assign(this.options, options);

        if (contentContainer) {
            this.contentContainer = typeof contentContainer === 'string'
                ? document.querySelector(contentContainer)
                : contentContainer;
        }
        if (!this.contentContainer && typeof document !== 'undefined') {
            this.contentContainer = document.getElementById('content');
        }

        this.setupDelegatedEvents();
        this.setupHistoryListener();
        this.ensureSidebarLoaded();

        // Discover or index any pre-rendered view panes in DOM
        if (this.contentContainer) {
            this.discoverExistingPanes();
        }

        // Determine initial route from URL hash or default
        let initialView = this.options.defaultView || 'home';
        if (typeof window !== 'undefined' && window.location && window.location.hash) {
            const hashView = this.normalizeViewName(window.location.hash);
            if (hashView) initialView = hashView;
        }

        this.initialized = true;

        // Establish initial view state
        this.currentView = initialView;
        this.syncNavLinks(initialView);
        this._initialEntered = true;

        // If initial view pane was already discovered in DOM, display it.
        // Otherwise (cold boot on empty container), mount initial view pane.
        if (this.panes.has(initialView)) {
            const pane = this.panes.get(initialView);
            pane.style.display = '';
            pane.classList.add('active');
        } else if (this.contentContainer) {
            this.getOrCreateViewPane(initialView).then(pane => {
                if (pane && this.currentView === initialView) {
                    pane.style.display = '';
                    pane.classList.add('active');
                }
            }).catch(() => {});
        }

        // Emit initial view:enter hook
        this.emit('view:enter', initialView, {});

        return this;
    }

    /**
     * Index existing .view-pane children in the contentContainer.
     * If contentContainer has raw children not yet wrapped in a .view-pane, wrap them.
     */
    discoverExistingPanes() {
        if (!this.contentContainer) return;

        const panes = this.contentContainer.querySelectorAll('.view-pane');
        for (const pane of panes) {
            const viewName = pane.dataset?.view || (pane.id && pane.id.replace(/^view-/, ''));
            if (viewName) {
                const normalized = this.normalizeViewName(viewName);
                this.panes.set(normalized, pane);
            }
        }

        // If contentContainer has existing children but no .view-pane, wrap them into home pane
        if (panes.length === 0 && this.contentContainer.children.length > 0) {
            const defaultName = this.options.defaultView || 'home';
            const doc = this.contentContainer.ownerDocument || (typeof document !== 'undefined' ? document : null);
            if (!doc) return;

            const pane = doc.createElement('div');
            pane.className = 'view-pane active';
            pane.id = `view-${defaultName}`;
            if (pane.dataset) pane.dataset.view = defaultName;
            pane.setAttribute('data-view', defaultName);

            while (this.contentContainer.firstChild) {
                pane.appendChild(this.contentContainer.firstChild);
            }
            this.contentContainer.appendChild(pane);
            this.panes.set(defaultName, pane);
        }
    }

    /**
     * Ensure desktop sidebar navigation partial is loaded into #sidebar if present and empty.
     */
    async ensureSidebarLoaded() {
        if (typeof document === 'undefined') return;
        const sidebar = document.getElementById('sidebar');
        if (!sidebar) return;

        // If sidebar is empty or has a pending hx-get attribute
        if (sidebar.children.length === 0 || sidebar.hasAttribute('hx-get')) {
            try {
                const html = await this.fetchViewTemplate('nav');
                if (html) {
                    sidebar.innerHTML = html;
                    sidebar.removeAttribute('hx-get');
                    sidebar.removeAttribute('hx-trigger');
                    sidebar.removeAttribute('hx-target');
                    if (window.lucide && typeof window.lucide.createIcons === 'function') {
                        try {
                            window.lucide.createIcons({ root: sidebar });
                        } catch (_) {
                            window.lucide.createIcons();
                        }
                    }
                    if (this.currentView) {
                        this.syncNavLinks(this.currentView);
                    }
                }
            } catch (_) {}
        }
    }

    /**
     * Fallback template loader. Fetches view HTML via Tauri's invoke('render_template')
     * with fallback to standard HTTP fetch of /partials/{name}.html.
     * @param {string} viewName
     * @returns {Promise<string>}
     */
    async fetchViewTemplate(viewName) {
        const candidateNames = [viewName];
        if (viewName === 'downloads') candidateNames.push('download');
        if (viewName === 'download') candidateNames.push('downloads');

        for (const name of candidateNames) {
            // 1. Try Tauri invoke('render_template', { name })
            try {
                const invoke = (window.Auralis && window.Auralis.bridge && typeof window.Auralis.bridge.invoke === 'function')
                    ? window.Auralis.bridge.invoke.bind(window.Auralis.bridge)
                    : (window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.invoke)
                    || (window.__TAURI__ && window.__TAURI__.core && window.__TAURI__.core.invoke);

                if (invoke) {
                    const html = await invoke('render_template', { name });
                    if (html && typeof html === 'string') {
                        return html;
                    }
                }
            } catch (_) {}

            // 2. Fallback to fetch('/partials/' + name + '.html')
            if (typeof fetch === 'function') {
                const urls = [`/partials/${name}.html`, `partials/${name}.html`];
                for (const url of urls) {
                    try {
                        const res = await fetch(url);
                        if (res && res.ok) {
                            return await res.text();
                        }
                    } catch (_) {}
                }
            }
        }

        // Return a clean fallback container if network fetch / invoke not available
        return `<div class="page-${viewName}"></div>`;
    }

    /**
     * Get an existing view pane from cache or create and mount a new one inside #content.
     * @param {string} viewName
     * @param {Object} params
     * @returns {Promise<HTMLElement>}
     */
    async getOrCreateViewPane(viewName, params = {}) {
        const normalized = this.normalizeViewName(viewName);

        // 1. Check in-memory pane cache
        if (this.panes.has(normalized)) {
            const pane = this.panes.get(normalized);
            if (pane && pane.parentElement === this.contentContainer) {
                return pane;
            }
        }

        // 2. Check if already present in DOM
        let pane = this.contentContainer ? this.contentContainer.querySelector(`#view-${normalized}`) : null;
        if (pane) {
            this.panes.set(normalized, pane);
            return pane;
        }

        // 3. Load view content (custom loader or template fetch)
        let htmlContent = '';
        if (this.views.has(normalized)) {
            const loader = this.views.get(normalized);
            const res = loader(params);
            const resolved = (res && typeof res.then === 'function') ? await res : res;
            if (resolved && (resolved.nodeType || resolved.tagName || resolved.classList)) {
                pane = resolved;
            } else if (typeof resolved === 'string') {
                htmlContent = resolved;
            }
        }

        if (!pane) {
            if (!htmlContent) {
                htmlContent = await this.fetchViewTemplate(normalized);
            }
            const doc = (this.contentContainer && this.contentContainer.ownerDocument)
                || (typeof document !== 'undefined' ? document : null);
            if (!doc) {
                throw new Error('No document context available to mount view pane');
            }
            pane = doc.createElement('div');
            pane.className = 'view-pane';
            pane.id = `view-${normalized}`;
            if (pane.dataset) pane.dataset.view = normalized;
            pane.setAttribute('data-view', normalized);
            pane.innerHTML = htmlContent;
        }

        // Initially hidden before mounting
        pane.style.display = 'none';
        pane.classList.remove('active');

        if (this.contentContainer) {
            this.contentContainer.appendChild(pane);
        }

        this.panes.set(normalized, pane);

        // Initialize Lucide icons within newly mounted pane
        if (typeof window !== 'undefined' && window.lucide && typeof window.lucide.createIcons === 'function') {
            try {
                window.lucide.createIcons({ root: pane });
            } catch (_) {
                window.lucide.createIcons();
            }
        }

        return pane;
    }

    /**
     * Navigate to a target view.
     * Hides inactive view panes, displays the target pane, preserves scroll positions
     * and form inputs, updates history, syncs nav links, and emits lifecycle hooks.
     * @param {string} viewName
     * @param {Object} [params={}]
     * @param {boolean} [addToHistory=true]
     * @returns {Promise<boolean>}
     */
    async navigate(viewName, params = {}, addToHistory = true) {
        if (!this.contentContainer && typeof document !== 'undefined') {
            this.contentContainer = document.getElementById('content');
        }

        const targetView = this.normalizeViewName(viewName);
        const navId = ++this._navId;

        // If navigating to already active view with existing displayed pane and not initial entrance
        if (targetView === this.currentView && this.panes.has(targetView) && !this._initialEntered) {
            const activePane = this.panes.get(targetView);
            if (activePane && activePane.classList.contains('active') && activePane.style.display !== 'none') {
                this.syncNavLinks(targetView);
                return true;
            }
        }

        // Update active classes on desktop sidebar and mobile navigation immediately for responsive tap feedback
        this.syncNavLinks(targetView);

        // Resolve or create target view pane (may involve async template fetch or custom loader)
        let targetPane = null;
        try {
            targetPane = await this.getOrCreateViewPane(targetView, params);
        } catch (err) {
            console.error(`[ViewRouter] Failed to navigate to '${targetView}':`, err);
            // If this navigation was not superseded, restore nav links to current view
            if (navId === this._navId && this.currentView) {
                this.syncNavLinks(this.currentView);
            }
            return false;
        }

        // Drop stale navigation if superseded by a newer tap while awaiting pane resolution
        if (navId !== this._navId) {
            return false;
        }

        const previousView = this.currentView;
        const wasInitial = (targetView === this.currentView && this._initialEntered);
        this._initialEntered = false;

        // If leaving a view, record its scroll position and emit view:leave
        if (previousView && previousView !== targetView) {
            this.emit('view:leave', previousView);
            const prevPane = this.panes.get(previousView) || (this.contentContainer && this.contentContainer.querySelector(`#view-${previousView}`));
            if (prevPane) {
                if (this.contentContainer) {
                    prevPane._savedScrollTop = this.contentContainer.scrollTop;
                }
                prevPane.style.display = 'none';
                prevPane.classList.remove('active');
            }
        }

        // Re-check token after emitting view:leave in case a listener triggered navigation
        if (navId !== this._navId) {
            return false;
        }

        this.previousView = previousView;
        this.currentView = targetView;

        // Ensure all other view panes in contentContainer are hidden
        if (this.contentContainer) {
            const allPanes = this.contentContainer.querySelectorAll('.view-pane');
            for (const p of allPanes) {
                if (p !== targetPane) {
                    p.style.display = 'none';
                    p.classList.remove('active');
                }
            }
        }

        // Show target pane and restore its saved scroll position
        if (targetPane) {
            targetPane.style.display = '';
            targetPane.classList.add('active');
            if (this.contentContainer) {
                if (typeof targetPane._savedScrollTop === 'number') {
                    this.contentContainer.scrollTop = targetPane._savedScrollTop;
                } else {
                    this.contentContainer.scrollTop = 0;
                }
            }
        }

        // Guarantee nav links match the final committed view
        this.syncNavLinks(targetView);

        // Update bridge state & sync player bar with Rust playback state without wiping view DOM
        if (typeof window !== 'undefined' && window.Auralis && window.Auralis.bridge) {
            window.Auralis.bridge.activeView = (targetView === 'download' ? 'downloads' : targetView);
            if (typeof window.Auralis.bridge._syncPlayerBar === 'function') {
                try {
                    window.Auralis.bridge._syncPlayerBar();
                } catch (_) {}
            }
        }

        // Update browser & Android WebView history
        if (typeof window !== 'undefined' && window.history) {
            const state = { view: targetView, params };
            const hash = '#' + targetView;
            if (addToHistory) {
                window.history.pushState(state, '', hash);
            } else {
                window.history.replaceState(state, '', hash);
            }
        }

        // Emit view:enter hook (skip if this was the initial boot event already emitted by init)
        if (!wasInitial) {
            this.emit('view:enter', targetView, params);
        }

        return true;
    }

    /**
     * Synchronize navigation indicators across desktop sidebar and mobile bottom nav.
     * Automatically toggles .active on .nav-item[data-nav] and .mobile-nav-item[data-nav].
     * @param {string} viewName
     */
    syncNavLinks(viewName) {
        if (typeof document === 'undefined') return;

        const target = this.normalizeViewName(viewName);
        const alt = (target === 'downloads') ? 'download' : (target === 'download' ? 'downloads' : null);

        const links = document.querySelectorAll('.nav-item, .mobile-nav-item, .library-subnav [data-nav]');
        for (const item of links) {
            const navTarget = item.dataset?.nav
                || item.getAttribute('data-nav')
                || (item.getAttribute('href') && item.getAttribute('href').replace(/^#/, ''))
                || (item.getAttribute('hx-get') && item.getAttribute('hx-get').match(/\/partials\/([a-zA-Z0-9_-]+)\.html/)?.[1]);

            if (navTarget) {
                const normNav = this.normalizeViewName(navTarget);
                const isActive = Boolean(normNav === target || (alt && normNav === alt));
                if (item.classList.contains('btn')) {
                    item.classList.toggle('btn-primary', isActive);
                    item.classList.toggle('btn-secondary', !isActive);
                } else {
                    item.classList.toggle('active', isActive);
                }
            }
        }
    }

    /**
     * Open a modal dialog or overlay partial (e.g. 'player-full') into #overlay-root.
     * @param {string} modalName
     * @returns {Promise<boolean>}
     */
    async openModal(modalName) {
        if (typeof document === 'undefined') return false;
        const overlayRoot = document.getElementById('overlay-root');
        if (!overlayRoot) return false;

        try {
            const html = await this.fetchViewTemplate(modalName);
            overlayRoot.innerHTML = html;

            if (window.lucide && typeof window.lucide.createIcons === 'function') {
                try {
                    window.lucide.createIcons({ root: overlayRoot });
                } catch (_) {
                    window.lucide.createIcons();
                }
            }

            if (modalName === 'player-full' || modalName.includes('player')) {
                if (window.Auralis && window.Auralis.player) {
                    if (typeof window.Auralis.player.wireFullScreenElements === 'function') {
                        window.Auralis.player.wireFullScreenElements();
                    }
                    if (typeof window.Auralis.player.hydrateState === 'function') {
                        window.Auralis.player.hydrateState().catch(() => {});
                    }
                }
            }

            this.emit('modal:open', modalName);
            return true;
        } catch (err) {
            console.error(`[ViewRouter] Failed to open modal '${modalName}':`, err);
            return false;
        }
    }

    /**
     * Close modal dialog or overlay partial in #overlay-root.
     * @param {string} [modalName]
     * @returns {boolean}
     */
    closeModal(modalName = 'player-full') {
        if (typeof document === 'undefined') return false;
        const overlayRoot = document.getElementById('overlay-root');
        if (overlayRoot) {
            overlayRoot.innerHTML = '';
        }
        this.emit('modal:close', modalName);
        return true;
    }

    /**
     * Setup global delegated click listener on document for [data-nav] and [data-nav-modal].
     * Also gracefully intercepts player-track clicks and legacy hx-get links.
     */
    setupDelegatedEvents() {
        if (this._delegationBound || typeof document === 'undefined') return;
        this._delegationBound = true;

        document.addEventListener('click', (event) => {
            const target = event.target;
            if (!target || typeof target.closest !== 'function') return;

            // 1. Intercept [data-nav]
            const navEl = target.closest('[data-nav]');
            if (navEl) {
                const targetView = navEl.dataset?.nav || navEl.getAttribute('data-nav');
                if (targetView) {
                    if (typeof event.preventDefault === 'function') event.preventDefault();
                    this.navigate(targetView);
                    return;
                }
            }

            // 2. Intercept [data-nav-modal]
            const modalEl = target.closest('[data-nav-modal]');
            if (modalEl) {
                const modalName = modalEl.dataset?.navModal || modalEl.getAttribute('data-nav-modal');
                if (modalName) {
                    if (typeof event.preventDefault === 'function') event.preventDefault();
                    this.openModal(modalName);
                    return;
                }
            }

            // 3. Intercept player bar click to open player-full modal
            const playerTrackEl = target.closest('.player-track');
            if (playerTrackEl && !target.closest('#track-like, button, a')) {
                if (typeof event.preventDefault === 'function') event.preventDefault();
                this.openModal('player-full');
                return;
            }

            // 4. Graceful migration fallback: intercept elements with hx-get
            const hxEl = target.closest('[hx-get]');
            if (hxEl) {
                const hxGet = hxEl.getAttribute('hx-get');
                if (hxGet) {
                    if (hxGet.includes('player-full')) {
                        if (typeof event.preventDefault === 'function') event.preventDefault();
                        this.openModal('player-full');
                        return;
                    }
                    const match = hxGet.match(/\/partials\/([a-zA-Z0-9_-]+)\.html/);
                    if (match && match[1] && match[1] !== 'nav') {
                        if (typeof event.preventDefault === 'function') event.preventDefault();
                        this.navigate(match[1]);
                        return;
                    }
                }
            }
        });
    }

    /**
     * Setup history & popstate listener for browser and Android hardware back button.
     */
    setupHistoryListener() {
        if (this._historyBound || typeof window === 'undefined') return;
        this._historyBound = true;

        window.addEventListener('popstate', (event) => {
            // If an overlay or modal was dismissed by the app's back dismissal handler, skip navigation
            const overlayRoot = document.getElementById('overlay-root');
            if (overlayRoot && overlayRoot.children.length > 0) {
                return;
            }
            const queueDrawer = document.getElementById('player-full-queue-drawer');
            if (queueDrawer && queueDrawer.classList.contains('open')) {
                return;
            }
            const playlistDetail = document.getElementById('playlist-detail');
            if (playlistDetail && playlistDetail.style.display !== 'none' && playlistDetail.children.length > 0) {
                return;
            }

            let targetView = null;
            if (event.state && event.state.view) {
                targetView = event.state.view;
            } else if (window.location.hash) {
                targetView = this.normalizeViewName(window.location.hash);
            } else {
                targetView = this.options.defaultView || 'home';
            }

            if (targetView && targetView !== this.currentView) {
                this.navigate(targetView, event.state?.params || {}, false);
            }
        });

        // Intercept Android hardware back button event if dispatched by Cordova/Capacitor/WebView
        document.addEventListener('backbutton', (e) => {
            const overlayRoot = document.getElementById('overlay-root');
            if (overlayRoot && overlayRoot.children.length > 0) {
                // Handled by dismissOpenOverlays
                return;
            }
            if (this.previousView && this.previousView !== this.currentView) {
                if (typeof e.preventDefault === 'function') e.preventDefault();
                window.history.back();
            }
        }, false);
    }
}

// Export singleton instance as default and named export
export const router = new ViewRouter();
export default router;
