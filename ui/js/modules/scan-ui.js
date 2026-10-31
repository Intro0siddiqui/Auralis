/**
 * Scan UI Module
 * Handles progress bars, toast indicators, scan log displays and UI feedback.
 */

import { copyWithToast } from './clipboard.js';

export const scanUiMethods = {
    _scanBannerTimeout: null,

    updateScanProgressUI(payload) {
        if (!payload) return;
        const current = payload.current ?? payload.scanned ?? payload.processed ?? payload.processed_files ?? payload.processedFiles ?? payload.count ?? 0;
        const total = payload.total ?? payload.total_files ?? payload.totalFiles ?? null;
        const file = payload.file ?? payload.path ?? payload.filename ?? payload.current_file ?? payload.currentFile ?? payload.title ?? '';
        const percentage = payload.percentage !== undefined && payload.percentage !== null
            ? payload.percentage
            : (total && total > 0 ? Math.round((current / total) * 100) : (payload.progress ? Math.round(payload.progress * 100) : null));

        const banner = document.getElementById('library-scan-progress');
        const titleEl = document.getElementById('library-scan-title');
        const subEl = document.getElementById('library-scan-subtitle');
        const counterEl = document.getElementById('library-scan-counter');
        const barEl = document.getElementById('library-scan-bar');

        if (banner) {
            if (this._scanBannerTimeout) {
                clearTimeout(this._scanBannerTimeout);
                this._scanBannerTimeout = null;
            }
            banner.style.display = 'flex';

            // Ensure loader icon is active during scanning
            const iconContainer = banner.querySelector('.scan-icon-container') || banner.querySelector('[data-lucide]')?.parentElement;
            if (iconContainer && !iconContainer.querySelector('.spin')) {
                iconContainer.innerHTML = '<i data-lucide="loader-2" style="width: 18px; height: 18px; color: var(--accent); flex-shrink: 0;" class="spin"></i>';
                if (window.lucide) window.lucide.createIcons();
            }

            if (titleEl) {
                titleEl.textContent = payload.title || (total ? `Scanning (${current}/${total})` : `Scanning (${current} files)`);
            }
            if (subEl) {
                const fileName = file ? file.split(/[\\/]/).pop() : (payload.subtitle || 'Processing audio files...');
                subEl.textContent = fileName;
            }
            if (counterEl) {
                counterEl.textContent = total ? `${current} / ${total}` : `${current} files`;
            }
            if (barEl) {
                if (percentage !== null) {
                    barEl.style.width = `${Math.min(100, Math.max(0, percentage))}%`;
                } else {
                    barEl.style.width = '100%';
                }
            }
        }

        this.updateScanToast(payload, current, total, percentage, file);
    },

    updateScanToast(payload, current, total, percentage, file) {
        const container = document.getElementById('toast-container') || document.body;
        let toast = document.getElementById('scan-progress-toast');
        if (!toast) {
            toast = document.createElement('div');
            toast.id = 'scan-progress-toast';
            toast.className = 'toast toast-info toast-scan';
            container.appendChild(toast);
        }

        const fileName = file ? file.split(/[\\/]/).pop() : (payload.subtitle || 'Scanning...');
        const countText = total ? `${current} / ${total}` : `${current} files`;
        const pctText = percentage !== null ? ` (${percentage}%)` : '';
        const pctVal = percentage !== null ? Math.min(100, Math.max(0, percentage)) : 100;
        const safeFileName = typeof this.escapeHtml === 'function' ? this.escapeHtml(fileName) : fileName;

        toast.classList.remove('toast-out');
        toast.innerHTML = `
            <div class="toast-scan-header">
                <span class="toast-scan-title">Scanning audio...</span>
                <span class="toast-scan-count">${countText}${pctText}</span>
            </div>
            <div class="toast-scan-filename">
                ${safeFileName}
            </div>
            <div class="progress-track neu-inset toast-scan-track">
                <div class="progress-fill toast-scan-fill" style="width: ${pctVal}%;"></div>
            </div>
        `;
    },

    appendScanLog(msg) {
        if (!msg) return;
        const banner = document.getElementById('library-scan-progress');
        if (banner) banner.style.display = 'flex';

        const content = document.getElementById('library-scan-log-content');
        if (content) {
            const line = document.createElement('div');
            const time = new Date().toLocaleTimeString();
            line.className = 'scan-log-line';
            if (msg.includes('❌') || msg.includes('Error') || msg.includes('failed')) {
                line.classList.add('scan-log-error');
                // Auto-show logbox only on actual errors so user can see what failed
                const box = document.getElementById('library-scan-logbox');
                if (box) box.style.display = 'block';
            } else if (msg.includes('⚠️') || msg.includes('Warning')) {
                line.classList.add('scan-log-warning');
            } else if (msg.includes('✅') || msg.includes('🎉')) {
                line.classList.add('scan-log-success');
            } else if (msg.includes('🎵') || msg.includes('📂')) {
                line.classList.add('scan-log-info');
            }
            line.textContent = `[${time}] ${msg}`;
            content.appendChild(line);
            const box = document.getElementById('library-scan-logbox');
            if (box && box.style.display !== 'none') box.scrollTop = box.scrollHeight;
        }
    },

    toggleScanLogs() {
        const box = document.getElementById('library-scan-logbox');
        if (box) {
            box.style.display = box.style.display === 'none' ? 'block' : 'none';
        }
    },

    copyScanLogs() {
        const content = document.getElementById('library-scan-log-content');
        if (content) {
            const text = content.innerText || content.textContent;
            copyWithToast({ text, label: 'diagnostic logs', showToast: (m, k) => this.showToast(m, k) });
        }
    },

    finishScanProgressUI(payload) {
        if (this._scanBannerTimeout) {
            clearTimeout(this._scanBannerTimeout);
            this._scanBannerTimeout = null;
        }

        const banner = document.getElementById('library-scan-progress');
        const titleEl = document.getElementById('library-scan-title');
        const subEl = document.getElementById('library-scan-subtitle');
        const counterEl = document.getElementById('library-scan-counter');
        const barEl = document.getElementById('library-scan-bar');
        const iconContainer = banner ? (banner.querySelector('.scan-icon-container') || banner.querySelector('[data-lucide]')?.parentElement) : null;

        const added = (payload && (payload.tracks_added ?? payload.added)) || 0;
        const updated = (payload && (payload.tracks_updated ?? payload.updated)) || 0;
        const errors = (payload && payload.errors && payload.errors.length) || 0;

        if (titleEl) titleEl.textContent = 'Scan Complete';
        if (counterEl) counterEl.textContent = 'Complete';
        if (barEl) barEl.style.width = '100%';

        if (subEl) {
            if (errors > 0) {
                subEl.textContent = `${added} added • ${errors} error(s)`;
            } else if (updated > 0) {
                subEl.textContent = `${added} added • ${updated} updated`;
            } else {
                subEl.textContent = `${added} tracks added`;
            }
        }

        if (iconContainer) {
            const iconName = errors > 0 ? 'alert-triangle' : 'check-circle-2';
            const iconColor = errors > 0 ? 'var(--warning, #eab308)' : 'var(--accent-success, #22c55e)';
            iconContainer.innerHTML = `<i data-lucide="${iconName}" style="width: 18px; height: 18px; color: ${iconColor}; flex-shrink: 0;"></i>`;
            if (window.lucide) window.lucide.createIcons();
        }

        const toast = document.getElementById('scan-progress-toast');
        if (toast) {
            toast.classList.add('toast-out');
            setTimeout(() => {
                if (toast && toast.parentElement) toast.remove();
            }, 300);
        }

        // Auto-dismiss the banner after 3.5 seconds if there are no errors
        if (banner && errors === 0) {
            this._scanBannerTimeout = setTimeout(() => {
                if (banner) {
                    banner.style.display = 'none';
                    const freshLogbox = document.getElementById('library-scan-logbox');
                    if (freshLogbox) freshLogbox.style.display = 'none';
                }
            }, 3500);
        }
    },

    hideScanProgressUI() {
        if (this._scanBannerTimeout) {
            clearTimeout(this._scanBannerTimeout);
            this._scanBannerTimeout = null;
        }
        const banner = document.getElementById('library-scan-progress');
        if (banner) {
            banner.style.display = 'none';
        }
        const toast = document.getElementById('scan-progress-toast');
        if (toast) {
            toast.classList.add('toast-out');
            setTimeout(() => {
                if (toast && toast.parentElement) toast.remove();
            }, 300);
        }
    }
};
