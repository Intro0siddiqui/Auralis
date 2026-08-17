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
            toast.className = 'toast toast-info toast-scan';
            container.appendChild(toast);
        }

        const fileName = file ? file.split(/[\\/]/).pop() : (payload.subtitle || 'Scanning...');
        const countText = total ? `${current} / ${total}` : `${current} files`;
        const pctText = percentage !== null ? ` (${percentage}%)` : '';
        const pctVal = percentage !== null ? Math.min(100, Math.max(0, percentage)) : 100;

        toast.classList.remove('toast-out');
        toast.innerHTML = `
            <div class="toast-scan-header">
                <span class="toast-scan-title">Scanning audio...</span>
                <span class="toast-scan-count">${countText}${pctText}</span>
            </div>
            <div class="toast-scan-filename">
                ${this.escapeHtml(fileName)}
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

        const box = document.getElementById('library-scan-logbox');
        if (box) box.style.display = 'block';

        const content = document.getElementById('library-scan-log-content');
        if (content) {
            const line = document.createElement('div');
            const time = new Date().toLocaleTimeString();
            line.className = 'scan-log-line';
            if (msg.includes('❌') || msg.includes('Error') || msg.includes('failed')) {
                line.classList.add('scan-log-error');
            } else if (msg.includes('⚠️') || msg.includes('Warning')) {
                line.classList.add('scan-log-warning');
            } else if (msg.includes('✅') || msg.includes('🎉')) {
                line.classList.add('scan-log-success');
            } else if (msg.includes('🎵') || msg.includes('📂')) {
                line.classList.add('scan-log-info');
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
            toast.classList.add('toast-out');
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
            toast.classList.add('toast-out');
            setTimeout(() => {
                if (toast && toast.parentElement) toast.remove();
            }, 300);
        }
    }
};
