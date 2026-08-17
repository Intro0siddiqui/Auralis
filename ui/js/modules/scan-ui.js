/**
 * Scan UI Module
 * Handles progress bars, toast indicators, scan log displays and UI feedback.
 */

export const scanUiMethods = {
    updateScanProgressUI(payload) {
        if (!payload) return;
        const current = payload.current ?? payload.scanned ?? payload.processed ?? payload.count ?? 0;
        const total = payload.total ?? payload.total_files ?? null;
        const file = payload.file ?? payload.path ?? payload.filename ?? payload.current_file ?? payload.title ?? '';
        const percentage = payload.percentage !== undefined && payload.percentage !== null
            ? payload.percentage
            : (total && total > 0 ? Math.round((current / total) * 100) : (payload.progress ? Math.round(payload.progress * 100) : null));

        const banner = document.getElementById('library-scan-progress');
        const titleEl = document.getElementById('library-scan-title');
        const subEl = document.getElementById('library-scan-subtitle');
        const counterEl = document.getElementById('library-scan-counter');
        const barEl = document.getElementById('library-scan-bar');

        if (banner) {
            banner.style.display = 'flex';
            if (titleEl) {
                titleEl.textContent = payload.title || (total ? `Scanning storage (${current}/${total})` : `Scanning storage (${current} files)`);
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
            toast.className = 'toast toast-info glass';
            toast.style.cssText = `
                position: fixed; top: calc(20px + env(safe-area-inset-top, 0px)); right: 20px; z-index: 1000;
                padding: 12px 20px; border-radius: 12px; background: rgba(11, 17, 24, 0.94);
                color: var(--text-1); border: 1px solid var(--glass-border); box-shadow: var(--shadow-lg);
                font-size: var(--text-sm); font-weight: 500; min-width: 240px; display: flex; flex-direction: column; gap: 6px;
                transition: opacity 300ms ease;
            `;
            container.appendChild(toast);
        }

        const fileName = file ? file.split(/[\\/]/).pop() : (payload.subtitle || 'Scanning...');
        const countText = total ? `${current} / ${total}` : `${current} files`;
        const pctText = percentage !== null ? ` (${percentage}%)` : '';

        toast.innerHTML = `
            <div style="display: flex; justify-content: space-between; align-items: center;">
                <span style="font-weight: 600;">Scanning audio...</span>
                <span style="color: var(--accent); font-size: var(--text-xs); font-weight: 600;">${countText}${pctText}</span>
            </div>
            <div style="font-size: var(--text-xs); color: var(--text-3); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; max-width: 260px;">
                ${this.escapeHtml(fileName)}
            </div>
            <div class="progress-track neu-inset" style="width: 100%; height: 4px; margin-top: 2px;">
                <div class="progress-fill" style="width: ${percentage !== null ? percentage : 100}%; background: var(--accent); height: 100%; transition: width 0.2s ease;"></div>
            </div>
        `;
        toast.style.opacity = '1';
    },

    appendScanLog(msg) {
        if (!msg) return;
        const banner = document.getElementById('library-scan-progress');
        if (banner) banner.style.display = 'flex';

        const box = document.getElementById('library-scan-logbox');
        if (box) box.style.display = 'block';

        const content = document.getElementById('library-scan-log-content');
        if (content) {
            const line = document.createElement('div');
            const time = new Date().toLocaleTimeString();
            line.style.cssText = 'white-space: pre-wrap; word-break: break-all; margin-bottom: 2px;';
            if (msg.includes('❌') || msg.includes('Error') || msg.includes('failed')) {
                line.style.color = '#ef4444';
            } else if (msg.includes('⚠️') || msg.includes('Warning')) {
                line.style.color = '#f59e0b';
            } else if (msg.includes('✅') || msg.includes('🎉')) {
                line.style.color = '#10b981';
            } else if (msg.includes('🎵') || msg.includes('📂')) {
                line.style.color = '#38bdf8';
            }
            line.textContent = `[${time}] ${msg}`;
            content.appendChild(line);
            if (box) box.scrollTop = box.scrollHeight;
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
            navigator.clipboard.writeText(text).then(() => {
                this.showToast('Diagnostic logs copied to clipboard!', 'success');
            }).catch(() => {
                this.showToast('Failed to copy logs', 'error');
            });
        }
    },

    finishScanProgressUI(payload) {
        const titleEl = document.getElementById('library-scan-title');
        const subEl = document.getElementById('library-scan-subtitle');
        const icon = document.querySelector('#library-scan-progress i');

        if (titleEl) titleEl.textContent = 'Scan Complete';
        if (subEl) {
            const added = (payload && payload.tracks_added) || 0;
            const errors = (payload && payload.errors && payload.errors.length) || 0;
            subEl.textContent = `${added} tracks added • ${errors} errors`;
        }
        if (icon) {
            icon.classList.remove('spin');
            icon.setAttribute('data-lucide', 'check-circle-2');
            if (window.lucide) window.lucide.createIcons();
        }

        const toast = document.getElementById('scan-progress-toast');
        if (toast) {
            toast.style.opacity = '0';
            setTimeout(() => {
                if (toast && toast.parentElement) toast.remove();
            }, 300);
        }
    },

    hideScanProgressUI() {
        const banner = document.getElementById('library-scan-progress');
        if (banner) {
            banner.style.display = 'none';
        }
        const toast = document.getElementById('scan-progress-toast');
        if (toast) {
            toast.style.opacity = '0';
            setTimeout(() => {
                if (toast && toast.parentElement) toast.remove();
            }, 300);
        }
    }
};
