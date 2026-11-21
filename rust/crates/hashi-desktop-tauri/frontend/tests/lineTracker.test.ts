import { test } from 'node:test';
import assert from 'node:assert/strict';
import { LineTracker, echoConfirmed, looksLikeSecretPrompt } from '../src/completion/lineTracker.ts';

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
  assert.equal(looksLikeSecretPrompt("user@host's password:"), true);
  assert.equal(looksLikeSecretPrompt('Enter passphrase for key ~/.ssh/id_ed25519:'), true);
  assert.equal(looksLikeSecretPrompt('[sudo] password for user:'), true);
  assert.equal(looksLikeSecretPrompt('Enter PIN:'), true);
  assert.equal(looksLikeSecretPrompt('Verification code:'), true);
  assert.equal(looksLikeSecretPrompt('パスワード:'), true);
  assert.equal(looksLikeSecretPrompt('user@host:~$ ls'), false);
  assert.equal(looksLikeSecretPrompt('spinning up worker'), false); // 'pin' は単語境界で見る
});
