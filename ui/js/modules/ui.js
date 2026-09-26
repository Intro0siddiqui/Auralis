/**
 * UI Module
 * Helper utilities for theming, escaping, formatting, asset resolution, cover art, and toasts.
 */

export const uiMethods = {
    async initTheme() {
        if (window.matchMedia) {
            try {
                const mediaQuery = window.matchMedia('(prefers-color-scheme: dark)');
                if (mediaQuery && typeof mediaQuery.addEventListener === 'function') {
                    mediaQuery.addEventListener('change', () => {
                        const currentSetting = (this.currentSettings && this.currentSettings.appearance && this.currentSettings.appearance.theme) || 'system';
                        if (String(currentSetting).toLowerCase() === 'system') {
                            this.applyTheme('system');
                        }
                    });
                }
            } catch (_) {}
        }

        try {
            const settings = await this.invoke('get_settings');
            if (settings) {
                this.currentSettings = settings;
                const theme = (settings.appearance && settings.appearance.theme) || 'system';
                this.applyTheme(theme);
            } else {
                this.applyTheme('system');
            }
        } catch (e) {
            this.applyTheme('system');
        }
    },

    applyTheme(theme) {
        const themeStr = String(theme || 'system').toLowerCase();
        let activeTheme = themeStr;
        if (themeStr === 'system') {
            activeTheme = (window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches) ? 'dark' : 'light';
        }

        if (document.documentElement && typeof document.documentElement.setAttribute === 'function') {
            document.documentElement.setAttribute('data-theme', activeTheme);
        }
        try {
            localStorage.setItem('auralis-theme', themeStr);
        } catch (_) {}
        const metaThemeColor = document.querySelector ? document.querySelector('meta[name="theme-color"]') : null;
        if (metaThemeColor && typeof metaThemeColor.setAttribute === 'function') {
            metaThemeColor.setAttribute('content', activeTheme === 'light' ? '#f0f4f8' : '#070b10');
        }
    },

    async setTheme(theme) {
        const themeStr = String(theme || 'system').toLowerCase();
        this.applyTheme(themeStr);

        // Update active UI classes immediately
        const themeOptions = document.querySelectorAll('.theme-option[data-theme]');
        themeOptions.forEach(opt => {
            opt.classList.toggle('active', opt.dataset.theme === themeStr);
        });

        // Update in-memory settings
        this.currentSettings = this.currentSettings || {};
        this.currentSettings.appearance = this.currentSettings.appearance || {};
        this.currentSettings.appearance.theme = themeStr;

        // Persist to SQLite backend
        try {
            if (this.currentSettings.audio && this.currentSettings.downloads && this.currentSettings.sync && this.currentSettings.library) {
                await this.invoke('update_settings', { settings: this.currentSettings });
            } else {
                const fullSettings = await this.invoke('get_settings');
                if (fullSettings) {
                    this.currentSettings = fullSettings;
                    this.currentSettings.appearance = this.currentSettings.appearance || {};
                    this.currentSettings.appearance.theme = themeStr;
                    await this.invoke('update_settings', { settings: this.currentSettings });
                }
            }
        } catch (err) {
            console.warn('Failed to persist theme setting:', err);
        }
    },

    formatTime(secs) {
        const m = Math.floor(secs / 60);
        const s = Math.floor(secs % 60);
        return `${m}:${s.toString().padStart(2, '0')}`;
    },

    escapeHtml(str) {
        if (!str) return '';
        return str.replace(/[&<>"']/g, match => ({
            '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;'
        }[match]));
    },

    assetUrl(path) {
        if (!path) return '';
        if (/^(https?:|data:|blob:|asset:)/.test(path)) return path;
        const internals = window.__TAURI_INTERNALS__;
        if (internals && typeof internals.convertFileSrc === 'function') {
            try {
                return internals.convertFileSrc(path);
            } catch (_) {}
        }
        if (window.__TAURI__ && window.__TAURI__.core && typeof window.__TAURI__.core.convertFileSrc === 'function') {
            try {
                return window.__TAURI__.core.convertFileSrc(path);
            } catch (_) {}
        }
        return path;
    },

    async embedArt(imgEl, path) {
        if (!imgEl || !path) return;
        try {
            const dataUri = await this.invoke('media_data_url', { path });
            if (dataUri) {
                imgEl.src = dataUri;
                return;
            }
            this._artworkFailed(imgEl, path, 'media_data_url returned nothing');
        } catch (err) {
            // A missing/broken cover used to leave the browser's broken-image
            // icon in every library card, with the reason only in a console
            // nobody can read on a release build. Report it once, then fall
            // back to the neutral placeholder so the card still looks right.
            this._artworkFailed(imgEl, path, (err && err.message) || String(err));
        }
    },

    _artworkFailed(imgEl, path, reason) {
        console.error('[Auralis] cover art unavailable', path, reason);
        try {
            window.__auralisArtworkFailures = window.__auralisArtworkFailures || [];
            window.__auralisArtworkFailures.push({ path, reason, at: new Date().toISOString() });
        } catch (_) {}
        if (imgEl && imgEl.parentNode) {
            // Replace the broken <img> with the same neutral icon the templates
            // use for a track without artwork.
            const holder = document.createElement('div');
            holder.className = 'art-placeholder';
            holder.style.cssText = 'width:100%;height:100%;display:flex;align-items:center;justify-content:center;opacity:.5';
            holder.innerHTML = '<i data-lucide="music" style="width:28px;height:28px"></i>';
            imgEl.parentNode.replaceChild(holder, imgEl);
            try {
                if (window.lucide && typeof window.lucide.createIcons === 'function') {
                    window.lucide.createIcons();
                }
            } catch (_) {}
        }
    },

    artImgTag(path, altText) {
        if (!path) return '';
        const safeAlt = this.escapeHtml(altText || '');
        const src = this.assetUrl(path);
        const jsonPath = JSON.stringify(path).replace(/</g, '\\u003c').replace(/"/g, '&quot;');
        return `<img src="${src}" alt="${safeAlt}" onerror="if(!this.dataset.fb){this.dataset.fb='1';window.Auralis.bridge.embedArt(this, ${jsonPath})}">`;
    },

    showToast(message, type = 'info', duration = 3500) {
        const container = document.getElementById('toast-container') || document.body;
        if (!container || !document.createElement) return;
        const toast = document.createElement('div');
        toast.className = `toast toast-${type}`;
        toast.textContent = message;
        container.appendChild(toast);

        const dismiss = () => {
            if (toast.classList.contains('toast-out')) return;
            toast.classList.add('toast-out');
            toast.addEventListener('animationend', () => {
                if (toast.parentElement) toast.remove();
            }, { once: true });
            setTimeout(() => {
                if (toast.parentElement) toast.remove();
            }, 400);
        };

        setTimeout(dismiss, duration);
        return toast;
    }
};
