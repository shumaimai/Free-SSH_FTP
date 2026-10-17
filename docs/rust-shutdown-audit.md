# Rust候補の終了処理監査

対象は `feature/rust-session-shutdown`（#179の `ac876ec` に依存）。2026-10-15に
実装担当とは別のエージェントが差分を静的に監査した。実行試験の担当は実装側であり、
この監査をWindows実機・全機能互換の認定には使わない。

| 指摘 | 対応 | 確認 |
| --- | --- | --- |
| P1: PTY Dropの無期限joinで明示終了も止まる | 通常closeの子/入出力待ちを期限付きにし、Dropは専用reaperへ資源を渡す。Unix前景process groupも終了 | setsidでPTYを保持する子のLinux再現試験。closeは未確認を返し、Dropは待たない |
| P2: エディタ処理中は終了確認から永久に進めない | 確認段階の30秒後にも明示終了経路を用意し、経過時間を終了処理へ引き継ぐ | 静的再監査で経路を確認。応答しない実SMBの試験は未実施 |
| P2: temp削除より親mode復元が先行し、削除失敗も隠れる | 通常取消/失敗とcommit所有タスクでtemp削除をawait。fallback cleanup失敗も終了結果へ保持 | 親000へのupload取消、原本保持、mode復元、temp残存なしをOpenSSHの4接続経路で確認 |

再監査では、上記3修正を確認し、追加のP1/P2阻害事項は見つからなかった。
遅いForwardReadyを終了対象に保持すること、Ready送信後にprimaryを完了する順序、
転送エラーの終了結果への併記も確認された。

別のエージェントも終了順と各bounded queueの排出を独立に静的監査し、追加P1/P2阻害事項は
見つからなかった。forward開始/停止・権限復旧の失敗は `Message::Done.failed` で終了結果へ
保持する。転送temp削除失敗は `Cleanup` variantで確定結果不明と区別し、chmod再試行へ流さない。
既存の編集用temp削除失敗の通知、SSH内部のagent/forward通信と接続途中hop cleanupの全joinは
baseにも残っていた別件であり、今回解決した範囲には含めない。

実装側の試験はRust単体、Python既存試験、Python/Rust契約、Linux実GUIを用いる。
GUIはBOM/CRLF保持のCtrl+S、通常WM_DELETE_WINDOW終了、実ローカルPTYのPID回収、
未保存の編集がある場合の終了抑止を確認する。権限単体はchmod ACK途中の取消後も
shutdownが復元を待ち、以後のacquire/recoverを拒否することを確認する。

Windows ConPTY実機、全通信/保存障害、強制終了時の確定/復元完了、別sessionへ離脱した
Unix子孫の完全回収は未保証。30秒後の明示終了は未確認を伝え、journalを保持する。
OS強制終了や電源断で完了を保証しない。全Rust移行の完了条件はIssue #170と
`rust-compatibility.md` で引き続き追跡する。

## PR #180追加監査への対応

[コメント6191522507](https://github.com/shumaimai/Free-SSH_FTP/pull/180#issuecomment-6191522507)
の指摘1/2には既に外側のclosing guardがあり、提示されたpanic/freezeは未再現だった。
停止はWork.startの受付token取得後にhandleを一覧から移し、spawn拒否による所有権喪失を防ぐ。
端末描画自身にも終了guardを置き、resizeはtry_lockと保留サイズでGUIを待たせず再試行する。
PTYはtry_waitの回収済み状態を保持し、close失敗後のstop/DropでもPID/PGIDにkillを再送しない。
未復元件数の表示は以前の記録を含むと明示する。

RemoteTempはBatch.cleanupのtokenを通常/commit/Drop fallbackの所有タスクへ渡す。
Batch.finish/Dropはその完了後に権限を復元するため、通常以外の削除も親000への復元に先行する。
削除失敗時は復元を試み、失敗結果・未復元journalを保持する。断線/強制終了の完了保証はしない。
LinuxのOpenSSH試験は9条件でtoken取消とfutureそのもののDropの両方を再現する。
単体試験はcleanup未完了時のshutdown待ち、回収済み状態、mutex保持中のGUI描画/resize再送も確認する。
