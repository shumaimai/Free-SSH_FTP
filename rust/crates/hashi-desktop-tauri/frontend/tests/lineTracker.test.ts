import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  LineTracker,
  echoConfirmed,
  isCursorReport,
  isTerminalReport,
  looksLikeSecretPrompt,
} from '../src/completion/lineTracker.ts';

test('印字可能な ASCII を追加する', () => {
  const t = new LineTracker();
  assert.equal(t.line, '');
  t.feed('l');
  t.feed('s');
  assert.equal(t.line, 'ls');
});

test('複数文字を一度に feed できる', () => {
  const t = new LineTracker();
  t.feed('ls -la /tmp');
  assert.equal(t.line, 'ls -la /tmp');
});

test('バックスペースで最後の1文字を消す', () => {
  const t = new LineTracker();
  t.feed('lsa');
  t.feed('\x7f');
  assert.equal(t.line, 'ls');
  t.feed('x\b');
  assert.equal(t.line, 'ls');
});

test('空の行でバックスペースは何もしない', () => {
  const t = new LineTracker();
  t.feed('\x7f');
  assert.equal(t.line, '');
  t.feed('l\x7f\x7f');
  assert.equal(t.line, '');
});

test('Enter と \\n で空の行にリセット', () => {
  const t = new LineTracker();
  t.feed('ls -l');
  t.feed('\r');
  assert.equal(t.line, '');
  t.feed('pwd');
  t.feed('\n');
  assert.equal(t.line, '');
});

test('Ctrl+C と Ctrl+U で空の行にリセット', () => {
  const t = new LineTracker();
  t.feed('ls -');
  t.feed('\x03');
  assert.equal(t.line, '');
  t.feed('ls -');
  t.feed('\x15');
  assert.equal(t.line, '');
});

test('その他の制御文字で不明になる', () => {
  for (const ch of ['\t', '\x12', '\x04', '\x17', '\x01', '\x05', '\x1b']) {
    const t = new LineTracker();
    t.feed('ls');
    t.feed(ch);
    assert.equal(t.line, null, JSON.stringify(ch));
  }
});

test('エスケープ列(矢印など)は先頭の ESC で不明になる', () => {
  const t = new LineTracker();
  t.feed('ls');
  t.feed('\x1b[D'); // 左矢印
  assert.equal(t.line, null);
});

test('不明のあいだは追加・削除しても null のまま', () => {
  const t = new LineTracker();
  t.feed('ls\t');
  assert.equal(t.line, null);
  t.feed('abc');
  t.feed('\x7f');
  assert.equal(t.line, null);
});

test('不明から Enter / Ctrl+C / Ctrl+U で復帰する', () => {
  for (const reset of ['\r', '\n', '\x03', '\x15']) {
    const t = new LineTracker();
    t.feed('ls\t'); // Tab で不明へ
    t.feed('x');
    t.feed(reset);
    assert.equal(t.line, '', JSON.stringify(reset));
    t.feed('pwd');
    assert.equal(t.line, 'pwd');
  }
});

test('非 ASCII(IME 確定など)で不明になる', () => {
  const t = new LineTracker();
  t.feed('ls');
  t.feed('あ');
  assert.equal(t.line, null);
  t.feed('\r');
  assert.equal(t.line, '');
});

test('行が 4096 文字を超えたら不明', () => {
  const t = new LineTracker();
  t.feed('a'.repeat(4096));
  assert.equal(t.line?.length, 4096);
  t.feed('a');
  assert.equal(t.line, null);
});

test('markUnknown で不明になる', () => {
  const t = new LineTracker();
  t.feed('ls');
  t.markUnknown();
  assert.equal(t.line, null);
  t.feed('\r');
  assert.equal(t.line, '');
});

test('reset で空の行に戻る(不明も解除)', () => {
  const t = new LineTracker();
  t.feed('ls\t');
  t.reset();
  assert.equal(t.line, '');
});

test('echoConfirmed: 画面のカーソル前が行で終わるか', () => {
  assert.equal(echoConfirmed('$ ls -l', 'ls -l'), true);
  assert.equal(echoConfirmed('$ ls -l', 'pwd'), false);
  assert.equal(echoConfirmed('', ''), true);
  assert.equal(echoConfirmed('$ pwd', ''), true);
  // 末尾の空白: line が空白で終わるとき画面側も空白で終わっていること
  assert.equal(echoConfirmed('$ ls ', 'ls '), true);
  assert.equal(echoConfirmed('$ ls', 'ls '), false);
});

test('looksLikeSecretPrompt: パスワード等の入力欄を検知', () => {
  // 末尾が `:` や `?` の入力待ちに見えるものは true
  assert.equal(looksLikeSecretPrompt("user@host's password:"), true);
  assert.equal(looksLikeSecretPrompt('Password: '), true);
  assert.equal(looksLikeSecretPrompt("Enter passphrase for key '/home/u/.ssh/id_ed25519': "), true);
  assert.equal(looksLikeSecretPrompt('[sudo] password for u: '), true);
  assert.equal(looksLikeSecretPrompt('Enter PIN:'), true);
  assert.equal(looksLikeSecretPrompt('PIN: '), true);
  assert.equal(looksLikeSecretPrompt('Verification code: '), true);
  assert.equal(looksLikeSecretPrompt('Token:'), true);
  assert.equal(looksLikeSecretPrompt('パスワード: '), true); // i18n-ignore: 照合用
  // 普通のプロンプト(語が末尾の :/? に結び付かない)は false
  assert.equal(looksLikeSecretPrompt('user@host:~$ ls'), false);
  assert.equal(looksLikeSecretPrompt('u@h:~/password-tool$ '), false); // ディレクトリ名で誤爆しない
  assert.equal(looksLikeSecretPrompt('C:\\pins> '), false);
  assert.equal(looksLikeSecretPrompt('u@h:~$ '), false);
  assert.equal(looksLikeSecretPrompt('spinning up worker'), false);
});

test('Ctrl+Space の NUL では行の中身を変えず、不明にもしない', () => {
  const t = new LineTracker();
  t.feed('ipc');
  t.feed('\x00');
  assert.equal(t.line, 'ipc');
  t.feed('\x7f');
  assert.equal(t.line, 'ip');
});

test('端末が自動で送る報告を見分ける', () => {
  for (const r of ['\x1b[I', '\x1b[O', '\x1b[?1;2c', '\x1b[>0;276;0c', '\x1b[?2004;1$y', '\x1b]11;rgb:0000/0000/0000\x1b\\', '\x1b]10;rgb:ffff/ffff/ffff\x07']) {
    assert.equal(isTerminalReport(r), true, JSON.stringify(r));
  }
  // キー入力(矢印・修飾付き矢印・Alt+文字・Delete・F3)は報告ではない
  for (const k of ['\x1b[A', '\x1b[1;5C', '\x1bI', '\x1b[3~', '\x1bOR', 'I', '\x1b[1;2R', '\x1b[I\x1b[A']) {
    assert.equal(isTerminalReport(k), false, JSON.stringify(k));
  }
});

test('カーソル位置の報告の形', () => {
  assert.equal(isCursorReport('\x1b[1;1R'), true);
  assert.equal(isCursorReport('\x1b[24;80R'), true);
  assert.equal(isCursorReport('\x1b[A'), false);
  assert.equal(isCursorReport('\x1b[24;80R\x1b[A'), false);
});
