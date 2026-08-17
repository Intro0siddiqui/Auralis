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
        const metaThemeColor = document.querySelector ? document.querySelector('meta[name="theme-color"]') : null;
        if (metaThemeColor && typeof metaThemeColor.setAttribute === 'function') {
            metaThemeColor.setAttribute('content', activeTheme === 'light' ? '#f0f4f8' : '#070b10');
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
            if (dataUri) imgEl.src = dataUri;
        } catch (err) {
            console.error('Cover art fallback failed:', err);
        }
    },

    artImgTag(path, altText) {
        if (!path) return '';
        const safeAlt = this.escapeHtml(altText || '');
        const src = this.assetUrl(path);
        const jsonPath = JSON.stringify(path).replace(/</g, '\\u003c');
        return `<img src="${src}" alt="${safeAlt}" onerror="if(!this.dataset.fb){this.dataset.fb='1';window.Auralis.bridge.embedArt(this, ${jsonPath})}">`;
    },

    showToast(message, type = 'info') {
        const container = document.getElementById('toast-container') || document.body;
        if (!container || !document.createElement) return;
        const toast = document.createElement('div');
        toast.className = `toast toast-${type} glass`;
        toast.style.cssText = `
            position: fixed; top: calc(20px + env(safe-area-inset-top, 0px)); right: 20px; z-index: 1000;
            padding: 12px 20px; border-radius: 12px; background: rgba(11, 17, 24, 0.94);
            color: var(--text-1); border: 1px solid var(--glass-border); box-shadow: var(--shadow-lg);
            font-size: var(--text-sm); font-weight: 500;
        `;
        toast.textContent = message;
        container.appendChild(toast);
        setTimeout(() => {
            toast.style.opacity = '0';
            toast.style.transition = 'opacity 300ms ease';
            setTimeout(() => toast.remove(), 300);
        }, 3500);
    }
};
