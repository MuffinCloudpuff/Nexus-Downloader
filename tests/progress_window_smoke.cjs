// Real browser download -> installed Rust daemon -> automatic toolbar popup.
// Uses a fresh profile, local fixture files, and deletes only tasks created here.
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const {existsSync} = require('node:fs');
const path = require('node:path');
const {spawn} = require('node:child_process');
const {createServer} = require('node:http');
const {chromium} = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const sleep = ms => new Promise(r => setTimeout(r, ms));
const root = process.cwd(), stamp = String(Date.now());
const out = path.join(root, 'output/playwright/progress-window', stamp);
const extension = path.resolve(process.env.NEXUS_EXTENSION_PATH || 'browser-extension/chromium');
const portable = path.resolve(process.env.NEXUS_PORTABLE_DIR || 'dist-portable/NexusDownloaderPortable');
const executable = process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE || chromium.executablePath();
const owned = new Set();
let browser, processHandle, server, token, allowCompletion = false;
async function api(route, body) {
  const response = await fetch('http://127.0.0.1:14371/api/' + route, {
    signal: AbortSignal.timeout(10000), method: body ? 'POST' : 'GET', headers: {Authorization: 'Bearer ' + token, 'Content-Type': 'application/json'},
    ...(body ? {body: JSON.stringify(body)} : {})});
  assert.ok(response.ok, `Fixture API ${response.status}`);
  return response.json();
}
async function until(fn, description) {
  for (let i = 0; i < 300; i++) { const result = await fn(); if (result) return result; await sleep(100); }
  throw Error('Timed out: ' + description);
}
// Toolbar popups are extension views, not tabs. Playwright's tab collection
// excludes them, so inspect their actual renderer using its CDP endpoint.
async function toolbarPanel(endpoint, worker, taskId) {
  const view = await until(async () => (await worker.evaluate(() => chrome.runtime.getContexts({contextTypes: ['POPUP']})))
    .find(c => taskId === true ? c.documentUrl.includes('?view=progress&task=')
      : taskId ? c.documentUrl.includes(`task=${encodeURIComponent(taskId)}`) : c.documentUrl.endsWith('/popup.html')), 'toolbar POPUP context');
  const target = await until(async () => (await (await fetch(endpoint + '/json/list')).json()).find(t => t.url === view.documentUrl), 'popup renderer');
  const socket = new WebSocket(target.webSocketDebuggerUrl), pending = new Map();
  let sequence = 0;
  await new Promise((resolve, reject) => { socket.addEventListener('open', resolve, {once: true}); socket.addEventListener('error', reject, {once: true}); });
  socket.addEventListener('message', event => {
    const message = JSON.parse(event.data), wait = pending.get(message.id);
    if (!wait) return;
    pending.delete(message.id); clearTimeout(wait.timer);
    message.error ? wait.reject(Error(message.error.message)) : wait.resolve(message.result);
  });
  socket.addEventListener('close', () => {
    for (const wait of pending.values()) { clearTimeout(wait.timer); wait.reject(Error('Popup closed')); }
    pending.clear();
  });
  const send = (method, params = {}) => new Promise((resolve, reject) => {
    const id = ++sequence;
    const timer = setTimeout(() => { pending.delete(id); reject(Error('CDP timeout: ' + method)); }, 10000);
    pending.set(id, {resolve, reject, timer}); socket.send(JSON.stringify({id, method, params}));
  });
  const evaluate = async expression => {
    const result = await send('Runtime.evaluate', {expression, returnByValue: true, awaitPromise: true});
    if (result.exceptionDetails) throw Error(result.exceptionDetails.text);
    return result.result.value;
  };
  return {
    text: () => evaluate('document.body.innerText'),
    click: async label => {
      await until(() => evaluate(`(() => { const button = [...document.querySelectorAll('button')].find(b => b.textContent.trim() === ${JSON.stringify(label)} || b.getAttribute('aria-label') === ${JSON.stringify(label)}); if (!button || button.disabled) return false; button.click(); return true; })()`), 'enabled button: ' + label);
    },
    screenshot: async file => {
      const image = await send('Page.captureScreenshot', {format: 'png'}); await fs.writeFile(file, Buffer.from(image.data, 'base64'));
    },
    dimensions: () => evaluate('({width: innerWidth, height: innerHeight, content: document.body.scrollHeight})'),
    close: async () => { await evaluate('window.close()').catch(() => {}); socket.close(); },
  };
}
(async () => {
  assert.ok(existsSync(path.join(extension, 'manifest.json')), 'Build browser-extension/app first, or set NEXUS_EXTENSION_PATH');
  assert.ok(existsSync(executable), 'Install Playwright Chromium, or set PLAYWRIGHT_CHROMIUM_EXECUTABLE');
  assert.ok(existsSync(path.join(portable, 'config/pairing_token.txt')), 'Start a portable backend first, or set NEXUS_PORTABLE_DIR');
  await fs.mkdir(out, {recursive: true});
  const downloads = path.join(out, 'downloads'); await fs.mkdir(downloads);
  token = (await fs.readFile(path.join(portable, 'config/pairing_token.txt'), 'utf8')).trim();
  const baseline = new Set((await api('tasks')).tasks.map(t => t.task_id));
  const names = ['first', 'second', 'complete', 'burst-a', 'burst-b'].map(suffix => `nexus-progress-${stamp}-${suffix}.zip`);
  const bytes = Buffer.alloc(32 * 1024 * 1024, 0x4e);
  server = createServer((req, res) => {
    const name = decodeURIComponent(new URL(req.url, 'http://localhost').pathname.slice(1));
    if (!names.includes(name)) {
      res.writeHead(200, {'Content-Type': 'text/html; charset=utf-8'});
      res.end(`<!doctype html><title>Nexus progress test</title><h1>Download progress fixture</h1>${names.map(n => `<p><a href="/${n}" download="${n}">${n}</a></p>`).join('')}`); return;
    }
    const size = name === names[2] ? 1024 * 1024 : bytes.length;
    let start = 0, end = size - 1;
    const range = /bytes=(\d+)-(\d*)/.exec(req.headers.range || '');
    if (range) { start = +range[1]; if (range[2]) end = Math.min(end, +range[2]); }
    res.writeHead(range ? 206 : 200, {'Content-Type': 'application/zip',
      'Content-Disposition': `attachment; filename="${name}"`, 'Accept-Ranges': 'bytes',
      'Content-Length': end - start + 1, ...(range ? {'Content-Range': `bytes ${start}-${end}/${size}`} : {})});
    res.flushHeaders();
    if (req.method === 'HEAD') { res.end(); return; }
    let offset = start;
    const timer = setInterval(() => {
      if (offset > end) { clearInterval(timer); res.end(); return; }
      if (name === names[2] && !allowCompletion && offset > start) return;
      const next = Math.min(end + 1, offset + 32 * 1024); res.write(bytes.subarray(offset, next)); offset = next;
    }, 100);
    res.on('close', () => clearInterval(timer));
  });
  await new Promise(r => server.listen(0, '127.0.0.1', r));
  const base = 'http://127.0.0.1:' + server.address().port;
  const profile = path.join(out, 'profile');
  await fs.mkdir(path.join(profile, 'Default'), {recursive: true});
  await fs.writeFile(path.join(profile, 'Default/Preferences'), JSON.stringify({download: {default_directory: downloads, prompt_for_download: false}}));
  processHandle = spawn(executable,
    ['--no-first-run', '--no-default-browser-check', '--remote-debugging-port=0',
      `--user-data-dir=${profile}`, `--disable-extensions-except=${extension}`, `--load-extension=${extension}`, 'about:blank'],
    {stdio: 'ignore', windowsHide: true});
  const endpoint = await until(async () => {
    try { return 'http://127.0.0.1:' + (await fs.readFile(path.join(profile, 'DevToolsActivePort'), 'utf8')).split('\n')[0]; }
    catch { return null; }
  }, 'isolated browser startup');
  browser = await chromium.connectOverCDP(endpoint);
  const context = browser.contexts()[0];
  context.setDefaultTimeout(15000);
  context.on('console', message => {
    if (message.type() === 'warning' && message.text().startsWith('[Nexus]')) console.log(message.text());
  });
  const ours = w => w.url().startsWith('chrome-extension://kacpjjdipobohmfbpmnmlmfghoidnngh/');
  const worker = context.serviceWorkers().find(ours) || await context.waitForEvent('serviceworker', {predicate: ours});
  // Give this isolated profile the existing local daemon credentials, not user browser data.
  await worker.evaluate(async token => chrome.storage.local.set({pairToken: token, desktopServerUrl: 'http://127.0.0.1:14371', interceptDownloads: true}), token);
  const cdp = await browser.newBrowserCDPSession();
  await cdp.send('Browser.setDownloadBehavior', {behavior: 'allow', downloadPath: downloads});
  const page = await context.newPage(); await page.goto(base);
  const originalWindows = await worker.evaluate(async () => (await chrome.windows.getAll()).map(w => w.id));
  console.log('Fixture UI:', (await page.locator('body').innerText()).split('\n')[0]);
  // Wait for initialization/automatic connection before the click.
  await sleep(5500);
  await page.bringToFront();
  await worker.evaluate(async () => { const current = await chrome.windows.getLastFocused(); await chrome.windows.update(current.id, {focused: true}); });
  await page.getByRole('link', {name: names[0], exact: true}).click();
  const first = await until(async () => {
    const t = (await api('tasks')).tasks.find(t => t.title === names[0]);
    if (t) owned.add(t.task_id); return t;
  }, 'first accepted download');
  const popup = await toolbarPanel(endpoint, worker, first.task_id);
  await until(async () => (await popup.text()).includes(names[0]), 'single task rendered');
  assert.ok(!(await popup.text()).includes(names[1]));
  assert.ok(!(await popup.text()).includes('高级功能'));
  assert.deepEqual(await worker.evaluate(async () => (await chrome.windows.getAll()).map(w => w.id)), originalWindows, 'No independent browser windows');
  assert.equal(await worker.evaluate(() => chrome.action.getPopup({})), 'chrome-extension://kacpjjdipobohmfbpmnmlmfghoidnngh/popup.html');
  assert.ok(first.output_path.startsWith(downloads), 'Respect browser-selected save directory');
  await until(async () => (await api(`tasks/${first.task_id}`))?.received_bytes > 0, 'live progress');
  await until(async () => !/(?:^|\n)0%/.test(await popup.text()), 'progress reaches UI');
  await popup.screenshot(path.join(out, 'single-download-active.png'));
  const size = await popup.dimensions();
  assert.ok(size.height < 160, 'Progress popup must fit only one compact task card');
  assert.ok(size.content <= size.height + 1, 'No clipped content');
  console.log('Compact popup dimensions:', size);
  await popup.click('暂停');
  await until(async () => (await api(`tasks/${first.task_id}`))?.status === 'paused', 'pause');
  await until(async () => (await popup.text()).includes('继续'), 'resume button');
  assert.ok(!(await popup.text()).includes('任务操作已发送'), 'No success toast');
  await sleep(3000); // Capture after the short command-confirmation toast fades.
  await popup.screenshot(path.join(out, 'single-download-paused.png'));
  await popup.click('继续');
  await until(async () => (await api(`tasks/${first.task_id}`))?.status === 'downloading', 'resume');
  console.log('PASS automatic single-task toolbar popup, no standalone windows, save destination, real pause/resume');

  await page.getByRole('link', {name: names[1], exact: true}).click();
  const second = await until(async () => {
    const t = (await api('tasks')).tasks.find(t => t.title === names[1]);
    if (t) owned.add(t.task_id); return t;
  }, 'second accepted download');
  const secondPopup = await toolbarPanel(endpoint, worker, true);
  await until(async () => (await secondPopup.text()).includes(names[1]), 'newest task rendered');
  assert.ok(!(await secondPopup.text()).includes(names[0]));
  assert.deepEqual(await worker.evaluate(async () => (await chrome.windows.getAll()).map(w => w.id)), originalWindows);
  await secondPopup.click('取消');
  await until(async () => !(await api('tasks')).tasks.some(t => t.task_id === second.task_id), 'delete succeeds');
  owned.delete(second.task_id);
  await until(async () => !(await worker.evaluate(() => chrome.runtime.getContexts({contextTypes: ['POPUP']}))).length, 'deleted-task popup fully closes');
  // Open the current toolbar action URL, just as a manual icon click would.
  await worker.evaluate(() => chrome.action.openPopup());
  const full = await toolbarPanel(endpoint, worker);
  await until(async () => (await full.text()).includes(names[0]), 'manual full list');
  assert.ok(!(await full.text()).includes(names[1]), 'Deleted task removed from manual list too');
  assert.ok((await full.text()).includes('高级功能'));
  const before = await api(`tasks/${first.task_id}`);
  await full.close();
  await until(async () => (await api(`tasks/${first.task_id}`))?.received_bytes > before.received_bytes, 'download continues after close');
  console.log('PASS newest task only, delete fully closes popup, manual full list, closing does not stop transfer');

  // A deleted task must not cause its old window URL to fall back to history.
  const missing = await context.newPage();
  await missing.goto(`chrome-extension://kacpjjdipobohmfbpmnmlmfghoidnngh/popup.html?view=progress&task=${encodeURIComponent(second.task_id)}`);
  await missing.getByText('此任务已删除或不再可用', {exact: true}).waitFor();
  assert.equal(await missing.getByText(names[0], {exact: true}).count(), 0);
  await missing.close();

  await page.getByRole('link', {name: names[2], exact: true}).click();
  const completing = await until(async () => {
    const task = (await api('tasks')).tasks.find(t => t.title === names[2]);
    if (task) owned.add(task.task_id); return task;
  }, 'completion fixture accepted');
  const finishingPanel = await toolbarPanel(endpoint, worker, completing.task_id);
  await until(async () => (await finishingPanel.text()).includes(names[2]), 'completion progress visible');
  allowCompletion = true;
  await until(async () => (await api(`tasks/${completing.task_id}`)).status === 'completed', 'real download completes');
  await until(async () => !(await worker.evaluate(() => chrome.runtime.getContexts({contextTypes: ['POPUP']}))).length, 'automatic popup retracts after completion');
  await worker.evaluate(() => chrome.action.openPopup());
  const manualAfterCompletion = await toolbarPanel(endpoint, worker);
  await until(async () => (await manualAfterCompletion.text()).includes(names[2]), 'completed task in manual list');
  await sleep(1200);
  assert.ok((await manualAfterCompletion.text()).includes('高级功能'), 'Manual history does not auto-close');
  await manualAfterCompletion.close();
  console.log('PASS completed download automatically retracts; manual list stays open');

  // Exercise back-to-back accepted downloads before the first popup has mounted.
  await worker.evaluate(async urls => Promise.all(urls.map(url => chrome.downloads.download({url}))), names.slice(3).map(name => `${base}/${name}`));
  const burst = await until(async () => {
    const tasks = (await api('tasks')).tasks.filter(t => names.slice(3).includes(t.title));
    tasks.forEach(t => owned.add(t.task_id)); return tasks.length === 2 ? tasks : null;
  }, 'concurrent downloads accepted');
  const lastId = await until(async () => {
    const seen = await worker.evaluate(async () => (await chrome.storage.session.get('progressPopupSeenTasks')).progressPopupSeenTasks || []);
    return burst.every(t => seen.includes(t.task_id)) ? seen.at(-1) : null;
  }, 'both concurrent popup requests handled');
  const burstViews = await worker.evaluate(() => chrome.runtime.getContexts({contextTypes: ['POPUP']}));
  assert.equal(burstViews.length, 1);
  const originalId = new URL(burstViews[0].documentUrl).searchParams.get('task');
  const burstPanel = await toolbarPanel(endpoint, worker, originalId);
  const newest = burst.find(t => t.task_id === lastId), older = burst.find(t => t.task_id !== lastId);
  await until(async () => (await burstPanel.text()).includes(newest.title), 'concurrent popup shows last accepted task');
  assert.ok(!(await burstPanel.text()).includes(older.title));
  await burstPanel.close();
  console.log('PASS concurrent downloads show only latest task in one toolbar popup');
  const finalIds = new Set((await api('tasks')).tasks.map(t => t.task_id));
  for (const id of baseline) assert.ok(finalIds.has(id));
  console.log('PASS deleted task does not expose other tasks; existing tasks preserved');
  console.log('Screenshot:', path.join(out, 'single-download-active.png'));
})().catch(error => { console.error(error.message); process.exitCode = 1; }).finally(async () => {
  // Recover any fixture accepted just before a timeout, without touching other tasks.
  if (token) {
    try { for (const t of (await api('tasks')).tasks) if (t.title.startsWith(`nexus-progress-${stamp}-`)) owned.add(t.task_id); } catch {}
    for (const id of owned) try { await api(`tasks/${id}/delete`, {}); } catch {}
  }
  if (browser) await Promise.race([browser.close().catch(() => {}), sleep(3000)]);
  if (processHandle && processHandle.exitCode === null && processHandle.signalCode === null) {
    const exited = new Promise(resolve => processHandle.once('exit', resolve));
    processHandle.kill(); await exited;
  }
  server?.closeAllConnections(); server?.close();
  const profile = path.resolve(out, 'profile');
  const boundary = path.resolve(root, 'output/playwright/progress-window') + path.sep;
  if (!profile.startsWith(boundary) || path.basename(profile) !== 'profile') throw Error('Unsafe test profile cleanup target');
  await fs.rm(profile, {recursive: true, force: true, maxRetries: 5, retryDelay: 200});
});
