# Rust候補とPython版の互換表

基準はPython版main `692ebe6509517a1f438272d4125d271299f818ac`（1.1.2）。
Issue #170の完了条件をこの表で省略しない。候補に存在する機能と、実機で確認できた機能は別に扱う。

| 機能 | Rust候補の実装 | 残る実装・検証 |
| --- | --- | --- |
| 旧版との保存先分離・実行中マーカー | shared/exclusive OSロック、PID+開始時刻、全起動経路 | マーカー非対応旧版停止の取り込みUI確認、Darwin照会 |
| Profile/Settings/known_hosts/export | 全旧フィールド、atomic state、v1、追加のみmerge | 全設定UI、旧データ取り込み・ロールバックUI |
| SSH/AI秘密・クラウド暗号 | 分離ファイル、scrypt/Fernet、WinVaultの2種類のTargetName | OSストア/暗号化ファイルの選択UI、実Python keyring双方互換 |
| 日本語ネイティブGUI | eframe/egui、NotoSansJP、接続先フォーム、タブ | 旧UIの操作・テーマ・DPI互換、IME実機、終了中ワーカー回収 |
| SSH認証・ホスト鍵 | key/password/agent、RSA SHA-2、公開鍵blob指紋、対象付き認証応答 | 全鍵形式・暗号行列、Windows agent実環境、パスフレーズ再入力行列 |
| ProxyJump | SSH configのHost解決、複数hop、鍵→agent→passwordの別認証、接続失敗時のhop回収 | Match/Includeの対応、全失敗行列、実運用の多段接続 |
| ポート転送 | -L/-R/-D、SOCKS5 CONNECT、GUI開始/停止、限定経路、個別取消 | 実Windowsの全経路、停止通信失敗時の待受残存検証 |
| ローカル端末 | portable-pty、Windows JobObject、専用入力スレッド、bounded出力 | ConPTY Windows CI、子孫/連続終了/高負荷の検証 |
| 端末解析・描画 | alacritty、UTF-8/全角/代替画面/履歴、選択表示/コピー、検索、SGR mouse、右クリックpaste、修飾キー、IME導線 | IME・絵文字/結合文字・リフロー実機、全TUI/操作/キー行列 |
| 端末安全性 | OSC52拒否、OSC8機能起動なし、入力/出力queue上限 | 結合文字セル32個上限を追加。全制御列fuzz・故障注入は未実施 |
| SFTP一覧/転送 | nav/xfer別、bounded raw一覧、32KiB、双方向prefix resume、再帰コピー、キュー/進捗/取消/既存policy | 非POSIXサーバー・名前・リンク・失敗行列、個別overwrite確認、転送metadata保持 |
| リモート再帰削除 | 確認語deleteのGUI、lstat、リンク追跡なし、安全な子名 | 権限・名前・失敗行列 |
| デュアルペイン・W端末 | local/remote二つの一覧、drop送信導線、既存bookmarks形式 | 同期ブラウズ、実D&D、bookmark削除、W端末、再接続・接続モード |
| 編集・ログ・snippets | text/固定長hex、BOM/UTF16 byte order/改行、検索/全置換、軽量highlight、local atomic/SFTP POSIX rename、保存失敗時dirty保持 | 行番号/行移動/折返し/全ショートカット、ACL/ハードリンク、8MiB性能、ログ/snippets UI |
| 権限・サーバー設定 | 明示的な一覧/転送の権限補助、chmod前fsync journal、対象/祖先の参照数、深い順・mode確認後の復元、再接続後の復旧UI | sudo、旧Python journal引継ぎ、終了時の復元完了待ち、sshd/netplan rollback |
| P2P・Google Drive同期 | 暗号v1契約のみ | X25519/SAS、通信、Google Drive現行backend、秘密同期と衝突処理 |
| AI/API/OAuth | AI秘密形式のみ | 3種API、会話/観測、OAuth全検証・更新単一所有者・再認証 |
| CommandBroker/MCP/公式CLI | 未実装 | 許可/対象世代/dedupe、IPC ACL、console HashiMCP、未改変公式CLI導線 |
| 更新・配布 | Rust候補のWindows CI artifact | preview/stableチャンネル分離、最新API保護、releaseビルド、全ライセンス、source/build証跡 |

## 試験の範囲

- Rust単体はstorage・challenge世代照合・端末・PTY・入力backpressureを確認する。
- Python/Rustの別プロセスでロック・v1秘密/クラウド/設定を確認する。
- Linux管理下OpenSSHの試験は合成Ed25519鍵を使い、実ユーザー保存情報を読まない。
- Windows実機は現在利用できないため、実装とCIを先に進める方針。
  CI成功はIME・DPI・物理入力・利用者環境の資格情報検証の代用にしない。
- 起動時間・メモリ・配布サイズは未測定。旧版との優劣はまだ主張しない。

SSH互換の追加段階は、HostName/User/Port/IdentityFile/ProxyJump/ForwardAgentを解決する。
Profileの明示値を優先し、ProxyCommand、未対応のMatch/Includeは直接接続へ格下げせず拒否する。
Linuxの合成OpenSSH環境でdirect/alias/2段hop/agent転送の4経路を確認する。
全経路でexecの締切後の接続維持、-L/-R/-Dのデータ往復と停止後のSSH維持を確認する。
agent試験は合成鍵を入れた専用ssh-agentを使い、利用者のagent鍵を使わない。

エディタ候補は既存ファイルをローカル/リモートで開く。`--edit FILE`でも起動できる。
未編集の内容は元バイト列をそのまま返し、UTF-16のbyte orderも保持する。
既存のBOM/CRLF形式をGUIのCtrl+Sまで確認するLinux実描画試験をCIへ追加する。
保存前とcommit前に元の内容を照合し、外部変更があれば上書きしない。
この照合は他アプリを排他ロックする仕組みではなく、最後の照合とrename間の競合は残る。
POSIX rename未対応のSFTPサーバーでは、元ファイルを保全して保存を拒否する。
ACL/extended attributes/hard linkの完全保持と権限緩和はこの候補では未対応。

SFTP一覧はraw READDIRの途中で100,000件/16MiBの上限を適用し、一覧要求ごとのIDで
古い結果を破棄する。再帰転送は深さ256・件数100,000・パス待ち行列16MiBを上限にする。
リンクは追跡しない。検索結果は10,000件/16MiBまで。GUIの転送は128件・同時4接続までで、
同じ接続の転送を順に進める。既存ファイルpolicyは追加時に固定し、標準は上書きしない。
フォルダ全体のトランザクションではなく、取消後の転送済み/部分ファイルは保持する。
現段階の上書き転送は直接truncateするため、取消/失敗で既存内容が部分データに置き換わる。
原本を保持する一時ファイルと確定置換は次段階で実装する。
最終照合後の外部変更競合、全階層のreparse防御、属性完全保持は継続検証する。

端末検索は履歴の日本語セルも扱い、選択の全角幅を保つ。SGR/legacy mouse、修飾cursor/function key、
右クリックの明示pasteを追加する。OSC52は拒否し、OSC8の隠れたURLは起動しない。
可視textのhttp(s) URLだけをCtrl+クリックで起動する。結合文字は1セル32個までで、
文字/全角flag/表示属性を維持する。IME実機・全TUI/マウス行列・大容量性能は継続検証する。

権限補助は標準で無効。旧Python版停止と旧記録の復元済み確認後だけ有効にする。
マーカー対応Python版は排他OS guardで起動を防ぎ、未対応旧版は利用者の確認に頼る。
preview/native共用の永続journalは`Hashi-Rust-Coordination/permission-journal-v2`で、
LinuxのXDG_CONFIG_HOME（未設定なら.config）、WindowsのLOCALAPPDATAに置く。
runtime領域には復旧記録を置かない。記録の保存とfsyncに失敗すればchmodしない。
未commit tempはpreparing領域で保持し、正常な記録の復旧を妨げない。
接続先/利用者/確認済みホスト鍵へ復旧を束縛する。同名パスは全接続で保守的に排他する。
最終パス/親リンクの別表記はrealpath照合で拒否するが、照合後の外部競合は排除できない。
一時chmodのACK途中取消も所有タスクで完了/復元し、読み戻したmodeが元と一致した記録だけ消す。
子の復元失敗があれば親も延期する。破損・版不明の確定記録は上書き/削除しない。
sudo/root所有ファイル、旧Pythonの未復元記録の自動取り込み、終了時の非同期タスク完了待ちは未対応。
突然終了/断線時は記録を保持し、同じ接続/鍵の再接続後に「権限補助」から明示的に復旧する。
有効な権限補助の一覧/転送/復旧は全タブで直列化し、Grant参加と新規付与の競合を避ける。
権限補助を有効にしていない別タブ/外部アプリの通常操作は取引に参加しないため、一時mode変更で途中失敗する可能性は残る。
