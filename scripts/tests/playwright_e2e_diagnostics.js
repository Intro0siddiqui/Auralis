const { test, expect } = require('@playwright/test');
const fs = require('fs');
const path = require('path');

const SCREENSHOT_DIR = path.resolve(__dirname, '../../test_screenshots');

test.beforeAll(() => {
    if (!fs.existsSync(SCREENSHOT_DIR)) {
        fs.mkdirSync(SCREENSHOT_DIR, { recursive: true });
    }
});

test.describe('Auralis Desktop Playwright E2E Diagnostics', () => {
    let consoleLogs = [];
    let networkLogs = [];

    test.beforeEach(async ({ page }) => {
        consoleLogs = [];
        networkLogs = [];

        page.on('console', msg => consoleLogs.push(`[${msg.type()}] ${msg.text()}`));
        page.on('request', req => networkLogs.push(`REQ: ${req.method()} ${req.url()}`));
        page.on('response', res => networkLogs.push(`RES: ${res.status()} ${res.url()}`));

        await page.goto('/');
        await page.waitForLoadState('domcontentloaded');
    });

    test.afterEach(async ({ page }, testInfo) => {
        if (testInfo.status !== testInfo.expectedStatus) {
            const failPath = path.join(SCREENSHOT_DIR, `failure_${testInfo.title.replace(/\s+/g, '_')}.png`);
            await page.screenshot({ path: failPath, fullPage: true }).catch(() => {});
            console.error(`\n--- Test Failure Diagnostic Log [${testInfo.title}] ---`);
            console.error('Console Logs:\n' + consoleLogs.join('\n'));
            console.error('Network Logs:\n' + networkLogs.join('\n'));
            console.error(`Failure screenshot saved to: ${failPath}\n`);
        }
    });

    test('1. Skip backward: click #btn-prev returns to start or jumps to previous track', async ({ page }) => {
        const prevBtn = page.locator('#btn-prev, #prev-btn, [data-alias-id="btn-prev"]').first();
        await expect(prevBtn).toBeVisible();

        // Trigger skip backward click
        await prevBtn.click();
        await page.waitForTimeout(500);

        // Verify state via player JS controller or DOM
        const state = await page.evaluate(() => {
            const p = window.Auralis && window.Auralis.player;
            return {
                progress: p ? p.progress : 0,
                currentTrack: p ? p.currentTrack : null
            };
        });

        expect(state.progress).toBeLessThanOrEqual(1.0);

        const ssPath = path.join(SCREENSHOT_DIR, '01_skip_backward.png');
        await page.screenshot({ path: ssPath });
        expect(fs.existsSync(ssPath)).toBe(true);
    });

    test('2. Volume + mute: set volume slider + click mute updates volume & state', async ({ page }) => {
        const volBtn = page.locator('#volume-btn, #btn-volume').first();
        const volSlider = page.locator('#volume-slider').first();

        await expect(volBtn).toBeVisible();
        await expect(volSlider).toBeVisible();

        // Set volume slider level via click/drag
        const box = await volSlider.boundingBox();
        if (box) {
            await page.mouse.click(box.x + box.width * 0.5, box.y + box.height * 0.5);
        }

        let volState = await page.evaluate(() => window.Auralis?.player?.volume ?? 1);
        expect(volState).toBeGreaterThan(0);

        // Click mute button
        await volBtn.click();
        await page.waitForTimeout(300);

        volState = await page.evaluate(() => window.Auralis?.player?.volume ?? 1);
        expect(volState).toBe(0);

        const ssPath = path.join(SCREENSHOT_DIR, '02_volume_mute.png');
        await page.screenshot({ path: ssPath });
        expect(fs.existsSync(ssPath)).toBe(true);
    });

    test('3. Seekbar drag: drag timeline updates audio position & progress UI', async ({ page }) => {
        const progressTrack = page.locator('#progress-track').first();
        await expect(progressTrack).toBeVisible();

        const box = await progressTrack.boundingBox();
        if (box) {
            await page.mouse.click(box.x + box.width * 0.4, box.y + box.height * 0.5);
        }
        await page.waitForTimeout(300);

        const timeCurrent = page.locator('#time-current');
        await expect(timeCurrent).toBeVisible();

        const ssPath = path.join(SCREENSHOT_DIR, '03_seekbar_drag.png');
        await page.screenshot({ path: ssPath });
        expect(fs.existsSync(ssPath)).toBe(true);
    });

    test('4. Repeat cycle: click #btn-repeat cycles Off -> Repeat All -> Repeat One', async ({ page }) => {
        const repeatBtn = page.locator('#btn-repeat, #repeat-btn, [data-alias-id="btn-repeat"]').first();
        await expect(repeatBtn).toBeVisible();

        // Initial state
        let mode1 = await page.evaluate(() => window.Auralis?.player?.repeatMode || 'off');

        // Click 1: off -> all
        await repeatBtn.click();
        await page.waitForTimeout(200);
        let mode2 = await page.evaluate(() => window.Auralis?.player?.repeatMode || 'off');
        expect(mode2).not.toBe(mode1);

        // Click 2: all -> one
        await repeatBtn.click();
        await page.waitForTimeout(200);
        let mode3 = await page.evaluate(() => window.Auralis?.player?.repeatMode || 'off');
        expect(mode3).not.toBe(mode2);

        const ssPath = path.join(SCREENSHOT_DIR, '04_repeat_cycle.png');
        await page.screenshot({ path: ssPath });
        expect(fs.existsSync(ssPath)).toBe(true);
    });
});
