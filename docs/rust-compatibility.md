# Rust候補とPython版の互換表

基準はPython版main `692ebe6509517a1f438272d4125d271299f818ac`（1.1.2）。
Issue #170の完了条件をこの表で省略しない。候補に存在する機能と、実機で確認できた機能は別に扱う。

| 機能 | Rust候補の実装 | 残る実装・検証 |
| --- | --- | --- |
| 旧版との保存先分離・実行中マーカー | shared/exclusive OSロック、PID+開始時刻、記録失敗は警告してPython起動、旧journalの検出 | マーカー非対応旧版停止の取り込みUI確認、Darwin照会 |
| Profile/Settings/known_hosts/export | 全旧フィールド、atomic state、v1、追加のみmerge | 全設定UI、旧データ取り込み・ロールバックUI |
| SSH/AI秘密・クラウド暗号 | 分離ファイル、scrypt/Fernet、WinVaultの2種類のTargetNameとPython互換Enterprise永続化 | OSストア/暗号化ファイルの選択UI、実Python keyring双方互換 |
| 日本語ネイティブGUI | eframe/egui、NotoSansJP、接続先フォーム、タブ、受付停止→処理/復元待ち→接続終了 | 旧UIの操作・テーマ・DPI互換、IME実機、全終了故障行列 |
| SSH認証・ホスト鍵 | key/password/agent、RSA SHA-2、公開鍵blob指紋、対象付き認証応答 | 全鍵形式・SHA-1専用サーバー等の暗号行列、Windows agent実環境、パスフレーズ再入力行列 |
| ProxyJump | SSH configのHost解決、複数hop、鍵→agent→passwordの別認証、接続失敗時のhop回収 | Match/Includeの対応、全失敗行列、実運用の多段接続 |
| ポート転送 | -L/-R/-D、SOCKS5 CONNECT、GUI開始/停止、限定経路、個別取消、停止の受付token確保後にハンドルを移す | 実Windowsの全経路、停止通信失敗時の待受残存検証 |
| ローカル端末 | portable-pty、Windows JobObject、専用入出力スレッド、bounded出力、通常closeの回収と非待機Drop、回収済みPIDへの再送信抑止、GUI resizeの非待機lock | 子孫/連続終了/高負荷の全行列、Unixの別sessionに離脱した子の回収 |
| 端末解析・描画 | alacritty、UTF-8/全角/代替画面/履歴、選択表示/コピー、検索、SGR mouse、右クリックpaste、修飾キー、IME導線 | IME・絵文字/結合文字・リフロー実機、全TUI/操作/キー行列 |
| 端末安全性 | OSC52拒否、OSC8機能起動なし、入力/出力queue上限 | 結合文字セル32個上限を追加。全制御列fuzz・故障注入は未実施 |
| SFTP一覧/転送 | nav/xfer別、bounded raw一覧、32KiB、双方向prefix resume、再帰コピー、キュー/進捗/取消/既存policy、原本保持/直接書込みの明示選択、新規umask互換 | 非POSIXサーバー・名前・リンク・失敗行列、個別overwrite確認、転送metadata保持 |
| リモート再帰削除 | 確認語deleteのGUI、lstat、リンク追跡なし、安全な子名 | 権限・名前・失敗行列 |
| デュアルペイン・W端末 | local/remote二つの一覧、drop送信導線、既存bookmarks形式 | 同期ブラウズ、実D&D、bookmark削除、W端末、再接続・接続モード |
| 編集・ログ・snippets | text/固定長hex、BOM/UTF16 byte order/改行、検索/全置換、軽量highlight、local atomic/SFTP POSIX rename、保存失敗時dirty保持。snippets（Python形式・`{{変数}}`・Enter送信・並べ替え）、SSH受信のセッションログ（LF確定行・有界キュー・0600）、設定画面のログ項目 | 行番号/行移動/折返し/全ショートカット、ACL/ハードリンク、8MiB性能。ログは代替画面とカーソル移動だけで描いた行を記録しない（Python版の画面差分方式と違う） |
| 権限・サーバー設定 | 明示的な一覧/転送の権限補助、chmod前fsync journal、対象/祖先の参照数、深い順・mode確認後の復元、終了時の復元待ち、再接続後の復旧UI | sudo、旧Python journalの復元/引継ぎUI、全終了故障行列、sshd/netplan rollback |
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
Linuxの合成OpenSSH環境でdirect/alias/2段hop/agent転送と、AES-128/192/256-CBC、3DES-CBC専用、
umask 027の計9条件を確認する。CBC条件では同じサーバーへ実Paramikoでも接続する。
現代的なChaCha/GCM/CTRを優先し、CBCは末尾の互換候補とする。SHA-1署名/MACは追加していない。
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
フォルダ全体のトランザクションではなく、確定済みファイルとresumeの部分ファイルは保持する。
転送方式はジョブ追加時に固定する。「原本保持」（既定）は同じdirのprivate tempへ32KiBずつ書き、
サイズ/metadata照合と同期後に確定する。「直接書込み」は既存inodeへ書くので所有者/ハードリンクを
維持でき、fsync拡張や親dirの書込み権限を要求しないが、取消/失敗時の部分更新を残す。
方式を自動で切り替えない。resumeは方式選択に関わらず直接appendと表示する。
新規ファイルはローカル/サーバーで0666から通常作成し、umask適用後のmodeを用いる。
原本保持では内容を書かない空probeで新規modeを測り、payloadは0600のtempへ書く。
直接書込みでは変更前にlstat/handle metadataを照合し、localはO_NOFOLLOW/reparse確認も行う。
SFTP v3にはファイルIDとO_NOFOLLOWがなく、最終照合後の差替え競合は完全には排除できない。
変更開始後の直接書込みエラーとclose応答不明は、chmodの自動再試行へ流さない。
確定前の取消/失敗では宛先の原本を保持する。remote uploadはfsync拡張、既存上書きはPOSIX renameを
変更前に要求し、未対応サーバーでは拒否する。確定ACK不明と確定後の同期/属性確認失敗は
chmod自動再試行へ流さず、権限復元の失敗も併記する。GUIは確定応答/復元まで待つ。
通常転送の未確定private tempはcleanupする。断線/終了/open ACK不明時には残る場合がある。
EXCL open成功を確認したtempだけ削除する。resumeはprefix照合後の既存部分ファイルへの直接append。
metadataが同じ長さ/mtime（SFTPは秒単位）の外部変更と最終照合後の競合は検出できない。
Unixは既存uid/gid/modeを保持できなければ確定前に拒否する。Windowsは確定したhandleでreadonlyを
再適用/読戻しする。ACL/extended attributes/hardlinksの完全保持は未対応。
Linuxの読み取れない親dirでは確定前に拒否する。対応サーバー/ファイルシステム行列は継続検証する。
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
所有者bitだけを追加する（祖先検索0100、一覧0500、新規親0300、既存直接書込み0200等）。
既存の原本保持置換のために親の書込み権限を緩めない。同一再帰batch内の新規作成用grantも流用しない。
権限補助を有効化する前に、旧Pythonの既定configルートと残存マーカーが記録したdata_rootの
perm_journal.jsonを読取専用で検査する。未復元/未知記録・破損・読取失敗は有効化を拒否する。
旧マーカー/記録は削除・書換えず、Python側で復元する。記録されていない独自保存先は確認できない。
sudo/root所有ファイル、旧Pythonの未復元記録の自動取り込みは未対応。
突然終了/断線時は記録を保持し、同じ接続/鍵の再接続後に「権限補助」から明示的に復旧する。
有効な権限補助の一覧/転送/復旧は全タブで直列化し、Grant参加と新規付与の競合を避ける。
権限補助を有効にしていない別タブ/外部アプリの通常操作は取引に参加しないため、一時mode変更で途中失敗する可能性は残る。

通常のタブ/アプリ終了は新しい操作を拒否し、取消tokenを伝えて操作・確定ACK・権限復元を待つ。
tempは権限scope内で削除を確認する。future Dropのfallbackとcommit所有タスクもscopeのtokenを保持し、
Batchの復元はその完了後に行う。削除失敗は元の操作結果と併記し、fallback cleanupの失敗も
接続終了画面へ残す。終了時の未復元件数には以前の記録も含むことを明示する。復元待ち後に転送停止・SSH切断・ローカルPTY回収を進める。
接続途中の遅いReadyも同じqueueで処理してから資源を閉じ、切断したタブの編集bufferは残す。
未保存の編集は破棄確認を求める。読込/保存が応答しない確認段階にも30秒後の終了導線を設ける。
30秒以上待って利用者が明示終了を選んだ場合は、確定結果/復元/cleanupの完了を保証しない。
復旧記録は消さず、次の接続で確認する。OSの強制終了や電源断も完了保証の対象外。
通常PTY closeは子の終了と入出力threadを確認する。Unixは元/前景process groupを終了するが、
別sessionへ離脱してPTYを保持した子は残り得る。その場合joinの待ちを打ち切って未確認とし、
Dropは専用reaperへ渡す。WindowsのConPTY close自体が戻らない場合もGUIでjoinを待たない。
強制終了時のruntime待ちは1秒までで、残るblocking workerを回収済みと主張しない。
Linux実描画試験は通常WM_DELETE_WINDOW要求からの終了とローカルPTYのPID回収を確認する。
Windows物理入力/IME、全通信障害/保存障害、別sessionの子孫回収は引き続き未検証/未対応。
完了待ちの対象は画面の操作・転送確定/cleanup・権限復元・選択したforwardとPTYである。
SSH内部のagent/forward通信、接続途中hop cleanupの全joinは未対応で、取消/切断を要求する範囲。
編集用 `.hashi-edit-*` temp削除失敗の通知も既存の未対応項目として残る。

今回の途中監査と追加終了監査への項目別対応・検証限界は `rust-progress-audit.md` に保存する。
WinVaultのPersistはkeyring 25.7.0のWindows backend既定と同じEnterpriseにし、Windows CIで
合成TargetNameのCredRead読戻しを行う。実利用者の旧資格情報とドメインroamingは未検証。

セッションログはSSH端末だけを対象にし、CMDは記録しない（Python版と同じ）。LFを受けた時点のカーソル行を、
折返しでつながる行と合わせて記録する。CRによる上書きは画面に残った表示を記録し、vim等の代替画面は記録しない。
書込みは専用スレッドへ有界キューで渡し、GUIでI/Oを待たない。満杯/書込み失敗はそのタブのログを停止し、
理由を表示する（黙って欠落させない）。新規ファイルはUnixで0600。既定の保存先は各版の設定ルート/logsで、旧Python版のlogsへ書かない。
スニペットはPython版`snippets.json`と同じ`[{name, body, send_enter}]`。型が違う記録は上書きせず編集を拒否する。
