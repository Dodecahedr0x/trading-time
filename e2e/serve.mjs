// Static server for the bundled web app (target/web/public); builds it first when missing.
import { createServer } from 'node:http';
import { existsSync, readFileSync, statSync } from 'node:fs';
import { execSync } from 'node:child_process';
import { extname, join, normalize } from 'node:path';

const root = new URL('../', import.meta.url).pathname;
const dir = join(root, 'target/web/public');
const port = Number(process.env.TT_WEB_PORT ?? 4173);

if (!existsSync(join(dir, 'index.html')) || process.env.TT_E2E_REBUILD) {
  execSync('dx bundle --web --release -p tradingtime-web --out-dir target/web', { cwd: root, stdio: 'inherit' });
}

const types = { '.html': 'text/html', '.js': 'text/javascript', '.wasm': 'application/wasm', '.css': 'text/css', '.svg': 'image/svg+xml' };

createServer((req, res) => {
  const path = normalize(decodeURIComponent(new URL(req.url, 'http://x').pathname)).replace(/^(\.\.[/\\])+/, '');
  let file = join(dir, path);
  if (!file.startsWith(dir) || !existsSync(file) || statSync(file).isDirectory()) file = join(dir, 'index.html');
  res.writeHead(200, { 'content-type': types[extname(file)] ?? 'application/octet-stream' });
  res.end(readFileSync(file));
}).listen(port, '127.0.0.1', () => console.log(`serving ${dir} on http://127.0.0.1:${port}`));
