// 組み込みのコマンド辞書から、入力中の行の補完候補を作る。
import type { Candidate, CompletionResult, ShellKind } from './types.ts';
import { dictFor, type DictNode } from './dict.ts';

const MAX_LINE_LENGTH = 4096;
const EMPTY: CompletionResult = { ghost: null, items: [] };

// コマンドを区切る文字。この後は「新しいコマンドの先頭」として扱う。
const SEPARATOR_CHARS = new Set(['|', ';', '&']);
const SEPARATOR_RE = /^[|;&]+$/;

// コマンドの前に置ける環境変数代入(FOO=1)。
const ENV_ASSIGN_RE = /^[A-Za-z_][A-Za-z0-9_]*=/;

// sudo の、次のトークンを値として取るオプション(`sudo -u user` など)。
// `-uroot` や `--user=root` のように値をくっ付けた形はトークンだけで完結するので含めない。
const SUDO_VALUE_OPTS = new Set([
  '-u', '--user',
  '-g', '--group',
  '-h', '--host',
  '-p', '--prompt',
  '-C',
  '-D', '--chdir',
  '-R', '--chroot',
  '-r', '--role',
  '-t', '--type',
  '-T', '--command-timeout',
  '--login-class',
]);

interface Tokenized {
  /** 空白を含まない語の列。区切り記号も1要素として入る。 */
  tokens: string[];
  /** カーソルが新しいトークンの先頭にある(行末が空白または区切り)。 */
  endedEmpty: boolean;
  /** 引用符が閉じていない。 */
  unclosedQuote: boolean;
}

/**
 * 行をトークンへ分割する。空白区切りで、引用符の中は1つの語として読む。
 * posix では `'` と `"` が引用符で `\` がエスケープ。cmd では `"` だけが引用符で
 * エスケープは無い(バックスラッシュはパスの区切り文字)。
 */
function tokenize(line: string, posix: boolean): Tokenized {
  const tokens: string[] = [];
  let cur = '';
  let quote: string | null = null;
  for (let i = 0; i < line.length; i++) {
    const ch = line[i];
    if (posix && quote !== "'" && ch === '\\') {
      // バックスラッシュは次の1文字をエスケープ(単一引用符の中では効かない)。
      cur += ch;
      if (i + 1 < line.length) cur += line[++i];
      continue;
    }
    if (quote !== null) {
      cur += ch;
      if (ch === quote) quote = null;
      continue;
    }
    if (posix && ch === "'") {
      quote = "'";
      cur += ch;
      continue;
    }
    if (ch === '"') {
      quote = '"';
      cur += ch;
      continue;
    }
    if (ch === ' ' || ch === '\t') {
      if (cur !== '') {
        tokens.push(cur);
        cur = '';
      }
      continue;
    }
    if (SEPARATOR_CHARS.has(ch)) {
      // `2>&1` のようなリダイレクト複製はコマンド区切りではない。
      if (ch === '&' && (cur.endsWith('>') || cur.endsWith('<'))) {
        cur += ch;
        continue;
      }
      if (cur !== '') {
        tokens.push(cur);
        cur = '';
      }
      // 同じ記号の連続(&&, ||, ;;)は1つの区切り。
      let sep = ch;
      while (i + 1 < line.length && line[i + 1] === ch) sep += line[++i];
      tokens.push(sep);
      continue;
    }
    cur += ch;
  }
  const endedEmpty = cur === '';
  if (!endedEmpty) tokens.push(cur);
  return { tokens, endedEmpty, unclosedQuote: quote !== null };
}

/**
 * 入力中の行(プロンプトを除いた、カーソルまでの文字列)に対する候補。
 * 最後のトークンを辞書の現在位置の子と前方一致で照合し、1トークンだけ補完する。
 * items は辞書の並び順、ghost は最後のトークンが空でないときの items[0].insert。
 */
export function complete(shell: ShellKind, line: string, limit = 50): CompletionResult {
  if (line.length > MAX_LINE_LENGTH) return EMPTY;
  if (/[^\x00-\x7f]/.test(line)) return EMPTY; // 非 ASCII(IME 確定など)は対象外

  const posix = shell === 'posix';
  const { tokens, endedEmpty, unclosedQuote } = tokenize(line, posix);
  // 引用符が閉じていない位置では中身を推測できないので候補を出さない。
  if (unclosedQuote) return EMPTY;

  let lastToken = '';
  let completed = tokens;
  if (!endedEmpty) {
    // 末尾は区切りではあり得ない(区切りで終わるなら cur が空で endedEmpty になる)。
    lastToken = tokens[tokens.length - 1];
    completed = tokens.slice(0, -1);
  }

  const dict = dictFor(shell);
  // 照合キー。cmd は大小文字を区別しないが、`-` で始まるオプション語は区別する
  // (ssh の -l と -L、curl の -o と -O のように大小文字で意味が違う)。
  // `/` オプションとコマンド名・サブコマンド名は従来どおり区別しない。
  const keyOf = (s: string) => (!posix && !s.startsWith('-') ? s.toLowerCase() : s);
  const findChild = (list: DictNode[] | undefined, tok: string) => list?.find((n) => keyOf(n.word) === keyOf(tok));

  // カーソルより前のトークンを左から読み、辞書上の現在位置を進める。
  let atStart = true; // 新しいコマンドの先頭にいる
  let node: DictNode | null = null; // いまのコマンド/サブコマンド(null = 未知)
  let subsOpen = true; // 位置引数が出る前ならサブコマンドも候補
  let lastIsValue = false; // カーソルが sudo オプションの値の位置にある
  const optsPath: DictNode[][] = []; // 通ってきたノードのオプション(外側→内側)。apt install -y のように親のオプションも後に置ける。
  for (let i = 0; i < completed.length; i++) {
    const tok = completed[i];
    if (SEPARATOR_RE.test(tok)) {
      atStart = true;
      node = null;
      subsOpen = true;
      optsPath.length = 0;
      continue;
    }
    if (atStart) {
      if (ENV_ASSIGN_RE.test(tok)) continue; // FOO=1 は読み飛ばす
      if (posix && tok === 'sudo') {
        // sudo とそのオプションを読み飛ばす。値を取るオプションは次のトークンも消費する。
        while (i + 1 < completed.length) {
          const t = completed[i + 1];
          if (!t.startsWith('-') || SEPARATOR_RE.test(t)) break;
          i++;
          if (SUDO_VALUE_OPTS.has(t)) {
            if (i + 1 < completed.length) i++;
            else lastIsValue = true; // 値は最後のトークン = いま打っている場所
          }
        }
        continue;
      }
      node = findChild(dict, tok) ?? null;
      atStart = false;
      subsOpen = true;
      optsPath.length = 0;
      if (node?.opts) optsPath.push(node.opts);
      continue;
    }
    if (node === null) continue; // 未知コマンドの引数位置
    // オプションはいくつ並んでもよい(cmd では / 始まりもオプション扱い。posix の / はパスなので位置引数)。
    if (tok.startsWith('-') || (!posix && tok.startsWith('/'))) continue;
    const sub: DictNode | undefined = subsOpen ? findChild(node.subs, tok) : undefined;
    if (sub) {
      node = sub;
      if (sub.opts) optsPath.push(sub.opts);
    } else {
      subsOpen = false; // 位置引数。これ以降はサブコマンドを出さない
    }
  }
  if (lastIsValue) return EMPTY;

  let children: DictNode[];
  if (atStart) {
    children = dict;
  } else if (node === null) {
    return EMPTY; // 未知コマンドの引数位置では候補なし
  } else {
    // 現在位置のサブコマンド → そのオプション → 祖先のオプションの順。
    children = [...(subsOpen ? (node.subs ?? []) : []), ...[...optsPath].reverse().flat()];
  }

  const prefix = keyOf(lastToken);
  const items: Candidate[] = [];
  const seen = new Set<string>(); // 親とサブコマンドで同名オプションがあるとき 1 回だけ出す(先に出た=内側のものを残す)
  for (const child of children) {
    if (items.length >= limit) break;
    const w = keyOf(child.word);
    if (!w.startsWith(prefix)) continue;
    if (w === prefix) continue; // 完全一致は続きが無いので出さない
    if (seen.has(w)) continue;
    seen.add(w);
    // insert は「打った文字はそのままに残りだけ」(cmd で IPCO → nfig)。
    items.push({ word: child.word, insert: child.word.slice(lastToken.length), description: child.desc });
  }
  return { ghost: lastToken !== '' && items.length > 0 ? items[0].insert : null, items };
}
