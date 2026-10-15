# Rust候補の途中監査への対応

対象はIssue #170の[コメント6186466574](https://github.com/shumaimai/Free-SSH_FTP/issues/170#issuecomment-6186466574)
とPR #180の[コメント6191522507](https://github.com/shumaimai/Free-SSH_FTP/pull/180#issuecomment-6191522507)。
元の監査はそのまま履歴に残し、対応はこの別文書に保存する。全Rust移行の完了を意味しない。

| 途中監査の指摘 | 対応と根拠 | 検証の範囲 |
| --- | --- | --- |
| 1. 新規転送が0600へ変わる | 新規は0666からumaskを適用。原本保持では空probeでmodeを測り、payloadはprivate tempへ書く | OpenSSHのumask022/027とlocal作成modeを確認 |
| 2. Atomicのみで上書き互換性・親権限が変わる | 原本保持を既定とし直接書込みを明示選択。後者は既存inodeへ書き、fsync拡張/親writeを要求しない。権限補助はowner最小bit、既存直接書込みはファイル0200だけ。既存原本保持置換の親writeは増やさず、同じbatchの新規用grantも流用しない | 実OpenSSHの親0500/ファイル000とinode保持、混在再帰、mode復元。別UIDを報告しchmodを拒否するSFTP契約サーバーで非fsync/所有者差を確認（実OSの別所有者ではない） |
| 3. marker異常で安定版が起動しない | PR #171単独で修正。実OSロック競合だけ起動を拒否し、FS/時刻/保存異常は警告して起動。取得済みguardは終了まで保持 | 非対応lock/権限/時刻/encoding/保存例外、実Qt警告と起動継続、両OS CI |
| 4. 旧Python journal/markerを見落とす | 権限補助の有効化前に既定configと残存markerのdata_rootを読取検査。未復元/未知/破損/読取不能は拒否。旧ファイルは変更しない | 実Python workerを強制終了し別rootのjournalを残す契約、未知/破損/symlink拒否・bytes不変 |
| 5. CBC専用サーバー互換 | 現代的な暗号を優先しAES-128/192/256-CBCと3DES-CBCを末尾へ追加。SHA-1署名/MACは追加しない | 各CBCだけを許可したOpenSSHへ実ParamikoとRustで接続し転送/forward/execを確認 |
| 6. WinVault PersistがPythonと異なる | インストール済みkeyring 25.7.0 Windows backendの既定Enterpriseへ一致 | 合成TargetNameをWindows APIで読戻してPersistを検査。利用者の旧store/ドメインroamingは未検証 |
| 7. stackが大きく先に171をreviewすべき | 起動ガード修正は独立PR #171に保存。後段候補へそのcommitを取り込むがmainへmergeしない | PR #171のhead/CIを個別に記録。以降は依存PRとして未mergeで保持 |

転送方式はユーザーの既定変更指定がないため原本保持を維持する。直接書込みは取消・失敗時に
部分更新が残り、原本保持にはrename/fsync対応とACL/ハードリンク等の制限がある。
方式/既存policyはキュー追加時に固定し、resumeは直接appendと表示する。
SFTP v3にファイルID/O_NOFOLLOWがないため、照合後の差替えは完全に防げない。
直接書込みの変更開始後・close応答不明は権限の自動再試行へ流さない。

| 追加の終了監査 | 対応と検証 |
| --- | --- |
| 1. forward停止のpanic/handle喪失 | 既存の外側guardで終了中UIへは到達しない。さらにWork.start成功後だけhandleを移し、拒否時には一覧へ残す。停止のexpectを除去 |
| 2. GUI resizeのblocking lock | 呼出元と描画側の終了guardに加えtry_lock化。競合サイズは保持し次frameで再送。実PTY mutex保持中のegui描画で確認 |
| 3. 回収済みPIDへの再kill | try_wait回収済み状態を保持しstop/DropのPID/PGID killを禁止。別sessionの子がPTYを保持するLinux試験で回収済みとbounded close/非待機Dropを確認 |
| 4. 過去の未復元件数 | 終了表示に「以前の記録を含む」を追加。記録は削除しない |
| 5. fallback temp cleanupと親modeの順序 | RemoteTempがBatch.cleanup tokenを所有し、commit/Dropへ引継ぐ。Batchは削除完了後に親modeを復元。OpenSSH親000のtoken取消/future Drop、単体shutdown待ちで確認 |

別エージェントによる静的監査と実装担当の実行試験は分けて記録する。
Windows実機のIME/ConPTY/資格情報、全終了/通信/保存障害、記録されていない旧独自保存先、
sudoや旧journalの自動復旧、全機能移植は未完了。main/releaseへの反映もこの作業には含めない。

追加の独立静的監査では、同一batchで新規作成用の親writeを既存置換へ流用する問題と、
removeのACK待ち中のFuture Dropでtempのpathを失う問題を検出し、どちらも修正した。
後者はSFTPプロトコルのREMOVE応答を停止し、finish taskをabortしてもscopeが完了せず、
fallbackのNoSuchFile応答確認後に完了する回帰試験を追加した。
最終の2エージェントの静的再監査では追加P1/P2は見つからなかった。実行試験の結果はPRとIssueに記録する。
