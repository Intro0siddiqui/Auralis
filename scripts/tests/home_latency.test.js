import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'fs';
import path from 'path';

describe('Home Page Latency & Event Dispatch Optimization Guards', () => {
    const rootDir = path.resolve(import.meta.dirname ?? path.dirname(new URL(import.meta.url).pathname), '../..');

    it('core.js registers view:enter only on canonical bridge listener without duplicate window/document listeners', () => {
        const coreSrc = fs.readFileSync(path.join(rootDir, 'ui/js/modules/core.js'), 'utf8');
        // Check that window.addEventListener('view:enter') and document.addEventListener('view:enter')
        // are not both attached alongside this.on('view:enter')
        assert.ok(!coreSrc.includes("document.addEventListener('view:enter'"), 'Must not attach view:enter listener to document');
        assert.ok(coreSrc.includes('this._viewEventsBound'), 'Must guard view events against duplicate binding');
    });

    it('views.js loadHomeView checks DOM populated state and dirty flag before re-fetching shelves', () => {
        const viewsSrc = fs.readFileSync(path.join(rootDir, 'ui/js/modules/views.js'), 'utf8');
        assert.ok(viewsSrc.includes('this._homeDirty'), 'loadHomeView must consult _homeDirty flag');
        assert.ok(viewsSrc.includes('this._homeFetchPromise'), 'loadHomeView must deduplicate in-flight fetch promises');
        assert.ok(viewsSrc.includes('dataset.populated'), 'loadHomeView must mark and check container populated dataset attribute');
    });

    it('html.rs renders asset protocol URLs in img src rather than failing raw file paths', () => {
        const htmlSrc = fs.readFileSync(path.join(rootDir, 'src/commands/library/html.rs'), 'utf8');
        assert.ok(htmlSrc.includes('to_asset_url'), 'html.rs must implement to_asset_url helper');
        assert.ok(htmlSrc.includes('loading="lazy"'), 'render_art_tag must specify loading="lazy"');
        assert.ok(htmlSrc.includes('decoding="async"'), 'render_art_tag must specify decoding="async"');
        assert.ok(!htmlSrc.includes('src="{safe_path}"'), 'render_art_tag must not render raw filesystem path as src');
    });

    it('ui.js embedArt tries assetUrl before falling back to media_data_url IPC', () => {
        const uiSrc = fs.readFileSync(path.join(rootDir, 'ui/js/modules/ui.js'), 'utf8');
        assert.ok(uiSrc.includes('this.assetUrl(path)'), 'embedArt must test assetUrl before media_data_url IPC');
    });
});
