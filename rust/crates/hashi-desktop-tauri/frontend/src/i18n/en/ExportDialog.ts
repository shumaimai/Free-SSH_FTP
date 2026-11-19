export default {
  'パスフレーズは{n}文字以上にしてください': 'Use a passphrase of at least {n} characters',
  '確認用のパスフレーズが一致しません': 'The confirmation passphrase does not match',
  '{file} へ{n}件の接続先を書き出しました（ホスト鍵{h}件・暗号化した秘密{s}件）':
    'Exported {n} connection(s) to {file} ({h} host key(s), {s} encrypted secret(s))',
  '{file} へ{n}件の接続先を書き出しました（ホスト鍵{h}件・パスワードは含めていません）':
    'Exported {n} connection(s) to {file} ({h} host key(s), passwords not included)',
  '接続先を書き出す': 'Export connections',
  '選んだ接続先の設定と承認済みのホスト鍵を1つのファイルにします。別のパソコンの Hashi（旧版を含む）の「取り込む」で読み込めます。':
    'Saves the selected connections and their trusted host keys to one file. Hashi on another computer (including the old version) can read it with "Import".',
  '接続先（{n}件を選択）': 'Connections ({n} selected)',
  'すべて外す': 'Clear all',
  'すべて選ぶ': 'Select all',
  '{label}を書き出す': 'Export {label}',
  '保存済みのパスワード・パスフレーズも含める（パスフレーズで暗号化）': 'Also include saved passwords and passphrases (encrypted with a passphrase)',
  'パスフレーズ': 'Passphrase',
  'パスフレーズ（確認）': 'Passphrase (confirm)',
  '取り込むときにこのパスフレーズが必要です。忘れると、パスワードは取り出せません（接続先の設定は読めます）。':
    'You need this passphrase when importing. If you forget it, the passwords cannot be recovered (the connection settings can still be read).',
  'キャンセル': 'Cancel',
  '書き出す…': 'Export…',
} as Record<string, string>;
