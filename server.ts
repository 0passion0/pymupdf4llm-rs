import express from 'express';
import fs from 'node:fs';
import path from 'node:path';
import zlib from 'node:zlib';
import { createServer as createViteServer } from 'vite';

const PORT = Number(process.env.PORT ?? 3000);
const ROOT = process.cwd();

function collectFiles(dir: string, out: Array<{ rel: string; data: Buffer }> = []) {
  if (!fs.existsSync(dir)) return out;
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    if (['target', '.git', 'node_modules', 'dist', 'web', '.aistudio'].includes(entry.name)) continue;
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) collectFiles(full, out);
    else if (entry.isFile()) {
      out.push({ rel: path.relative(ROOT, full).replace(/\\/g, '/'), data: fs.readFileSync(full) });
    }
  }
  return out;
}

function buildCrateTarGz(): Buffer {
  const targets = ['Cargo.toml', 'LICENSE', 'README.md', 'README.zh-CN.md', 'src', 'tests'];
  const files: Array<{ rel: string; data: Buffer }> = [];
  for (const t of targets) {
    const full = path.join(ROOT, t);
    if (!fs.existsSync(full)) continue;
    if (fs.statSync(full).isDirectory()) collectFiles(full, files);
    else files.push({ rel: t, data: fs.readFileSync(full) });
  }
  const chunks: Buffer[] = [];
  for (const f of files) {
    const header = Buffer.alloc(512, 0);
    header.write(`pymupdf4llm-rs/${f.rel}`.slice(0, 100), 0, 'utf8');
    header.write('0000644\0', 100, 'ascii');
    header.write('0000000\0', 108, 'ascii');
    header.write('0000000\0', 116, 'ascii');
    header.write(`${f.data.length.toString(8).padStart(11, '0')}\0`, 124, 'ascii');
    header.write(`${Math.floor(Date.now() / 1000).toString(8).padStart(11, '0')}\0`, 136, 'ascii');
    header.fill(0x20, 148, 156);
    header.write('0', 156, 'ascii');
    header.write('ustar\0', 257, 'ascii');
    header.write('00', 263, 'ascii');
    let sum = 0;
    for (let i = 0; i < 512; i++) sum += header[i];
    header.write(`${sum.toString(8).padStart(6, '0')}\0 `, 148, 'ascii');
    chunks.push(header, f.data);
    const pad = (512 - (f.data.length % 512)) % 512;
    if (pad > 0) chunks.push(Buffer.alloc(pad, 0));
  }
  chunks.push(Buffer.alloc(1024, 0));
  return zlib.gzipSync(Buffer.concat(chunks));
}

async function start() {
  const app = express();
  app.get('/api/health', (_req, res) => res.json({ status: 'ok', crate: 'pymupdf4llm-rs', version: '0.4.0' }));
  app.get('/api/crate/download', (_req, res) => {
    res.setHeader('Content-Type', 'application/gzip');
    res.setHeader('Content-Disposition', 'attachment; filename="pymupdf4llm-rs-0.4.0.tar.gz"');
    res.send(buildCrateTarGz());
  });

  if (process.env.NODE_ENV !== 'production') {
    const vite = await createViteServer({ server: { middlewareMode: true }, appType: 'spa' });
    app.use(vite.middlewares);
  } else {
    const dist = path.resolve(ROOT, 'dist');
    app.use(express.static(dist));
    app.get('*', (_req, res) => res.sendFile(path.join(dist, 'index.html')));
  }
  app.listen(PORT, '0.0.0.0', () => console.log(`pymupdf4llm-rs server on http://0.0.0.0:${PORT}`));
}

start();
