import express from 'express';
import fs from 'node:fs';
import path from 'node:path';
import zlib from 'node:zlib';
import { createServer as createViteServer } from 'vite';
import { runPyMuPdf4LlmPipeline } from './src/engine/pymupdf4llmPipeline';
import { DEFAULT_OPTIONS, type ToMarkdownOptions } from './src/engine/types';

const PORT = Number(process.env.PORT ?? 3000);
const CRATE_ROOT = path.resolve(process.cwd(), 'rust', 'pymupdf4llm-rs');

interface CrateFile {
  path: string;
  sizeBytes: number;
  lines: number;
  content: string;
}

function collectCrateFiles(dir: string, baseDir = dir): CrateFile[] {
  if (!fs.existsSync(dir)) return [];
  const entries = fs.readdirSync(dir, { withFileTypes: true });
  const files: CrateFile[] = [];
  for (const entry of entries.sort((a, b) => a.name.localeCompare(b.name))) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      files.push(...collectCrateFiles(full, baseDir));
    } else if (entry.isFile()) {
      const rel = path.relative(baseDir, full).replace(/\\/g, '/');
      const content = fs.readFileSync(full, 'utf8');
      files.push({
        path: rel,
        sizeBytes: Buffer.byteLength(content, 'utf8'),
        lines: content.split('\n').length,
        content,
      });
    }
  }
  return files;
}

function createTarHeader(name: string, size: number): Buffer {
  const header = Buffer.alloc(512, 0);
  const fullName = `pymupdf4llm-rs/${name}`;
  header.write(fullName.slice(0, 100), 0, 'utf8');
  header.write('0000644\0', 100, 'ascii');
  header.write('0000000\0', 108, 'ascii');
  header.write('0000000\0', 116, 'ascii');
  header.write(`${size.toString(8).padStart(11, '0')}\0`, 124, 'ascii');
  header.write(`${Math.floor(Date.now() / 1000).toString(8).padStart(11, '0')}\0`, 136, 'ascii');
  header.fill(0x20, 148, 156); // checksum placeholder
  header.write('0', 156, 'ascii'); // regular file
  header.write('ustar\0', 257, 'ascii');
  header.write('00', 263, 'ascii');

  let sum = 0;
  for (let i = 0; i < 512; i++) sum += header[i];
  header.write(`${sum.toString(8).padStart(6, '0')}\0 `, 148, 'ascii');
  return header;
}

function buildCrateTarGz(files: CrateFile[]): Buffer {
  const chunks: Buffer[] = [];
  for (const f of files) {
    const data = Buffer.from(f.content, 'utf8');
    chunks.push(createTarHeader(f.path, data.length));
    chunks.push(data);
    const pad = (512 - (data.length % 512)) % 512;
    if (pad > 0) chunks.push(Buffer.alloc(pad, 0));
  }
  chunks.push(Buffer.alloc(1024, 0)); // EOF blocks
  return zlib.gzipSync(Buffer.concat(chunks));
}

async function startServer() {
  const app = express();
  app.use(express.json({ limit: '25mb' }));

  app.get('/api/health', (_req, res) => {
    res.json({ status: 'ok', crate: 'pymupdf4llm-rs' });
  });

  app.get('/api/crate/files', (_req, res) => {
    const files = collectCrateFiles(CRATE_ROOT);
    res.json({
      crateName: 'pymupdf4llm-rs',
      version: '0.4.0',
      fileCount: files.length,
      totalBytes: files.reduce((acc, f) => acc + f.sizeBytes, 0),
      totalLines: files.reduce((acc, f) => acc + f.lines, 0),
      files,
    });
  });

  app.get('/api/crate/download', (_req, res) => {
    const files = collectCrateFiles(CRATE_ROOT);
    const archive = buildCrateTarGz(files);
    res.setHeader('Content-Type', 'application/gzip');
    res.setHeader('Content-Disposition', 'attachment; filename="pymupdf4llm-rs-0.4.0.tar.gz"');
    res.send(archive);
  });

  app.post('/api/parse', async (req, res) => {
    try {
      const { fileName, base64, options } = req.body as {
        fileName?: string;
        base64?: string;
        options?: Partial<ToMarkdownOptions>;
      };
      if (!base64) {
        res.status(400).json({ error: 'Missing PDF base64 payload' });
        return;
      }
      const bytes = new Uint8Array(Buffer.from(base64, 'base64'));
      const mergedOptions: ToMarkdownOptions = {
        ...DEFAULT_OPTIONS,
        ...options,
        margins: { ...DEFAULT_OPTIONS.margins, ...(options?.margins ?? {}) },
        hygiene: { ...DEFAULT_OPTIONS.hygiene, ...(options?.hygiene ?? {}) },
      };
      const result = await runPyMuPdf4LlmPipeline(bytes, fileName ?? 'uploaded.pdf', mergedOptions);
      res.json(result);
    } catch (err) {
      res.status(422).json({ error: err instanceof Error ? err.message : 'PDF parsing failed' });
    }
  });

  if (process.env.NODE_ENV !== 'production') {
    const vite = await createViteServer({
      server: { middlewareMode: true },
      appType: 'spa',
    });
    app.use(vite.middlewares);
  } else {
    const distDir = path.resolve(process.cwd(), 'dist');
    app.use(express.static(distDir));
    app.get('*', (_req, res) => {
      res.sendFile(path.join(distDir, 'index.html'));
    });
  }

  app.listen(PORT, '0.0.0.0', () => {
    console.log(`pymupdf4llm-rs workbench listening on http://0.0.0.0:${PORT}`);
  });
}

startServer();
