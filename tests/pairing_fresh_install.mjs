// Windows end-to-end pairing regression: real daemon + real stdio bridge +
// shipped extension client. Only Chrome's storage/native API boundary is adapted.
// Run with: node tests/pairing_fresh_install.mjs <release-gd3-daemon.exe>
// Uses empty temporary installs. Restores the two native-host registry values.
import assert from 'node:assert/strict';
import {spawn, execFileSync} from 'node:child_process';
import {once} from 'node:events';
import {mkdtemp, copyFile, mkdir, readFile, writeFile, access, unlink} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {resolve, join} from 'node:path';
import {createServer} from 'node:net';
import {createRequire} from 'node:module';

const root = process.cwd();
const extension = await access(join(root, 'browser-extension/app/package.json')).then(
  () => join(root, 'browser-extension/app'),
  () => join(root, 'Ghost-Downloader-3-main/browser_extension/app'));
const {build} = createRequire(join(extension, 'package.json'))('esbuild');
const binary = resolve(process.argv[2]);
const temporary = await mkdtemp(join(tmpdir(), 'nexus-pairing-'));
const install = join(temporary, 'fresh install');
await mkdir(install);
const executable = join(install, 'gd3-daemon.exe');
await copyFile(binary, executable);
const listener = createServer();
await new Promise(r => listener.listen(0, '127.0.0.1', r));
const port = listener.address().port;
await new Promise(r => listener.close(r));
const base = `http://127.0.0.1:${port}`;
const environment = {...process.env, GD3_HTTP_ADDR: `127.0.0.1:${port}`,
  HTTP_PROXY: 'http://127.0.0.1:9', HTTPS_PROXY: 'http://127.0.0.1:9', ALL_PROXY: 'http://127.0.0.1:9'};
const registryPrefix = `HKCU:\\Software\\`;
const keys = ['Google\\Chrome', 'Microsoft\\Edge'].map(browser =>
  registryPrefix + browser + '\\NativeMessagingHosts\\com.gd3.nexusdownloader');
function registry(command, extra = {}) {
  return execFileSync('powershell.exe', ['-NoProfile', '-Command', command], {
    windowsHide: true, encoding: 'utf8', env: {...process.env, NEXUS_TEST_KEYS: JSON.stringify(keys), ...extra}}).trim();
}
const saved = registry(`$ErrorActionPreference='Stop'; @(($env:NEXUS_TEST_KEYS | ConvertFrom-Json) | ForEach-Object {
  if(Test-Path -LiteralPath $_){$k=Get-Item -LiteralPath $_; @{path=$_;exists=$true;value=$k.GetValue('')}}
  else{@{path=$_;exists=$false;value=$null}}
}) | ConvertTo-Json -Compress`);
await writeFile(join(temporary, 'registry-before.json'), saved);
let daemon;
async function stop() {
  if (daemon && daemon.exitCode === null && daemon.signalCode === null) {
    const exited = once(daemon, 'exit'); daemon.kill(); await exited;
  }
  daemon = undefined;
}
async function start() {
  daemon = spawn(executable, [], {cwd: install, env: environment, windowsHide: true, stdio: 'ignore'});
  for (let attempt = 0; attempt < 100; attempt++) {
    if (daemon.exitCode !== null) throw new Error('Fresh daemon exited early');
    try {
      const response = await fetch(`${base}/api/tasks`, {signal: AbortSignal.timeout(400)});
      if (response.status === 401) return;
    } catch {}
    await new Promise(r => setTimeout(r, 100));
  }
  throw new Error('Fresh daemon did not become ready');
}
async function native(message = {action: 'get_token'}) {
  const child = spawn(executable, ['--native-messaging'], {
    cwd: temporary, env: environment, windowsHide: true, stdio: ['pipe', 'pipe', 'ignore']});
  const body = Buffer.from(JSON.stringify(message));
  const header = Buffer.alloc(4); header.writeUInt32LE(body.length);
  const chunks = [];
  child.stdout.on('data', chunk => chunks.push(chunk));
  const timeout = setTimeout(() => child.kill(), 8000);
  child.stdin.end(Buffer.concat([header, body]));
  try { await once(child, 'exit'); } finally { clearTimeout(timeout); }
  const data = Buffer.concat(chunks);
  assert.ok(data.length >= 4, 'Bridge must reply even while daemon is offline');
  assert.equal(data.length, data.readUInt32LE(0) + 4, 'Native framing must contain only a response');
  return JSON.parse(data.subarray(4).toString());
}
const stored = {};
globalThis.self = {setTimeout, clearTimeout};
globalThis.chrome = {
  storage: {local: {get: async defaults => ({...defaults, ...stored}), set: async values => Object.assign(stored, values)}},
  runtime: {
    getManifest: () => ({version: 'test'}), getURL: path => `chrome-extension://test/${path}`,
    sendNativeMessage: (_host, message, callback) => {
      native(message).then(callback).catch(() => {
        chrome.runtime.lastError = {message: 'Test native bridge failed'};
        callback(); delete chrome.runtime.lastError;
      });
    },
  },
};
const compiled = await build({entryPoints: [join(extension, 'src/background/daemon-client.ts')],
  bundle: true, platform: 'node', format: 'cjs', write: false});
const module = {exports: {}};
new Function('module', 'exports', compiled.outputFiles[0].text)(module, module.exports);
const {createDaemonClient} = module.exports;
function client() { const result = createDaemonClient(); return result.setServerUrl(base).then(() => result); }
const tokenPath = join(install, 'config/pairing_token.txt');
try {
  const extensionFirst = await client();
  await assert.rejects(extensionFirst.connect(), /等待后台启动/);
  assert.equal(extensionFirst.hasToken(), false);
  assert.equal(await access(join(install, 'config')).then(() => true, () => false), false,
    'Offline bridge must not create config or token files');
  await start();
  const generated = (await readFile(tokenPath, 'utf8')).trim();
  assert.ok(generated.length > 32, 'Daemon must initialize token without React UI');
  await extensionFirst.connect();
  assert.equal(extensionFirst.isReady(), true);
  assert.ok(extensionFirst.buildSnapshot().token === generated, 'Bridge must relay daemon token');
  assert.equal((await extensionFirst.getTasks()).length, 0);
  console.log('PASS empty install: extension first, offline wait, backend starts, automatic pairing');

  const peers = await Promise.all([client(), client(), client()]);
  await Promise.all(peers.map(peer => peer.connect()));
  assert.ok(peers.every(peer => peer.buildSnapshot().token === generated), 'Concurrent clients must share token');
  const registration = JSON.parse(registry(`@(($env:NEXUS_TEST_KEYS | ConvertFrom-Json) | ForEach-Object {
    (Get-Item -LiteralPath $_).GetValue('')
  }) | ConvertTo-Json -Compress`));
  for (const manifestPath of registration) {
    const manifest = JSON.parse(await readFile(manifestPath, 'utf8'));
    assert.ok(manifest.path.toLowerCase() === executable.toLowerCase());
    assert.deepEqual(manifest.allowed_origins, ['chrome-extension://kacpjjdipobohmfbpmnmlmfghoidnngh/']);
  }
  console.log('PASS backend first, concurrent clients, Chrome and Edge registration');

  await stop();
  const offline = await native();
  assert.equal(offline.error, 'daemon_unavailable');
  assert.ok((await readFile(tokenPath, 'utf8')).trim() === generated, 'Offline bridge must not change token');
  await assert.rejects(extensionFirst.connect());
  await start(); await extensionFirst.connect();
  assert.equal(extensionFirst.isReady(), true);
  console.log('PASS daemon exit/restart, persistent token, no UI or manual pairing');

  const revoked = await fetch(`${base}/api/pairing/revoke`, {
    method: 'POST', headers: {Authorization: `Bearer ${generated}`}});
  assert.equal(revoked.status, 200);
  await assert.rejects(extensionFirst.connect());
  assert.equal(extensionFirst.hasToken(), false);
  await extensionFirst.connect();
  assert.equal(extensionFirst.isReady(), true);
  assert.ok(extensionFirst.buildSnapshot().token !== generated, 'Revoked token must change');
  const reloaded = createDaemonClient();
  await reloaded.loadPersistentState(); await reloaded.connect();
  assert.equal(reloaded.isReady(), true);
  console.log('PASS token revocation repair and extension worker reload');

  await stop(); await unlink(tokenPath);
  await start();
  await assert.rejects(reloaded.connect());
  await reloaded.connect();
  assert.equal(reloaded.isReady(), true);
  console.log('PASS regenerated installation token repairs stale extension state');
} finally {
  await stop();
  registry(`$ErrorActionPreference='Stop'; foreach($s in ($env:NEXUS_TEST_REGISTRY | ConvertFrom-Json)) {
    if($s.exists){$k=[Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($s.path.Substring(6),$true); try{if($null -eq $s.value){$k.DeleteValue('', $false)}else{$k.SetValue('',[string]$s.value)}}finally{$k.Dispose()}}
    elseif(Test-Path -LiteralPath $s.path){Remove-Item -LiteralPath $s.path}
  }`, {NEXUS_TEST_REGISTRY: saved});
  console.log('Native-host registration restored; isolated test data retained at:', temporary);
}
