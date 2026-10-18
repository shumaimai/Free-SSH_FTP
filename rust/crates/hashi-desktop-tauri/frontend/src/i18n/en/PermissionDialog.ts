export default {
  '一時的な権限変更': 'Temporary permission changes',
  '一覧と転送で権限が足りないとき、自分が所有するファイルのmodeを一時的に変えて作業し、終わったら元へ戻します。': 'When permissions are insufficient for listing or transfers, the mode of files you own is changed temporarily and restored when the work is done.',
  '復元に失敗したときは記録を残し、再接続後にここから復元できます。sudo・ACL・旧版（Python）の未復元記録の取り込みには対応していません。': 'If a restore fails, the record is kept and you can restore it from here after reconnecting. sudo, ACLs, and importing unrestored records from the old version (Python) are not supported.',
  '旧版に未復元の記録があるときは、有効にできません（旧版で復旧してください）。': 'If the old version has unrestored records, this cannot be enabled (recover them in the old version).',
  '旧Python版をすべて終了した（確認できない旧版も含む）': 'All old Python versions are closed (including any I cannot check)',
  'この接続: {state}': 'This connection: {state}',
  '有効': 'enabled',
  '無効': 'disabled',
  '確認中': 'checking',
  'この接続で有効にしました（接続を閉じるまで）': 'Enabled for this connection (until it is closed)',
  'この接続で有効にする': 'Enable for this connection',
  'この接続の未復元記録を復元': 'Restore unrestored records for this connection',
} as Record<string, string>;
