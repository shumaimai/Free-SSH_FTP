// アイコンの検査。build の前に走る。
// @fluentui/react-icons は src/icons.tsx からだけ読み込む(1 つの機能に 1 つのアイコンを、1 か所で決めるため)。
import { readdirSync, readFileSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url)); // Windows でも正しいパスになる
const src = join(root, 'src');
const ALLOWED = join(src, 'icons.tsx');

function walk(dir) {
  const out = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) out.push(...walk(path));
    else if (/\.(ts|tsx)$/.test(entry.name)) out.push(path);
  }
  return out;
}

const errors = [];
for (const file of walk(src)) {
  if (file === ALLOWED) continue;
  readFileSync(file, 'utf8')
    .split('\n')
    .forEach((line, i) => {
      if (line.includes('@fluentui/react-icons')) {
        errors.push(`${relative(root, file).replaceAll('\\', '/')}:${i + 1}: アイコンは src/icons.tsx の Icon から使ってください`);
      }
    });
}
if (errors.length > 0) {
  console.error(errors.join('\n'));
  process.exit(1);
}
console.log('check-icons: ok');
