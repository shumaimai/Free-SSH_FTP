// 画面の文言の検査。build の前に走る。
//  1. 日本語を含む文字列は t('…') / N_('…') の中だけに書く（直書きは失敗）。
//  2. t / N_ のキーはすべて英語の辞書にある。辞書に余りがない。{名前} の組が原文と訳で一致する。訳に日本語が残っていない。
// 直書きを許す行は、同じ行の末尾に `// i18n-ignore: 理由` と書く。
import { readdirSync, readFileSync } from 'node:fs';
import { join, relative } from 'node:path';
import { parseAst } from 'vite';

const root = new URL('..', import.meta.url).pathname;
const src = join(root, 'src');
const JA = /[぀-ヿ㐀-鿿＀-￯]/;
const SKIP = new Set(['mock.ts']); // ブラウザ確認用の模擬データ。表に出る文言ではない。

function walk(dir) {
  const out = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) {
      if (entry.name === 'i18n') continue;
      out.push(...walk(path));
    } else if (/\.(ts|tsx)$/.test(entry.name) && !SKIP.has(entry.name) && !entry.name.endsWith('.d.ts')) out.push(path);
  }
  return out;
}

const errors = [];
const used = new Set();

function lineOf(text, offset) {
  return text.slice(0, offset).split('\n').length;
}

for (const file of walk(src)) {
  const text = readFileSync(file, 'utf8');
  const lines = text.split('\n');
  const ast = parseAst(text, { lang: file.endsWith('x') ? 'tsx' : 'ts' });
  const consumed = new Set();
  const report = (node, what) => {
    const line = lineOf(text, node.start);
    if (lines[line - 1].includes('i18n-ignore')) return;
    errors.push(`${relative(root, file)}:${line}: ${what}`);
  };
  (function visit(node) {
    if (!node || typeof node !== 'object') return;
    if (Array.isArray(node)) return node.forEach(visit);
    if (node.type === 'CallExpression' && node.callee?.type === 'Identifier' && (node.callee.name === 't' || node.callee.name === 'N_')) {
      const arg = node.arguments[0];
      let key = null;
      if (arg?.type === 'Literal' && typeof arg.value === 'string') key = arg.value;
      else if (arg?.type === 'TemplateLiteral' && arg.expressions.length === 0) key = arg.quasis[0].value.cooked;
      if (key !== null) {
        used.add(key);
        consumed.add(arg.start);
        if (arg.type === 'TemplateLiteral') consumed.add(arg.quasis[0].start);
      } else if (node.callee.name === 'N_' || (arg && arg.type !== 'Identifier' && arg.type !== 'MemberExpression')) {
        report(node, 't() / N_() の第1引数は文字列リテラルにしてください');
      }
    }
    if (!consumed.has(node.start)) {
      if (node.type === 'Literal' && typeof node.value === 'string' && JA.test(node.value)) report(node, `t() を通していない日本語: ${JSON.stringify(node.value.slice(0, 30))}`);
      if (node.type === 'TemplateElement' && JA.test(node.value.cooked ?? '')) report(node, `t() を通していない日本語(テンプレート): ${JSON.stringify((node.value.cooked ?? '').slice(0, 30))}`);
      if (node.type === 'JSXText' && JA.test(node.value)) report(node, `t() を通していない日本語(JSX): ${JSON.stringify(node.value.trim().slice(0, 30))}`);
    }
    for (const [name, value] of Object.entries(node)) {
      if (name === 'start' || name === 'end' || name === 'type') continue;
      visit(value);
    }
  })(ast);
}

// 辞書(TypeScriptの `export default {…} as …` を直接読む)
const dictDir = join(src, 'i18n', 'en');
const dict = {};
for (const name of readdirSync(dictDir).filter((n) => n.endsWith('.ts'))) {
  const text = readFileSync(join(dictDir, name), 'utf8');
  const ast = parseAst(text, { lang: 'ts' });
  const exported = ast.body.find((n) => n.type === 'ExportDefaultDeclaration');
  let obj = exported?.declaration;
  while (obj && obj.type !== 'ObjectExpression') obj = obj.expression;
  if (!obj) {
    errors.push(`i18n/en/${name}: export default { … } が見つかりません`);
    continue;
  }
  for (const prop of obj.properties) {
    const k = prop.key.type === 'Literal' ? prop.key.value : prop.key.name;
    const v = prop.value.type === 'TemplateLiteral' ? prop.value.quasis.map((q) => q.value.cooked).join('') : prop.value.value;
    if (typeof v !== 'string') {
      errors.push(`i18n/en/${name}: ${JSON.stringify(k)} の訳が文字列リテラルではありません`);
      continue;
    }
    if (k in dict) errors.push(`i18n/en/${name}: キーが重複しています: ${JSON.stringify(k)}`);
    dict[k] = v;
    if (JA.test(v)) errors.push(`i18n/en/${name}: 訳に日本語が残っています: ${JSON.stringify(k)}`);
    const names = (s) => [...s.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort().join(',');
    if (names(k) !== names(v)) errors.push(`i18n/en/${name}: {名前} が原文と訳で違います: ${JSON.stringify(k)}`);
  }
}
for (const key of used) if (!(key in dict)) errors.push(`辞書にありません: ${JSON.stringify(key)}`);
for (const key of Object.keys(dict)) if (!used.has(key)) errors.push(`使われていない訳です: ${JSON.stringify(key)}`);

if (errors.length) {
  console.error(`check-i18n: ${errors.length} 件\n` + errors.join('\n'));
  process.exit(1);
}
console.log(`check-i18n: ok (${used.size} 文言)`);
