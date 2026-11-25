import { spawn } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { resolve, sep } from 'node:path';
import { createInterface } from 'node:readline';
import { performance } from 'node:perf_hooks';

const [cli, wave, webRoot, port = '0'] = process.argv.slice(2);
if (!cli || !wave || !webRoot) {
  throw new Error('usage: node server.mjs CLI FILE.wave WEB_ROOT [PORT]');
}

let child;
let stopped;
const pending = [];

function query(body) {
  if (stopped) throw stopped;
  if (!child) {
    child = spawn(resolve(cli), ['wave-serve', resolve(wave)], {
      stdio: ['pipe', 'pipe', 'inherit'],
      windowsHide: true,
    });
    const fail = (error) => {
      stopped ??= error;
      for (const request of pending.splice(0)) request.reject(stopped);
    };
    child.on('error', fail);
    child.stdin.on('error', fail);
    child.on('exit', () => fail(new Error('Wave reader exited')));
    createInterface({ input: child.stdout }).on('line', (line) => {
      const request = pending.shift();
      if (!request) return;
      const first = line.indexOf('\t');
      const second = line.indexOf('\t', first + 1);
      request.resolve({
        queryMs: Number(line.slice(0, first)),
        encodeMs: Number(line.slice(first + 1, second)),
        body: line.slice(second + 1),
      });
    });
  }
  return new Promise((resolve, reject) => {
    pending.push({ resolve, reject });
    child.stdin.write(body + '\n');
  });
}

const root = resolve(webRoot);
const server = createServer(async (req, res) => {
  try {
    const origin = 'http://127.0.0.1:' + server.address().port;
    if (req.headers.host !== new URL(origin).host) {
      res.writeHead(403).end();
      return;
    }
    if (req.method === 'POST' && req.url === '/api/wave' &&
        req.headers.origin === origin && req.headers['content-type'] === 'application/json') {
      const chunks = [];
      let bytes = 0;
      for await (const chunk of req) {
        bytes += chunk.length;
        if (bytes > 4096) {
          res.writeHead(413).end();
          return;
        }
        chunks.push(chunk);
      }
      // Canonicalize to one line for the child's request protocol.
      const request = JSON.stringify(JSON.parse(Buffer.concat(chunks)));
      const start = performance.now();
      const { body, queryMs, encodeMs } = await query(request);
      const pipeMs = performance.now() - start;
      res.writeHead(200, {
        'Content-Type': 'application/json',
        'Content-Length': Buffer.byteLength(body),
        'Cache-Control': 'no-store',
        'Server-Timing': `query;dur=${queryMs}, encode;dur=${encodeMs}, pipe;dur=${pipeMs}`,
      }).end(body);
      return;
    }
    if (req.method !== 'GET') {
      res.writeHead(405).end();
      return;
    }
    const url = new URL(req.url, origin);
    const file = resolve(root, '.' + (url.pathname === '/' ? '/index.html' : url.pathname));
    if (!file.startsWith(root + sep)) {
      res.writeHead(404).end();
      return;
    }
    const body = await readFile(file);
    res.writeHead(200, {
      'Content-Type': file.endsWith('.js') ? 'text/javascript' :
        file.endsWith('.css') ? 'text/css' : 'text/html; charset=utf-8',
      'Cache-Control': 'no-store',
    }).end(body);
  } catch (error) {
    res.writeHead(error.code === 'ENOENT' ? 404 : 500, { 'Content-Type': 'application/json' })
      .end(JSON.stringify({ error: 'Local waveform request failed' }));
  }
});

function stop() {
  child?.stdin.end();
  server.close();
}
process.on('SIGINT', stop);
process.on('SIGTERM', stop);
// The benchmark owns the server via this pipe. Standalone use ends via Ctrl+C.
if (!process.stdin.isTTY) process.stdin.resume().on('end', stop);
server.listen(Number(port), '127.0.0.1', () => console.log(server.address().port));
