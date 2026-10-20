// Run after building the extension. PLAYWRIGHT_MODULE / PLAYWRIGHT_CHROMIUM_EXECUTABLE
// can point at an existing local browser automation installation; no user profile is used.
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const {existsSync} = require('node:fs');
const {chromium} = require(process.env.PLAYWRIGHT_MODULE || 'playwright');

(async () => {
  const root = process.cwd();
  const extension = path.resolve(process.env.NEXUS_EXTENSION_PATH || (existsSync('browser-extension/chromium')
    ? 'browser-extension/chromium' : 'Ghost-Downloader-3-main/browser_extension/chromium'));
  const bundle = await fs.readFile(path.join(extension, 'catch-script/overlay-media-state.js'), 'utf8');
  const overlay = await fs.readFile(path.join(extension, 'catch-script/media-download-overlay.js'), 'utf8');
  // Minimal reproduction of the pre-fix lifecycle: a world-local singleton,
  // shared DOM host, and a timer that reattaches it after removal.
  const legacy = `(() => {
    if (window.GhostDownloaderMediaButton?.installed) return;
    const host = document.createElement('div'); host.id = 'ghostDownloaderMediaDownload';
    host.attachShadow({mode: 'open'}).innerHTML = '<button>下载此媒体</button>';
    Object.assign(host.style, {position: 'fixed', top: '112px', left: '420px'});
    const update = () => document.documentElement.appendChild(host);
    update(); setInterval(update, 1000);
    window.GhostDownloaderMediaButton = {installed: true};
  })();`;
  const browser = await chromium.launch({headless: true, ...(process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE ? {executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE} : {})});
  try {
    const page = await browser.newPage({viewport: {width: 1280, height: 720}});
    await page.route('https://www.douyin.com/**', r => r.fulfill({contentType: 'text/html', body: '<!doctype html><title>Overlay lifecycle fixture</title><style>body{margin:0;display:flex;gap:30px;padding-top:100px}video{width:600px;height:360px;background:#333}</style><video controls></video><video controls></video><div id="unrelated">keep</div>'}));
    await page.goto('https://www.douyin.com/video/1111111111111111111');
    const cdp = await page.context().newCDPSession(page);
    const {frameTree} = await cdp.send('Page.getFrameTree');
    let generation = 0;
    async function install(source) {
      const {executionContextId} = await cdp.send('Page.createIsolatedWorld', {frameId: frameTree.frame.id, worldName: `extension-generation-${++generation}`});
      const result = await cdp.send('Runtime.evaluate', {contextId: executionContextId, expression: `
        globalThis.chrome = {runtime: {id: 'fixture', sendMessage(message, callback) {
          const result = {enabled: true}; callback?.(result); return Promise.resolve(result);
        }, onMessage: {addListener() {}, removeListener() {}}}};
        ${bundle}\n${source}
      `});
      assert.ok(!result.exceptionDetails, JSON.stringify(result.exceptionDetails));
      await page.waitForTimeout(1100);
      return executionContextId;
    }
    const hosts = page.locator('#ghostDownloaderMediaDownload');
    // Reproduce extension reload: a new isolated world, same shared page DOM.
    await install(legacy);
    await install(legacy);
    assert.equal(await hosts.count(), 2, 'legacy worlds reproduce the reported duplicate');
    console.log('Reproduced: two legacy isolated worlds leave two buttons.');
    let contextId = await install(overlay);
    await page.waitForTimeout(2500); // Legacy timers try to resurrect their hosts repeatedly.
    assert.equal(await hosts.count(), 1, 'new owner removes resurrected legacy hosts');
    for (let i = 0; i < 3; i++) {
      contextId = await install(overlay);
      assert.equal(await hosts.count(), 1, 'repeated extension replacements retain one button');
    }
    // Re-executing in the same world remains idempotent as well.
    const repeat = await cdp.send('Runtime.evaluate', {contextId, expression: overlay});
    assert.ok(!repeat.exceptionDetails);
    await page.waitForTimeout(1500);
    assert.equal(await hosts.count(), 1);
    assert.equal(await page.locator('#unrelated').textContent(), 'keep');
    // Opening comments shrinks the video, not the browser viewport. Both
    // placement and the auto-hide wake zone must follow that actual video.
    const video = page.locator('video').first();
    await video.evaluate(e => Object.assign(e.style, {position: 'absolute', left: '0', top: '0', width: '960px', height: '720px'}));
    await page.waitForTimeout(1200);
    const button = hosts.getByRole('button');
    let bounds = await button.boundingBox();
    assert.ok(bounds.x + bounds.width <= 960 && bounds.x > 700, 'button stays out of the comments sidebar');
    await page.mouse.move(1100, 40);
    await page.waitForTimeout(2000);
    assert.equal(await hosts.evaluate(e => getComputedStyle(e).opacity), '0');
    await page.mouse.move(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2);
    await page.waitForFunction(() => getComputedStyle(document.querySelector('#ghostDownloaderMediaDownload')).opacity === '1');
    await video.evaluate(e => e.style.width = '720px');
    await page.waitForTimeout(200);
    bounds = await button.boundingBox();
    assert.ok(bounds.x + bounds.width <= 720, 'resize observer follows video shrink without window resize');
    await video.evaluate(e => e.style.width = '1280px');
    await page.waitForTimeout(200);
    bounds = await button.boundingBox();
    assert.ok(bounds.x > 1100, 'button follows the expanded video');
    const output = path.join(root, '.release/output/playwright/overlay-lifecycle');
    await fs.mkdir(output, {recursive: true});
    await page.screenshot({path: path.join(output, 'single-overlay.png')});
    await cdp.send('Runtime.evaluate', {contextId, expression: 'chrome.runtime.id = undefined'});
    await page.waitForTimeout(2200);
    assert.equal(await hosts.evaluateAll(elements => elements.filter(e => getComputedStyle(e).display !== 'none').length), 0,
      'invalidated extension and legacy timers cannot leave visible dead buttons');
    await install(overlay);
    assert.equal(await hosts.count(), 1, 'new owner recovers after invalidation');
    console.log('Passed: legacy migration, timer resurrection, three replacements, repeated injection, context invalidation, recovery, sidebar-safe player anchoring, resize tracking, hover wake, unrelated DOM preserved.');
  } finally {await browser.close();}
})().catch(error => {console.error(error.stack); process.exitCode = 1;});
