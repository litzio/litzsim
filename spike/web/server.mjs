import { createServer } from 'node:http';
import { readdirSync, realpathSync } from 'node:fs';
import { createRequire } from 'node:module';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { activate } from './plugin/host.mjs';

const [executable, fixtureDirectory, mode] = process.argv.slice(2);
if (!executable || !fixtureDirectory || !['cold', 'persistent'].includes(mode)) {
  throw new Error('Usage: node server.mjs LITZSIM_CLI FIXTURE_DIRECTORY cold|persistent');
}
const root = realpathSync(fixtureDirectory);
const base = path.dirname(fileURLToPath(import.meta.url));
const require = createRequire(process.env.SPIKE_BROWSER_PACKAGE ?? import.meta.url);
const { build } = require('esbuild');
const script = await build({
  entryPoints: [path.join(base, 'browser.jsx')], bundle: true, write: false,
  alias: { react: require.resolve('react'), 'react-dom/client': require.resolve('react-dom/client') },
  platform: 'browser', define: { 'process.env.NODE_ENV': '"production"' },
});
const app = activate({ settings: { executable: realpathSync(executable), persistentCore: mode === 'persistent' } });
const page = '<!doctype html><html lang="en"><meta charset="utf-8"><title>LitzSim</title>' +
  '<body style="font:16px system-ui;margin:0"><main id="root"></main><script defer src="/app.js"></script></body></html>';

const server = createServer(async (request, response) => {
  const send = (status, type, body) => {
    response.writeHead(status, { 'Content-Type': type, 'Cache-Control': 'no-store' });
    response.end(body);
  };
  try {
    if (request.method === 'GET' && request.url === '/') return send(200, 'text/html; charset=utf-8', page);
    if (request.method === 'GET' && request.url === '/app.js') return send(200, 'text/javascript', script.outputFiles[0].contents);
    if (request.method === 'GET' && request.url === '/files') {
      const files = readdirSync(root, { withFileTypes: true }).filter(file =>
        file.isFile() && file.name.toLowerCase().endsWith('.litzsim')).map(file => file.name);
      return send(200, 'application/json', JSON.stringify(files));
    }
    const route = /^\/operation\/([a-z]+)$/.exec(request.url);
    if (request.method !== 'POST' || !route || !Object.hasOwn(app.operations, route[1]))
      return send(404, 'text/plain', 'Not found');
    if (request.headers.origin !== 'http://' + server.address().address + ':' + server.address().port ||
        request.headers['content-type'] !== 'application/json')
      return send(403, 'text/plain', 'Only the local editor can call operations.');
    let body = '';
    request.setEncoding('utf8');
    for await (const chunk of request) {
      body += chunk;
      if (body.length > 1024 * 1024) throw new Error('Request is too large.');
    }
    const input = JSON.parse(body);
    if (Object.hasOwn(input, 'file')) {
      if (input.file !== path.basename(input.file) || path.extname(input.file).toLowerCase() !== '.litzsim')
        throw new Error('Select a fixture circuit.');
      input.file = realpathSync(path.join(root, input.file));
      if (path.dirname(input.file) !== root) throw new Error('File is outside the fixture directory.');
    }
    const result = await app.operations[route[1]](input);
    return send(200, 'application/json', JSON.stringify({ result }));
  } catch (e) {
    send(400, 'application/json', JSON.stringify({ error: e.message }));
  }
});
server.listen(0, '127.0.0.1', () => console.log('Standalone UI http://127.0.0.1:' + server.address().port));
async function shutdown() {
  server.close();
  await app.dispose();
}
process.once('SIGINT', shutdown);
process.once('SIGTERM', shutdown);
