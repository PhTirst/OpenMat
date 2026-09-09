import { spawn } from 'node:child_process';
import { createReadStream, createWriteStream } from 'node:fs';
import { mkdir, readFile, realpath, stat } from 'node:fs/promises';
import { createServer } from 'node:http';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2);
let configPath = path.join(root, 'openmat-web.json');
let openBrowser = true;
for (let index = 0; index < args.length; index += 1) {
  if (args[index] === '--no-open') openBrowser = false;
  else if (args[index] === '--config' && args[index + 1]) configPath = path.resolve(args[++index]);
  else throw new Error('Usage: launch.mjs [--no-open] [--config settings.json]');
}

let kernel;
let http;
let stopping = false;
function stop(code = 0) {
  if (stopping) return;
  stopping = true;
  http?.close();
  http?.closeAllConnections();
  if (kernel && kernel.exitCode === null) kernel.kill();
  process.exitCode = code;
}
process.on('SIGINT', () => stop());
process.on('SIGTERM', () => stop());
process.on('exit', () => { if (kernel && kernel.exitCode === null) kernel.kill(); });

try {
  const config = JSON.parse((await readFile(configPath, 'utf8')).replace(/^\uFEFF/, ''));
  if (config.schemaVersion !== 1) throw new Error('openmat-web.json: schemaVersion must be 1.');
  for (const name of ['webPort', 'kernelPort']) {
    if (!Number.isInteger(config[name]) || config[name] < 1 || config[name] > 65535) {
      throw new Error(`openmat-web.json: ${name} must be an integer from 1 to 65535.`);
    }
  }
  if (config.webPort === config.kernelPort) throw new Error('Web and kernel ports must be different.');
  if (typeof config.workspaceRoot !== 'string' || !config.workspaceRoot.trim()) {
    throw new Error('openmat-web.json: workspaceRoot must be a nonempty path.');
  }
  const workspace = path.resolve(path.dirname(configPath), config.workspaceRoot);
  const www = await realpath(path.join(root, 'www'));
  const kernelUrl = `ws://127.0.0.1:${config.kernelPort}/kernel`;
  const webUrl = `http://127.0.0.1:${config.webPort}/`;
  const mime = {
    '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8',
    '.css': 'text/css; charset=utf-8', '.json': 'application/json; charset=utf-8',
    '.wasm': 'application/wasm', '.svg': 'image/svg+xml', '.png': 'image/png',
    '.ico': 'image/x-icon', '.woff': 'font/woff', '.woff2': 'font/woff2', '.ttf': 'font/ttf',
  };
  http = createServer(async (request, response) => {
    response.setHeader('X-Content-Type-Options', 'nosniff');
    response.setHeader('Cache-Control', 'no-cache');
    if (request.method !== 'GET' && request.method !== 'HEAD') {
      response.writeHead(405, { Allow: 'GET, HEAD' }); response.end(); return;
    }
    try {
      const pathname = decodeURIComponent(new URL(request.url, webUrl).pathname);
      if (pathname === '/openmat-runtime.js') {
        const script = `window.__OPENMAT_RUNTIME__ = ${JSON.stringify({ kernelWebSocketUrl: kernelUrl })};\n`;
        response.writeHead(200, { 'Content-Type': mime['.js'], 'Content-Length': Buffer.byteLength(script) });
        response.end(request.method === 'HEAD' ? undefined : script); return;
      }
      const candidate = path.resolve(www, `.${pathname === '/' ? '/index.html' : pathname}`);
      const inside = (file) => file.startsWith(`${www}${path.sep}`);
      if (!inside(candidate)) { response.writeHead(404); response.end(); return; }
      const file = await realpath(candidate);
      if (!inside(file)) { response.writeHead(404); response.end(); return; }
      const details = await stat(file);
      if (!details.isFile()) { response.writeHead(404); response.end(); return; }
      response.writeHead(200, { 'Content-Type': mime[path.extname(file)] ?? 'application/octet-stream', 'Content-Length': details.size });
      if (request.method === 'HEAD') response.end();
      else createReadStream(file).on('error', () => response.destroy()).pipe(response);
    } catch {
      response.writeHead(404); response.end();
    }
  });
  await new Promise((resolve, reject) => {
    http.once('error', reject);
    http.listen({ host: '127.0.0.1', port: config.webPort, exclusive: true }, resolve);
  });
  http.on('error', (error) => { console.error(error.message); stop(1); });
  await mkdir(workspace, { recursive: true });
  await mkdir(path.join(root, 'logs'), { recursive: true });
  kernel = spawn(path.join(root, 'bin', 'openmat-server.exe'), [
    '--listen', `127.0.0.1:${config.kernelPort}`, '--workspace-root', workspace,
    '--openblas-dll', path.join(root, 'bin', 'libopenblas.dll'),
  ], { cwd: workspace, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
  kernel.stdout.pipe(createWriteStream(path.join(root, 'logs', 'kernel.stdout.log')));
  kernel.stderr.pipe(createWriteStream(path.join(root, 'logs', 'kernel.stderr.log')));
  kernel.stderr.on('data', (data) => process.stderr.write(data));
  await new Promise((resolve, reject) => {
    let output = '';
    const timeout = setTimeout(() => reject(new Error('Kernel startup timed out; inspect logs.')), 30000);
    const cleanup = () => { clearTimeout(timeout); kernel.stdout.off('data', read); kernel.off('error', fail); kernel.off('exit', exited); };
    const fail = (error) => { cleanup(); reject(error); };
    const exited = (code) => fail(new Error(`Kernel exited during startup (${code}); inspect logs.`));
    const read = (data) => {
      output = (output + data.toString()).slice(-65536);
      if (output.split(/\r?\n/).includes(kernelUrl)) { cleanup(); resolve(); }
    };
    kernel.stdout.on('data', read); kernel.once('error', fail); kernel.once('exit', exited);
  });
  kernel.on('exit', (code) => { if (!stopping) { console.error(`Kernel stopped (${code}).`); stop(1); } });
  console.log(`OpenMat Web ready: ${webUrl}\nWorkspace: ${workspace}\nPress Ctrl+C to stop both services.`);
  if (openBrowser) {
    const browser = spawn('explorer.exe', [webUrl], { windowsHide: true, stdio: 'ignore' });
    browser.on('error', () => console.log(`Open ${webUrl} in your browser.`));
    browser.unref();
  }
} catch (error) {
  console.error(`OpenMat Web: ${error.message}`);
  stop(1);
}
