import { test } from 'node:test';
import assert from 'node:assert/strict';
import { complete } from '../src/completion/engine.ts';

function words(shell: 'posix' | 'cmd', line: string, limit = 50): string[] {
  return complete(shell, line, limit).items.map((i) => i.word);
}

test('行頭の前方一致', () => {
  const r = complete('posix', 'sys');
  assert.equal(r.items[0].word, 'systemctl');
  assert.equal(r.items[0].insert, 'temctl');
  assert.equal(r.ghost, 'temctl');
});

test('サブコマンドの補完', () => {
  const r = complete('posix', 'systemctl st');
  const ws = r.items.map((i) => i.word);
  assert.ok(ws.includes('status'));
  assert.ok(ws.includes('start'));
  assert.ok(ws.includes('stop'));
  assert.equal(r.items[0].word, 'status'); // 辞書の並び順
  assert.equal(r.ghost, 'atus');
});

test('入れ子のサブコマンド(docker compose)', () => {
  assert.ok(words('posix', 'docker compose ').includes('up'));
  const r = complete('posix', 'docker compose u');
  assert.equal(r.items[0].word, 'up');
  assert.equal(r.ghost, 'p');
});

test('オプションの補完(- で始まる)', () => {
  const r = complete('posix', 'tail -');
  assert.equal(r.items[0].word, '-f');
  assert.equal(r.ghost, 'f');
  assert.ok(words('posix', 'tail -').includes('-n'));
});

test('オプションはいくつ並んでもよい', () => {
  assert.ok(words('posix', 'grep -r -i -').includes('-n'));
});

test('パイプの後は新しいコマンドの先頭', () => {
  const r = complete('posix', 'ls | gre');
  assert.equal(r.items[0].word, 'grep');
  assert.equal(r.ghost, 'p');
});

test('空白無しのパイプでも区切る', () => {
  const r = complete('posix', 'ls|gr');
  assert.ok(r.items.map((i) => i.word).includes('grep'));
});

test('&& と ; と & の後も新しいコマンドの先頭', () => {
  assert.equal(complete('posix', 'ls && ta').items[0].word, 'tail');
  assert.equal(complete('posix', 'ls; ta').items[0].word, 'tail');
  assert.equal(complete('posix', 'sleep 1 & ta').items[0].word, 'tail');
  assert.equal(complete('posix', 'ls || ec').items[0].word, 'echo');
});

test('sudo を読み飛ばす', () => {
  const r = complete('posix', 'sudo sys');
  assert.equal(r.items[0].word, 'systemctl');
  assert.equal(r.ghost, 'temctl');
});

test('sudo -u user を読み飛ばす', () => {
  assert.equal(complete('posix', 'sudo -u root sys').items[0].word, 'systemctl');
  assert.equal(complete('posix', 'sudo -uroot sys').items[0].word, 'systemctl');
  assert.equal(complete('posix', 'sudo --user root sys').items[0].word, 'systemctl');
  // -u の値を打っている途中は候補なし
  assert.deepEqual(complete('posix', 'sudo -u ro').items, []);
  assert.equal(complete('posix', 'sudo -u ro').ghost, null);
  assert.deepEqual(complete('posix', 'sudo -u ').items, []);
});

test('sudo の値を取らないオプションはそのまま読み飛ばす', () => {
  assert.equal(complete('posix', 'sudo -E sys').items[0].word, 'systemctl');
  assert.equal(complete('posix', 'sudo -i sys').items[0].word, 'systemctl');
});

test('環境変数代入を読み飛ばす', () => {
  assert.equal(complete('posix', 'FOO=1 sys').items[0].word, 'systemctl');
  assert.equal(complete('posix', 'FOO=1 BAR=2 sys').items[0].word, 'systemctl');
  assert.equal(complete('posix', 'sudo FOO=1 sys').items[0].word, 'systemctl');
});

test('未閉じの引用符では候補なし', () => {
  for (const line of ['echo "sys', "echo 'sys", 'echo "a b | sys']) {
    const r = complete('posix', line);
    assert.deepEqual(r.items, [], line);
    assert.equal(r.ghost, null);
  }
});

test('閉じた引用符はそのまま通す', () => {
  assert.equal(complete('posix', 'echo "a" | sys').items[0].word, 'systemctl');
  assert.equal(complete('posix', "echo 'a' | sys").items[0].word, 'systemctl');
});

test('バックスラッシュでエスケープされた引用符は開かない', () => {
  assert.equal(complete('posix', 'echo a\\"b | sys').items[0].word, 'systemctl');
  // 単一引用符の中ではバックスラッシュは効かない → 'a\' で閉じ、b' が未閉じ
  assert.deepEqual(complete('posix', "echo 'a\\'b' | sys").items, []);
});

test('2>&1 はコマンド区切りではない', () => {
  assert.ok(words('posix', 'ls 2>&1 ').includes('-l'));
  assert.equal(complete('posix', 'echo x 2>&1 | sys').items[0].word, 'systemctl');
});

test('空トークンでは ghost は null で items だけ返る', () => {
  const r = complete('posix', 'ls ');
  assert.equal(r.ghost, null);
  assert.ok(r.items.map((i) => i.word).includes('-l'));
  const r2 = complete('posix', '');
  assert.equal(r2.ghost, null);
  assert.ok(r2.items.length > 0);
  const r3 = complete('posix', 'git ');
  assert.equal(r3.ghost, null);
  assert.ok(r3.items.map((i) => i.word).includes('status'));
});

test('完全一致したものは候補から除く', () => {
  const r = complete('posix', 'systemctl status');
  assert.deepEqual(r.items, []);
  assert.equal(r.ghost, null);
  assert.equal(complete('posix', 'systemctl statu').items[0].word, 'status');
});

test('オプションはサブコマンドの後でも出る(apt install -y)', () => {
  assert.ok(words('posix', 'apt install -').includes('-y'));
  // 完全一致は除く
  assert.deepEqual(complete('posix', 'apt install nginx -y').items, []);
});

test('未知のコマンドの引数位置では候補なし', () => {
  assert.deepEqual(complete('posix', 'foobar x').items, []);
  assert.deepEqual(complete('posix', 'foobar ').items, []);
  assert.equal(complete('posix', 'foobar x').ghost, null);
});

test('既知コマンドの引数位置ではオプションのみ', () => {
  const r = complete('posix', 'ls /tmp ');
  assert.ok(r.items.every((i) => i.word.startsWith('-')));
  // 位置引数の後はサブコマンドを出さない
  assert.ok(!words('posix', 'apt install nginx i').includes('install'));
});

test('非 ASCII を含む行は候補なし', () => {
  assert.deepEqual(complete('posix', '日本語').items, []);
  assert.deepEqual(complete('posix', 'ls 日本').items, []);
});

test('長すぎる行は候補なし', () => {
  assert.deepEqual(complete('posix', 'a'.repeat(4097)).items, []);
  assert.ok(complete('posix', 'a'.repeat(4096)).items.length >= 0);
});

test('limit 件まで', () => {
  assert.equal(complete('posix', '', 3).items.length, 3);
  assert.equal(complete('posix', 's', 2).items.length, 2);
  assert.ok(complete('posix', '', 0).items.length === 0);
});

test('1トークンずつ補完する', () => {
  const r = complete('posix', 'docker l');
  assert.equal(r.items[0].word, 'logs'); // 'logs -f' ではない
  assert.equal(r.items[0].insert, 'ogs');
});

test('ps aux のような引数に見えるオプションも候補になる', () => {
  assert.equal(complete('posix', 'ps a').items[0].word, 'aux');
});

test('cmd: 大文字小文字を区別せず insert は残りだけ', () => {
  const r = complete('cmd', 'IPCO');
  assert.equal(r.items[0].word, 'ipconfig');
  assert.equal(r.items[0].insert, 'nfig');
  assert.equal(r.ghost, 'nfig');
});

test('cmd: 完全一致は除く', () => {
  assert.deepEqual(complete('cmd', 'ipconfig').items, []);
  assert.deepEqual(complete('cmd', 'IPCONFIG').items, []);
});

test('cmd: / オプション', () => {
  const ws = words('cmd', 'IPCONFIG /');
  assert.ok(ws.includes('/all'));
  assert.ok(ws.includes('/flushdns'));
  assert.equal(complete('cmd', 'taskkill /').items[0].word, '/PID');
});

test('cmd: バックスラッシュはエスケープではない(パスの区切り)', () => {
  // "C:\" の " は cmd では閉じ引用符(posix なら \" がエスケープされて未閉じになる)
  assert.equal(complete('cmd', 'dir "C:\\" & tas').items[0].word, 'tasklist');
  assert.deepEqual(complete('posix', 'dir "C:\\" & tas').items, []);
  // 引用符が開いたままなら候補なし
  assert.deepEqual(complete('cmd', 'dir "C:\\Pro').items, []);
});

test('cmd: サブコマンド(net use)', () => {
  assert.equal(complete('cmd', 'net u').items[0].word, 'use');
  assert.ok(words('cmd', 'winget ').includes('install'));
});

test('cmd: 区切りの後は新しいコマンド', () => {
  assert.equal(complete('cmd', 'dir & ip').items[0].word, 'ipconfig');
});

test('親とサブコマンドの同名オプションは1回だけ出す', () => {
  const p = complete('posix', 'git add -').items.filter((i) => i.word === '-p');
  assert.equal(p.length, 1);
  assert.equal(p[0].description, '変更を選んでステージ'); // 内側(git add)の説明を残す
  const v = complete('posix', 'docker run -').items.filter((i) => i.word === '-v');
  assert.equal(v.length, 1);
  assert.equal(v[0].description, 'ボリュームをマウント'); // docker の -v(バージョン)は出さない
  const f = complete('posix', 'docker compose logs -').items.filter((i) => i.word === '-f');
  assert.equal(f.length, 1);
  assert.equal(f[0].description, '新着を追い続ける'); // compose の -f(ファイル指定)は出さない
});

test('cmd: / オプションも位置引数にしない(その後もサブコマンドを出す)', () => {
  // /foo はオプション扱いで読み飛ばすので、次の u で use が候補になる
  assert.ok(words('cmd', 'net /foo u').includes('use'));
  // posix の / はパス(位置引数)なので、その後はサブコマンドを出さない
  assert.ok(!words('posix', 'git /x st').includes('status'));
});

test('cmd: - で始まるオプションは大小文字を区別する', () => {
  // ssh の -l(ユーザー名)と -L(ローカルフォワード)は別物
  const afterL = words('cmd', 'ssh -l');
  assert.ok(!afterL.includes('-L'));
  assert.ok(!afterL.includes('-l')); // 完全一致は除く
  assert.ok(!words('cmd', 'ssh -L').includes('-l'));
  // curl の -o と -O も別物
  assert.ok(!words('cmd', 'curl -o').includes('-O'));
  assert.ok(!words('cmd', 'curl -O').includes('-o'));
  // `-` だけなら大小両方が出る(重複除去も大小文字を区別する)
  const all = words('cmd', 'ssh -');
  assert.ok(all.includes('-l'));
  assert.ok(all.includes('-L'));
  // / オプションとサブコマンドは従来どおり区別しない
  assert.equal(complete('cmd', 'IPCONFIG /FLU').items[0].word, '/flushdns');
  assert.ok(words('cmd', 'NET U').includes('use'));
});
