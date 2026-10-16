# Rust候補とPython版の互換表

基準はPython版main `692ebe6509517a1f438272d4125d271299f818ac`（1.1.2）。
Issue #170の完了条件をこの表で省略しない。候補に存在する機能と、実機で確認できた機能は別に扱う。

| 機能 | Rust候補の実装 | 残る実装・検証 |
| --- | --- | --- |
| 旧版との保存先分離・実行中マーカー | shared/exclusive OSロック、PID+開始時刻、記録失敗は警告してPython起動、旧journalの検出 | マーカー非対応旧版停止の取り込みUI確認、Darwin照会 |
| Profile/Settings/known_hosts/export | 全旧フィールド、atomic state、v1、追加のみmerge | 全設定UI、旧データ取り込み・ロールバックUI |
| SSH/AI秘密・クラウド暗号 | 分離ファイル、scrypt/Fernet、WinVaultの2種類のTargetNameとPython互換Enterprise永続化 | OSストア/暗号化ファイルの選択UI、実Python keyring双方互換 |
| Web画面（Tauri 2 + WebView2） | `hashi-desktop-tauri`。Fluent UIのホーム（接続カード）・編集窓・上部タブ・xterm.js端末（ローカルCMD/SSH）・ホスト鍵/秘密入力の確認窓、外観設定（テーマ・アクセント色・フォント・端末配色）、AIドック（接続・会話・端末共有・送信前確認・外部操作の承認）、終了時の接続/PTY回収 | ファイルドック、スニペット・設定・権限補助・ポート転送・鍵生成・エディタ・MCPの移植（現在はegui版）、Windows実機・IME・拡大表示、WebView2の有無の案内 |
| 日本語ネイティブGUI（egui版） | eframe/egui、NotoSansJP、新しい画面構成（ホームの接続カード、接続の編集窓、上部タブ、右の「ファイル/AI」ドック、ツールメニュー）、受付停止→処理/復元待ち→接続終了 | テーマ・DPI、IME実機、全終了故障行列。旧Python版の画面構成は意図して再現しない（機能の対応だけを本表で追う） |
| SSH認証・ホスト鍵 | key/password/agent、RSA SHA-2、公開鍵blob指紋、対象付き認証応答 | 全鍵形式・SHA-1専用サーバー等の暗号行列、Windows agent実環境、パスフレーズ再入力行列 |
| ProxyJump | SSH configのHost解決、複数hop、鍵→agent→passwordの別認証、接続失敗時のhop回収 | Match/Includeの対応、全失敗行列、実運用の多段接続 |
| ポート転送 | -L/-R/-D、SOCKS5 CONNECT、GUI開始/停止、限定経路、個別取消、停止の受付token確保後にハンドルを移す | 実Windowsの全経路、停止通信失敗時の待受残存検証 |
| ローカル端末 | portable-pty、Windows JobObject、専用入出力スレッド、bounded出力、通常closeの回収と非待機Drop、回収済みPIDへの再送信抑止、GUI resizeの非待機lock | 子孫/連続終了/高負荷の全行列、Unixの別sessionに離脱した子の回収 |
| 端末解析・描画 | alacritty、UTF-8/全角/代替画面/履歴、選択表示/コピー、検索、SGR mouse、右クリックpaste、修飾キー、IME導線 | IME・絵文字/結合文字・リフロー実機、全TUI/操作/キー行列 |
| 端末安全性 | OSC52拒否、OSC8機能起動なし、入力/出力queue上限 | 結合文字セル32個上限を追加。全制御列fuzz・故障注入は未実施 |
| SFTP一覧/転送 | nav/xfer別、bounded raw一覧、32KiB、双方向prefix resume、再帰コピー、キュー/進捗/取消/既存policy、原本保持/直接書込みの明示選択、新規umask互換 | 非POSIXサーバー・名前・リンク・失敗行列、個別overwrite確認、転送metadata保持 |
| リモート再帰削除 | 確認語deleteのGUI、lstat、リンク追跡なし、安全な子名 | 権限・名前・失敗行列 |
| デュアルペイン・W端末 | local/remote二つの一覧、drop送信導線、既存bookmarks形式、同期ブラウズ（相対移動・ドライブ/ルート越え拒否・`sync_browse`設定） | 実D&D、bookmark削除、W端末、再接続・接続モード、同期ブラウズの実サーバー深い階層/リンク越え |
| 編集・ログ・snippets | text/固定長hex、BOM/UTF16 byte order/改行、検索/全置換、軽量highlight、local atomic/SFTP POSIX rename、保存失敗時dirty保持。snippets（Python形式・`{{変数}}`・Enter送信・並べ替え）、SSH受信のセッションログ（LF確定行・有界キュー・0600）、設定画面のログ項目 | 行番号/行移動/折返し/全ショートカット、ACL/ハードリンク、8MiB性能。ログは代替画面とカーソル移動だけで描いた行を記録しない（Python版の画面差分方式と違う） |
| 権限・サーバー設定 | 明示的な一覧/転送の権限補助、chmod前fsync journal、対象/祖先の参照数、深い順・mode確認後の復元、終了時の復元待ち、再接続後の復旧UI | sudo、旧Python journalの復元/引継ぎUI、全終了故障行列、sshd/netplan rollback |
| SSH鍵生成・公開鍵登録 | Ed25519/ECDSA(256/384/521)/RSA(2048/3072/4096)、パスフレーズ（bcrypt-pbkdf）、全種別OpenSSH形式、no-clobber/明示上書き、Unix 0600と新規フォルダ0700、Windowsの~/.ssh外警告、SFTPで~/.ssh/authorized_keysへ末尾追記・重複防止 | Windows実機のACL、RSA 4096の体感時間、Python版はRSA/ECDSAを従来PEMで書く差（Python版はOpenSSH形式も読める） |
| P2P・Google Drive同期 | 暗号v1契約のみ | X25519/SAS、通信、Google Drive現行backend、秘密同期と衝突処理 |
| AI/API/OAuth | AI秘密形式、プロバイダー中立のツール定義/秘密除去/観測JSON、APIキー方式（OpenAI Responses・Anthropic Messages・互換Chat Completions・Command Code、モデル一覧、toolsなし接続確認、URL検証・リダイレクト禁止・SSE上限）、会話（最大8往復・取消・観測データの送信前確認/編集）、AI専用暗号化ストアへのキー保存、設定は`ai_connection`（Python版の`ai_api_kind`/`ai_api_profiles`は引き継がない） | ChatGPT OAuth（独立サインイン・アカウント選択・更新単一所有者）、実API（OpenAI/Anthropic/Command Code）での確認、OSの資格情報ストア |
| CommandBroker/MCP/公式CLI | 端末台帳（ID・接続世代・共有可否・ANSI除去した出力/画面）、Broker（読むだけ/操作ごとに確認/自動実行、15分、停止で取消、actor+要求IDの重複防止、本文なし監査）、SSH専用exec/ローカル別プロセスの独立実行、`hashi-ai`の6ツール・秘密除去・観測JSON、`hashi-mcp`の認証付きloopback IPC（本人限定フォルダ・instance/token照合・notification実行禁止・切断で取消）と`HashiMCP-RustPreview` console stdio、GUIの共有/承認/MCP設定画面 | 未改変公式Claude CLIの導入/起動/MCP連携、Windows実機のicacls/ConPTY、AI会話画面 |
| 更新・配布 | Rust候補のWindows CI artifact。preview更新通知（Release一覧から`rust-preview-v`タグ・prerelease・公式URLだけを選ぶ）、Python版の更新通知はPythonタグ形式だけを通知、`rust-preview-v*`専用workflow（prerelease・make_latest:false・版照合・ライセンス同梱） | 実タグでのworkflow初回実行、stable `rust-v`のworkflow、署名/SmartScreen、MCP別EXEの同梱、戻し手順 |

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

更新確認は`GET /repos/shumaimai/Free-SSH_FTP/releases?per_page=100`を読み、draftを除外し、
チャネルの接頭辞+SemVer（buildメタデータなし、stableはprereleaseなし）とGitHubのprerelease区分が一致し、
`html_url`が公式リポジトリのタグページと一致するものだけを比較する。失敗はPython版と同じく表示しない。
HTTPはOSの証明書検証器（rustls-platform-verifier）・5秒・4MiB上限で、HTTPSだけを使う。
Cargoの版は`0.1.0-alpha.0`から始め、プレビュー公開時はタグと同じ版にする（workflowが照合する）。

鍵生成は秘密鍵を同じフォルダの一時ファイルへ書いてから置き換え、既存ファイルは明示した場合だけ上書きする。
公開鍵の登録は既存のauthorized_keysを書き換えず末尾へ追記する（Python版は全体を書き直す）。
読み込みは1MiBまでで、超える場合は登録しない。公開鍵をシェルへ埋め込まない点はPython版と同じ。
実OpenSSHのSFTPで、改行なし既存行への追記・重複防止・0600/0700・新規.ssh作成を確認する（一時.sshを使い実ホームを変更しない）。

同期ブラウズはPython版`mirror_move`と同じ相対移動（共通の親までさかのぼって降りる）だけを写し、絶対パスを写さない。
Windowsのドライブ/UNC共有はntpathと同じに分解し、ドライブをまたぐ移動とルートを突き抜ける移動は追随しない。
跳ね返り防止はPython版の「1回読み飛ばす」フラグではなく、追随で開始した一覧要求のStampだけを読み飛ばす。
追随が失敗/取消されても待ち状態が残らず、次の本物の移動を取りこぼさない。失敗は通知だけで、ダイアログにしない。

操作仲介はPython版`command_broker.py`の規則を引き継ぐ。許可はメモリ内だけで15分、モデルやMCPツールから変更できない。
対象は端末IDと接続世代の組で、再接続（再登録）後の古い世代は拒否し、新しい世代も利用者が選び直す。
確認モードの承認待ちは停止・MCP切断・要求取消で取り消す。実行中の記録はFutureが破棄されても残さない。
対話入力は送信成功だけを返し、完了や終了コードを推測しない。独立実行はSSHの専用execチャネルか
ローカルの別プロセスで、対話端末のcwd/環境を引き継がない（ローカルは開始フォルダ必須）。出力は末尾64KiB。
Windowsのローカル独立実行は旧版と同じく外側cmdをASCIIだけで起動し、本文を環境変数の遅延展開で渡す。
Rust版はUnixでも`/bin/sh -c`の別プロセスグループで独立実行できる（旧版はWindows専用）。
子孫が出力パイプを保持する場合はEOFを0.5秒だけ待ち、`output_complete:false`・completion unknownとする。
MCPの接続情報は本人限定の一時フォルダ（Unix 0700、Windowsはicaclsで継承解除し本人とSYSTEMだけ）に置き、
アプリ終了/MCP停止で削除する。stdioヘルパーはstdoutへJSON-RPCだけを書き、同時4件まで並列化する。
GUIの実描画（Xvfb）で、CMD共有→MCP開始→実stdioヘルパーからlist_sessions/send_input→承認ダイアログ→
CMDでの実行→read_outputまでと、通常終了で接続情報が消えることを確認した。Windows実機は未確認。

AI相談の画面はPython版から作り直した。サービス（OpenAI / Claude / 自分のPC / その他）を選び、APIキーを貼って
「接続する」を押すと、モデル一覧の取得→モデルの自動選択→toolsなしの応答確認→設定とキーの保存を1回で行う。
API形式・種別・URL検証はサービスから内部で決め、「自分のPC」「その他」だけ詳細設定（通信形式・ツール対応）を出す。
Command CodeのURLは「その他」に入力したときだけ自動で専用扱いにする。キー名は従来どおり
`api:{種別}:{正規化URLのSHA-256}`で、設定は`ai_connection`（サービス/URL/モデル/形式/ツール可否）だけに置く。
再起動時は保存キーを読み戻し、通信せずに会話へ戻る。キーは既定でこのPCへ暗号化保存し（チェックで外せる）、
「保存キーを削除」で消せる。APIキーはAI専用の暗号化ストア（`ai-secrets.dat`）へ保存し、SSH資格情報・設定JSONへ混ぜない。
公式サービスは入力済みのURLを無視して公式URLだけを使う。モデル切替は通信せず会話を新しくする。
接続確認ではtoolsを送らない。相談専用（端末ツールなし）ではtoolsを送らず、返されたツール要求も実行しない。
完了イベントのない応答（Responsesのcompleted、Messagesのmessage_stop+stop_reason、Chatのfinish_reason）の
ツールは実行しない。HTTPSだけ（loopbackのHTTPは可）、認証付き通信のリダイレクトを拒否し、POSTを自動再送しない。
共有対象がある送信では、送信前に観測データを確認・編集できる。共有する端末と実行モード（読むだけ/操作は確認/自動で操作）は
会話欄の上で選び、送信のたびにBrokerへ設定する。「外部連携（MCP）」はMCPクライアント向けの共有だけを扱う。
GUI実描画（新UI）で、ローカルの互換API偽サーバーを相手に、1回の接続操作→再起動後の自動復元→送信内容の確認→
AIのsend_input要求がBroker経由でCMDへ届き、ツール結果を返して最終応答を表示するまでを確認した。実サービスは未確認。

## Web画面（Tauri）の方針

見た目を標準的なWindowsアプリに近づけるため、画面だけをWeb（Tauri 2 + WebView2、Fluent UI、xterm.js）で作り直している。
SSH・転送・権限補助・AI・保存などの核はRustのまま共有し、画面とはcommand/eventだけで話す。WebView2が使えない環境では
起動できないため、egui版を移行が済むまで残す。「WebViewなし」は旧方針で、Web画面版では採らない。

- 画面は同梱の静的ファイルだけを読む。CSPは`default-src 'none'`起点で、外部URL・CDN・フォントを使わない。
  Tauriのプラグイン（fs/shell/dialog等）は使わず、権限は`core:event`/`core:window`の既定だけ。
  秘密鍵の選択はRust側のOSダイアログで行い、画面へ返すのはパスだけ。
- `freezePrototype`は使わない（ReactとFluent UIの初期化が`Attempted to assign to readonly property`で失敗し、画面が真っ白になる。実測）。
- ホスト鍵・秘密入力の要求はRust側が控え、画面からの応答は`challenge_id`だけで照合する。対象（host/port/user/hop/世代）を
  画面から受け取らない。変更されたホスト鍵は`trust`の入力を要求し、Rust側（hashi-core）でも照合する。
- 端末入力は`session_input`を順番どおりに送る（IPCは追い越し得るため、送信中に溜まった分を1回にまとめる）。1回1MiBまで。
  端末出力は生のバイト列（`Channel`のRaw）で渡し、同じ出力をhashi-terminalのモデルへも流す（AI観測・ログ用。描画には使わない）。
- xterm.jsのOSC52（クリップボード書込み）・リンク起動の拡張は読み込まない。貼り付けは利用者の操作だけ。
- 終了はウィンドウを閉じる要求を止め、全セッションを取消→端末/接続の終了→受信タスクの回収（上限付き）の順に終えてから閉じる。
  Xvfb上でローカルPTYとSSHを開いたまま閉じ、2秒以内にプロセスが消えて子プロセスが残らないことを確認した。
- 画面の確認は、WebView2と同じChromium系でモックのバックエンドを描画して行う（`VITE_MOCK`。製品のビルドには入らない）。
  実バックエンドはXvfb + WebKitGTKで、ローカルCMDの実行、実sshdへの鍵認証、ホスト鍵確認、シェル操作を確認した。
  Windows実機（WebView2、IME、拡大表示）は未確認。
- 外観設定（テーマ・アクセント色7種・画面フォント5種・端末フォント5種/文字サイズ・端末配色5種）は固定の候補のidだけを`appearance`へ保存する。
  Rust側が候補と範囲（10〜24px）を検証し、画面から来た文字列をそのまま保存しない。フォントはその機械にあるものだけが使われる。
- AIドックは右側。サービス選択（OpenAI / Claude / 自分のPC / その他）→キー→「接続する」の1操作で、モデル一覧・応答確認・保存まで行う。
  選択・接続の規則は`hashi_ai::connect`でegui版と共有し、設定は同じ`ai_connection`。APIキーは画面へ返さず、保存済みかどうかだけを伝える。
  キーの取得ページはサービスごとの固定URLだけをRust側で開く（画面からURLを受け取らない）。接続先URLを書き換えると入力済みのキーを消す。
  共有する端末と実行モード（読むだけ/操作は確認/自動で操作）は会話欄の上で選び、送信のたびにBrokerへ設定する。共有対象があると、
  観測データを送る前に確認・編集させる。外部操作の承認はRust側の要求を画面へ流し、閉じる操作は拒否として扱う。
  端末は接続できてから台帳へ登録し、受信出力と表示中の画面を150msごとにまとめて台帳へ写す。終了時はAI処理・共有・承認待ちを先に止める。
  Xvfb + 実バックエンドで、偽のOpenAI互換サーバーを相手に、接続→再起動後の自動復元→送信前確認→承認→PTYでの実行→最終応答を確認した。
  実サービス（OpenAI/Claude）とWindows実機は未確認。
