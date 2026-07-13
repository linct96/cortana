import { readdir, readFile } from 'node:fs/promises';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../src/', import.meta.url));
const limit = 500;
const oversized = [];

async function scan(directory) {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) await scan(path);
    else if (entry.name.endsWith('.tsx')) {
      const lines = (await readFile(path, 'utf8')).split('\n').length;
      if (lines > limit) oversized.push(`${path}: ${lines} lines`);
    }
  }
}

await scan(root);
if (oversized.length) {
  console.error(`TSX files must not exceed ${limit} lines:\n${oversized.join('\n')}`);
  process.exitCode = 1;
}
