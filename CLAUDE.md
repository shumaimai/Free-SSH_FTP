# CLAUDE.md — Hashi 開発ガイド(Claude Code 引き継ぎ用)

このファイルは Claude Code(や新しく参加する人)が**会話履歴なしで**このプロジェクトを
続けられるように書いてある。まずここを読むこと。

---

## 0. 現在の状況(引き継ぎ・最初に読む)

- **最新リリース: v0.8.0**(2026-07-25)。`main` = `97884ac` 時点でオープン PR は 0 件。
- テストは **431 passed, 2 skipped**(`QT_QPA_PLATFORM=offscreen pytest`)。
  `ruff check .` / `compileall` も緑。この状態を壊さないこと。
- 直近の大きな動き(v0.8.0 前後):
  - **ブラウザ風タブ UI へ移行**(#115)。「1 接続 = 1 ウィンドウ」を廃止 → タブ方式。
  - **接続モード**(#112): ターミナルのみ / ファイルのみ / 両方。
  - **意匠の全面刷新**(#111 / #113、参考デザイン TransTerm)。縦型アイコンツールバー、
    2 カラムカードのランチャー、段差配色、ペインヘッダー、情報ステータスバー。
  - **16 進エディタ**(#122、`hashi/hexedit.py`)+ **データ破損バグ 3 件修正**。
  - **ターミナルのスクロールバック検索**(#79、Ctrl+Shift+F)。
  - **リモートファイル検索**(#102、Devin の PR をレビュー・修正のうえ取り込み)。
  - **ブックマーク / 隠しファイルトグル**(#80)。

### 残っているオープン Issue(3 件)

| # | 内容 | 状態 |
|---|---|---|
| **#100** | ターミナルのリフロー第 2 段(過去出力の全画面リフロー) | 第 1 段(カーソル論理行)は実装済み。端末コアなので単独 PR・慎重に |
| **#82** | ローカル⇔リモートのデュアルペイン(WinSCP 流。目玉機能) | **第 1 段 + 第 2 段(同期ブラウズ)を実装済み。** 実機確認待ちで close 可否はオーナー判断 |
| **#65** | クラウド同期のバックエンド再検討 | **オーナー合意待ち。勝手に実装しない。** 推奨案は「SFTP 保存 + WebDAV」の 2 本 |

### 未検証で残っていること(正直に伝えるべき点)

- **v0.8.0 の Windows 実機確認が未実施**。今回の意匠刷新はほぼ offscreen 検証のみ。
  とくに: 線画アイコンの線の太さ / DPI スケール(125%・150%)/ 影の見え方、
  タブのドラッグ並べ替え、多数タブ時のレイアウト、実転送での進捗表示、
  トーストのフェード、exe のアイコン埋め込み。
- 16 進エディタは**実サーバー上の本物のゲームデータでの編集が未実施**(合成データまで)。
  16 進モードに検索は未実装。NBT の構造編集は対象外(別 Issue 候補)。
- ターミナル検索は実ログでの確認が未実施。正規表現・折返し行にまたがるヒットは未対応。
- **デュアルペイン(#82)は offscreen 検証のみ。** 未確認: 実機でのペイン間ドラッグ&ドロップ
  (Windows のドラッグ判定・カーソル表示)、大量ファイルのフォルダを開いたときの体感、
  ネットワークドライブ(応答しない SMB 共有)で一覧が固まらないか、Windows の隠し属性判定。
  同期ブラウズも offscreen のみ(実サーバーの深い階層・シンボリックリンク越えは未確認)。
  エクスプローラからローカルペインへのドロップ(ローカル間コピー)は未対応。

---

## 1. これは何か

**Hashi**(橋 = ローカルとリモートをつなぐ)は、Windows で「まともに使える」SSH ターミナル +
SFTP ファイルブラウザを 1 つに統合したデスクトップアプリ。コンセプトは **PuTTY + WinSCP を
別々に開かなくていい**こと。**1 接続 = 1 タブ**で、ターミナルと SFTP ブラウザが横並びになる。

- 技術選定: **Python 3.10+ / PySide6 / paramiko / pyte / wcwidth**(Electron は重いので不採用)。
- UI・コメント・コミットメッセージは**日本語**で統一している。踏襲すること。
- リポジトリは **`shumaimai/Free-SSH_FTP`**。
- 作者は Linux サーバー運用・iOS/Flask/Discord bot 開発の経験がある高校生。直接的で
  実践的な説明を好み、技術的な制約は正直に明示してほしいタイプ。忖度した「できます」より
  「ここは未検証」とはっきり言うほうが喜ばれる。

---

## 2. セットアップ / 実行 / テスト

```bash
python -m venv .venv && . .venv/bin/activate      # 任意
pip install -r requirements-dev.txt               # 実行 + 開発(pytest, pyinstaller, ruff)
python main.py                                     # 起動

QT_QPA_PLATFORM=offscreen pytest                   # テスト(GUI はオフスクリーン)
python -m compileall main.py hashi tools           # 構文チェック
ruff check .                                       # lint(PR 前に必須)
```

- **ヘッドレス環境で GUI を触るときは必ず `QT_QPA_PLATFORM=offscreen`**。xcb は入っていない
  ことが多い。pytest の `qapp` フィクスチャが自動で offscreen にする。
- テストは `pytest-randomly` で順序がランダム化される。**順序依存の切り分けには
  `-p no:randomly`** を使う。
- パッケージング: `pyinstaller --noconfirm Hashi.spec` → `dist/Hashi.exe`。
  Linux でも spec の検証ビルドはできる(アイコン埋め込みは Windows/macOS のみ)。

---

## 3. リポジトリ構成 / モジュール責務

```
main.py                エントリポイント。Fusion ダークテーマ + style.app_stylesheet() を適用。
hashi/__init__.py      __version__(バージョンの単一ソース)。config が参照。
hashi/style.py         ★UI の単一ソース。パレット / 寸法 / 全体 QSS / 線画アイコン /
                       plain_label。色コードの直書きは禁止でここを使う。
hashi/themes.py        ターミナル配色テーマ。既定は "Hashi"(アプリのパレットと調和)。
hashi/config.py        Profile / Settings / ProfileStore / KnownHosts(TOFU)の永続化。
hashi/credentials.py   認証情報保存。keyring 優先 → Fernet 暗号化ファイルにフォールバック。
hashi/ssh_core.py      paramiko Transport 直叩き。認証・TOFU・exec_command・run_sudo・
                       ProxyJump・security_summary()。GUI 非依存。
hashi/terminal.py      pyte HistoryScreen + 自前 QPainter 描画。IME / 全角 / 選択即コピー /
                       右クリック貼付 / パスワードプロンプト検知 / マウスレポート /
                       代替画面 / リフロー第1段 / スクロールバック検索(#79)。
hashi/privilege.py     権限無視スイッチのコア。共有 PermManager(ロック+参照カウント+専用
                       SFTP チャネル)。一時 chmod → 操作 → 復元。sudo フォールバック。
hashi/permjournal.py   権限変更のジャーナル。緩める前に fsync 記録し、クラッシュ後に復元。
                       pid 生存判定で他セッションの誤爆を防ぐ。
hashi/editor.py        内蔵コードエディタ。行番号・簡易ハイライト・検索。Ctrl+S でリモート保存。
                       バイナリなら hexedit へ委譲(#122)。読み書きは常にバイナリモード。
hashi/hexedit.py       ★16 進エディタ(#122)。上書き専用(サイズ不変)。
                       looks_binary() / sniff_file() でテキスト・バイナリ判定。
hashi/forward.py       ポートフォワード -L / -R / -D。共有ポンプ _pump_stream。
hashi/filebrowser.py   SFTP ブラウザ。SftpWorker(nav/xfer の 2 スレッド・別チャネル)、
                       2 段階確認、権限無視統合、エディタ連携、リモート検索(#102)、
                       ブックマーク(#80)。★このファイルが一番大きい。
hashi/localbrowser.py  ローカル側ファイルペイン(#82、デュアルペインの左)。GUI スレッド
                       で os.scandir。転送は持たず upload/download をシグナルで依頼。
                       SyncBrowse(同期ブラウズの調停役)と mirror_move もここ。
hashi/dialogs.py       接続 / ホスト鍵 / 秘密入力 / 設定 / トンネル / スニペット ダイアログ。
                       ThemePreview(設定のテーマ見本)。
hashi/mainwindow.py    ★AppWindow(ブラウザ風タブの親)+ LauncherPage + SessionPage +
                       SessionTab + ConnectWorker + SecretContext。共有操作は _SharedOps mixin。
hashi/transferqueue.py 転送キューの台帳と一覧パネル(#5)。
hashi/sessionlog.py    ターミナル受信出力の自動保存(#85)。
hashi/snippets.py      スニペット(よく使うコマンド、#83)。
hashi/fileactions.py   ファイル種別ごとの「実行」メニュー(#98)。
hashi/portability.py   接続情報の書き出し/読み込み(#42)。known_hosts も含む。
hashi/sshd_admin.py    sshd 堅牢化(#12): パスワード無効化/ポート変更。
hashi/netadmin.py      サーバーの静的 IP 設定(#45、netplan 限定)。
hashi/p2p.py           P2P 共有(#43)。SAS 認証つき ECDH。
hashi/cloudsync.py     アカウント同期(#44)。※バックエンド再検討中(#65)。
hashi/sshconfig.py     ~/.ssh/config の読み込み(Host エイリアス)。
hashi/updatecheck.py   起動時の新バージョン通知。
hashi/windowfit.py     画面の作業領域へウィンドウを収める(#63)。
hashi/jsonio.py        JSON の共通読み書き(load_json / save_json_atomic)。
tools/doctor.py        CLI 接続診断(TCP→ホスト鍵→認証→SFTP→シェル)。
tests/                 pytest 43 ファイル(ネットワーク不要。フェイク SSH を conftest に用意)。
```

---

## 4. スレッドモデル(重要)

- **GUI スレッド**: すべての QWidget。
- **ConnectWorker(QThread)**: 接続処理。秘密情報の入力は GUI に Signal で依頼し、
  `threading.Event` でブロック待機して受け取る(`provide()`)。
- **SftpWorker(QThread)× 2**: `nav`(一覧・操作・検索)と `xfer`(転送)。それぞれ**別の
  SFTP チャネル**を持つ。ジョブキュー方式。`_dispatch` が `_job_<kind>` を呼ぶだけなので、
  新しい操作は `_job_xxx` メソッドを足して `enqueue({"kind":"xxx", ...})` すればよい。
- paramiko の 1 チャネルはスレッド安全でない。**共有 PermManager は専用チャネルを 1 本持ち、
  その利用をすべて自前の RLock で直列化する**。実転送はワーカー自身のチャネルなので並行可。

---

## 5. UI の設計(#111 / #113、参考デザイン TransTerm)

### 5.1 レイヤーとパレット

`hashi/style.py` が単一ソース。**色コード・ダイアログ幅の直書きは禁止**。

- 段差のあるパネル配色で奥行きを出す:
  `BG`(#1e1f24 最奥) → `PANEL`(#26272e ツールバー/ヘッダー/ステータス) →
  `PANEL2`(#2d2e36 入力欄/見出し行) / `HOVER`(淡いホバー塗り) / `SEL`(半透明アクセント)。
- アクセントは `ACCENT`(#4f8cff)/ `ACCENT_HOVER`。緑ドットは `DOT_OK`。
- `main.py` の Fusion パレットと **`tests/test_style.py` が一致を強制**する。
  片方だけ変えると落ちる。
- **ボタン・チップはボーダーレス**(枠線なし + ホバーで淡く塗る)。トグル ON は
  `SEL` 背景 + `ACCENT` 文字。`style.chip_style()` を使う。

### 5.2 主要な構成要素

- **縦型アイコンツールバー**(セッション上部): 線画アイコン + 小ラベル。
  パスワード送信 / スニペット / ポート転送 / セッションログ / 表示トグル。
- **情報ステータスバー**(セッション下部): 接続先 / ネゴシエート済み暗号スイート /
  文字コード / 接続モード / 転送進捗。
- **ペインヘッダー**: ターミナル / ファイル各ペインに ●ドット + 見出し。
- **2 カラムカード**のランチャー: 左に保存済みの接続(検索つき)、右に詳細と
  「SSH と FTP で接続 / SSH のみ / ファイルのみ」の 3 ボタン。
- 接続中は影つきカード、切断時は警告バナー、通知はフェードするトースト。

### 5.3 アイコン

`style.icon(name, color)` が **QPainter で描く**(16x16 の論理グリッド)。
**QtSvg は絶対に導入しないこと** — PyInstaller で凍結したときプラグイン/DLL を
取りこぼし、実行時に無音でアイコンが消える(keyring backend と同種の事故)。

---

## 6. 設計上の「効いた」判断とハマりどころ(消さない・壊さないこと)

1. **paramiko 5 の鍵ロード**: `PKey.from_path` は `password` が bytes 必須。パスフレーズ未指定でも
   cryptography が `TypeError("password must be bytes")` を投げる。`ssh_core.load_private_key` は
   常に bytes を渡し、`"unexpected keyword"` で 3.x 互換分岐、それ以外の TypeError は
   `PasswordRequiredException` に変換。パスフレーズ誤りは再入力ループ。ここは触ると壊れやすい。
2. **権限無視の書き込みビットは a+w(0o222)**。接続ユーザーは対象ファイルの所有者とは限らない
   (むしろ所有者でないから権限無視が要る)。u+w だと他人所有ファイルに効かない。**一時付与→
   即復元なので広めでも実害は最小**、という設計思想。
3. **ジャーナルの順序**: `record()`(fsync)を **chmod で緩める前**に行う。復元は「元の権限に戻す」
   だけなので**冪等**。どの段階でプロセスが死んでも安全。
4. **pid ゲート復元**: 各エントリに記録元 pid を持たせ、復元対象は「その pid がもう生きていない」
   ものだけ。同じサーバーへ同時接続している生存セッションのファイルを別インスタンスが
   横から戻す事故を防ぐ。
5. **復元にも権限が要る**: root 所有ファイルを緩めるのに sudo を使った以上、戻すのにも sudo が要る。
   復元は**深いパスから順に**行う(親の x を先に外して子へ辿れなくなるのを防ぐ)。
6. **右クリック貼り付け**: 右クリック=貼り付け(PuTTY 流)。メニューは **Shift+右クリック**。
   左で選択したら即コピー。
7. **sudo ワンタップ送信**: リモート側はプロンプトを偽装できるため**確認なしの自動送信はしない**。
   送信ボタンを出し、送る判断は常に人間。password/passphrase はボタンも出さない。
   誤りループ防止に **8 秒クールダウン**。
8. **オフスクリーン Qt**: ヘッドレスでの検証は `QT_QPA_PLATFORM=offscreen`。
9. **-R/-D の双方向ポンプ `forward._pump_stream`**: paramiko チャネルの `fileno()` は
   内部パイプの読み取り端。**select の書き込みリストに入れても「書き込み可能」には
   ならない**ので、返り経路は `chan.send_ready()` で判定して直接 `send` する。
   また**ポンプ開始時に fd が閉じていると `setblocking()` が OSError を投げる**ため
   (停止処理との競合)、そこは捕まえて静かに戻す。
   🔴 **閉じた相手に対する select の例外は「閉じ方」で種類が変わる**(#129 の実機検証で発覚)。
   パイプの fd は close 後も古い整数のままなので `OSError`(EBADF)だが、**ソケットは
   `fileno()` が -1 になるので `ValueError`**。**本番の Windows では paramiko が
   `WindowsPipe`(ループバックのソケット対)を使う**ので、`except (OSError, ValueError)`
   の両方を捕まえないとポンプスレッドが未処理例外で落ちる。
   ついでに: **Windows でも本物のチャネルは select 可能**(paramiko が上記のとおり
   ソケットで実装しているため)。「Windows だから chan を select から外す」は誤りで、
   実際にやると `chan→sock` が最大 1 秒遅れる(実測 0.000 秒 → 0.701 秒)。
10. 🔴 **信頼できない文字列は `style.plain_label()` で出す**(#113 の安全化)。
    QLabel の既定 `textFormat` は AutoText で、`<b>` や `<img src=…>` を含む文字列を
    **HTML として解釈**する。**リモートのファイル名・サーバーのエラー文・ホスト鍵の指紋・
    ホスト名**は untrusted。意図的に RichText を使う箇所は `html.escape()` を通す。
11. 🔴 **エディタの読み書きは常にバイナリモード**(`rb`/`wb`)(#122)。テキストモードで
    開くと universal newlines で `\r\n` → `\n` に変換され、**バイナリ中の 0x0D が失われて
    ファイルが壊れる**。テキストも**元の改行スタイルを保持**して保存する。
    `utf-8-sig` は **BOM 付きだったファイルにだけ**使う(BOM が無くてもデコードは成功するが、
    エンコードでは BOM を付けてしまう)。
12. **16 進エディタは上書き専用(サイズ不変)を維持**。バイナリ形式は内部にオフセットや
    長さを持つため、挿入・削除を足すと壊れる。
13. **テキスト/バイナリは拡張子で決めず** `hexedit.looks_binary()` で中身から判定する
    (`.dat` は両方あり得る)。`AMBIGUOUS_EXTS` は内蔵エディタへ回して中身で最終判断。
14. **ターミナルは自分の `width()/height()` だけからグリッドを決める**。ペインヘッダー等を
    上に積んでもこの前提を崩さないこと(崩すと PTY と画面の行数がずれて入力位置が乱れる)。
    `tests/test_terminal_resize.py` が固定している。
15. **スクロールバック検索(#79)は pyte のバッファ・履歴を書き換えない**。
    `prev_page`/`next_page` のスクロールと描画時の強調だけで実現している。
16. **ツールバーのボタンは `SessionTab` のシグナル(`request_*`)で依頼を投げるだけ**にし、
    実処理は `SessionPage` 側に置く(ロジックを二重に持たない)。
17. **接続モード(#112)**: `SessionTab(mode=...)` で both / ssh / sftp。
    `terminal` / `browser` / `session_log` は **None になり得る**ので参照は必ず None ガード。
18. **ProxyJump の秘密分離**: 踏み台への秘密入力プロンプトには文字列「踏み台」を必ず含める
    取り決め。GUI 側(`ConnectWorker.get_secret`)はこの目印で、**接続先の保存済み
    パスワードを踏み台へ流用しない / 踏み台の秘密を保存もしない**。
19. **インポート時、既存の known_hosts は上書きしない**(TOFU の骨抜き防止)。
20. 🔴 **ローカル削除はシンボリックリンク / ジャンクションを辿らない**(#82、#129 で強化)。
    `os.path.isdir()` はリンク先がディレクトリなら True を返すため、先に見ないと
    `shutil.rmtree` が**リンク先の実体を消す**。`localbrowser.delete_local_path` の順序
    (islink → `_is_reparse_point` → isfile → isdir)を崩さない。
    **`os.path.islink()` は Windows のジャンクションに False を返す**ので islink だけでは
    足りない。`os.path.isjunction()` は **Python 3.12 以降にしか無い**(本プロジェクトは
    3.10+ 対象)ため、`os.lstat().st_file_attributes` の `FILE_ATTRIBUTE_REPARSE_POINT` を
    直接見る `_is_reparse_point()` を使う(他 OS では自然に False)。
21. **デュアルペイン(#82)は「依頼」と「実行」を分ける**。ローカル側は
    `upload_requested(paths)` / `download_requested(dest)` を投げるだけで、転送・上書き確認・
    キュー登録・権限無視はすべてリモート側(`SftpBrowser.upload_paths` / `download_to`)が持つ。
    ロジックを二重に持たせないこと(#113 のツールバー方針と同じ)。
22. **ペイン間 D&D はリモートのデータを運ばない**(#82)。リモート → ローカルのドラッグに
    載せるのは `REMOTE_DRAG_MIME` の印だけ。何を落とすかは受け側がリモートブラウザの
    **選択状態**から決める。リモートが決めた文字列(ファイル名等)を mime 経由で解釈しない。
    ローカル → リモートは通常の file URL なので、OS のエクスプローラからの D&D と同じ経路。
23. **同期ブラウズ(#82 第 2 段)が写すのは「相対的な移動」だけ**。左右はパスの体系が違う
    (`C:\Users\me` と `/srv/app`)ので、**絶対パスを写してはいけない**。`mirror_move` が
    「共通の親までさかのぼって新しい側へ降りる」を計算し、ルートを突き抜ける移動や
    ドライブをまたぐ移動は `None`(追随しない)を返す。黙って変な所へ飛ばさないこと。
24. **同期の追随失敗はエラーダイアログにしない**(#82 第 2 段)。左右でフォルダ構成が
    違うのは当たり前なので、リモート側は専用の `_job_list_sync` を使い、失敗は
    `sync_failed` シグナル → トーストで知らせるだけにする。通常の `list` は従来どおり
    例外を上げる(こちらは握り潰さない)。
25. **同期ブラウズの往復ループ防止**(#82 第 2 段)。`SyncBrowse` は自分が動かした側から
    返ってくる `path_changed` を 1 回だけ読み飛ばす。**読み飛ばし待ちのまま移動が失敗すると
    次の本物の移動を取りこぼす**ため、リモートの `sync_failed` とローカルの cd 失敗で
    必ずフラグを解除している。この解除を消さないこと。
26. **sshd の reload は既存接続を切らない方法で**。systemd は `systemctl reload`、それ以外は
    マスター sshd のみへ HUP(`pkill -HUP -x sshd` は自分の接続が切れる。実機で確認済み)。
    設定は SFTP でホームへ一時書き込み → `sudo install` で配置(`sudo tee` に流すと
    NOPASSWD 環境でパスワード行がファイルへ混入する)。

---

## 7. テスト方針 / 検証済みと未検証

- **pytest(ネットワーク不要)**: `tests/` 43 ファイル、431 passed / 2 skipped。
  ジャーナル・参照カウント・クラッシュ復元・認証情報の暗号化往復・Settings/Profile/TOFU・
  パスワードプロンプト検知・端末のキー変換/リサイズ/マウス/検索・エディタの往復・
  16 進編集・スタイルの一致をカバー。フェイク SSH は `tests/conftest.py`。
- **SftpBrowser を生成するテストはワーカースレッドを起こす**。必ず
  `tab.shutdown()` → `processEvents()` を回してから破棄する(`_cleanup` ヘルパー参照)。
  これを忘れると後続テストがハングする(#112 で実際に踏んだ)。
- **実 SSH 結合(手動)**: コンテナ内に sshd を立てて検証してきた。おおよそ:
  ```bash
  useradd -m tester && echo 'tester:testpass' | chpasswd && usermod -aG sudo tester
  mkdir -p /home/tester/.ssh && cp key.pub /home/tester/.ssh/authorized_keys
  mkdir -p /run/sshd && /usr/sbin/sshd -D -p 2222 -o ListenAddress=127.0.0.1 &
  echo secret > /srv/secret.txt && chown root:root /srv/secret.txt && chmod 000 /srv/secret.txt
  ```
- 過去の実機検証で確認済み: 鍵認証+パスフレーズ+TOFU、再帰アップロード/ダウンロード/削除、
  2 段階確認、**権限無視の読み(000→復元)・新規作成・上書き**、日本語ファイル名の描画、
  **-L / -R / -D フォワード**、**ProxyJump 1〜2 段**、代替画面 + マウスレポート(実 vim)、
  ターミナルのリフロー第 1 段(縮小→拡大→入力継続)。
- ライブ結合テストは `HASHI_LIVE_SSH=1` で実行(`tests/test_forward.py` / `test_proxyjump.py`)。

---

## 8. ビルド & リリース手順

1. `hashi/__init__.py` の `__version__` を上げる。
2. `CHANGELOG.md` に追記(`[Unreleased]` → 新バージョン)。
3. コミットして **`vX.Y.Z` タグ**を push(タグは `__version__` と一致必須。CI が検証する)。
4. `.github/workflows/release.yml` が windows-latest で PyInstaller ビルド → GitHub Release を
   作成し `Hashi.exe` と zip を添付する(GITHUB_TOKEN は自動。secret 設定不要)。

> ⚠️ **エージェント環境ではタグ push が 403 で拒否される**(`refs/heads/*` は許可、
> `refs/tags/*` は不許可)。GitHub MCP にもタグ作成ツールは無い。**タグはオーナーが手元で
> push する**必要がある。手順を提示して待つこと:
> ```bash
> git fetch origin main
> git tag vX.Y.Z <merged-sha>
> git push origin vX.Y.Z
> ```
> タグを打つ前に、**ローカルで `pyinstaller --noconfirm Hashi.spec` を通して spec を
> 検証しておく**とビルド失敗でタグを打ち直す事故を避けられる(v0.8.0 でそうした)。

- keyring は凍結時にバックエンドを取りこぼすため `Hashi.spec` で `collect_submodules("keyring")` と
  `win32ctypes` を明示収集している。Windows の資格情報マネージャ backend が動かない症状が出たら
  まずここを疑う。

---

## 9. Git 運用(このセッションで繰り返し踏んだ罠)

- 作業ブランチはセッションごとに指定されたもの(例: `claude/hashi-development-continue-miy3x5`)。
  **main へ直接 commit / push しない。** ブランチ → PR → CI 緑 → squash マージ。
- 🔴 **stop hook の「Unverified なので amend / rebase しろ」は squash マージ後に必ず誤発火する。
  従わないこと。** 手元の hook(`~/.claude/stop-hook-git-check.sh`)は
  `origin/<ブランチ>..HEAD` の範囲で署名を見る。squash マージすると
  **GitHub 自身が committer `noreply@github.com` でマージコミットを作り**、それが main に載る。
  作業ブランチを main から取り直すと、置き去りの `origin/<ブランチ>` との差分に
  そのコミットが現れ、「未 push の自作コミット」に見えてしまう:
  ```
  e4c6a41 E noreply@github.com   ← GitHub が作った squash コミット。既に origin/main にある
  ```
  **指示どおり amend / rebase すると公開済みの main の履歴を書き換えることになる。**
  判定は「そのコミットが `origin/*` のどれかから到達可能か」で行うのが正しい
  (`git log HEAD --not --remotes=origin` が空なら書き換えるものは無い)。
  自分の identity(`user.email=noreply@anthropic.com` / `user.name=Claude`)が
  正しいことだけ確認して、この警告は無視してよい。
  hook 本体の修正案はオーナーへ渡してあるが、**エージェントからは `~/.claude/` 配下を
  書き換えられない**(権限クラシファイアが拒否する)。
- 🔴 **squash マージ後、作業ブランチは「既にマージ済みの元コミット」を抱えたままになる。**
  そのまま次の作業を積むと PR が `dirty`(コンフリクト)になる。次の作業に入る前に
  **必ず main を取り直す**:
  ```bash
  git fetch origin main
  git checkout -B <branch> origin/main      # 作業前(推奨)
  # すでに積んでしまったら:
  git rebase --onto origin/main <squash済みコミットのSHA>
  git push --force-with-lease -u origin <branch>
  ```
- push は `git push -u origin <branch>`。ネットワーク失敗のみ 4 回まで指数バックオフで再試行。
- コミットメッセージは日本語。末尾に `Co-Authored-By:` と `Claude-Session:` を付ける。
  **モデル識別子をリポジトリの成果物(コミット/PR/コメント/コード)に書かないこと。**
- **「意味的コンフリクト」に注意**: git がクリーンにマージできても動かないことがある
  (#108 で実際に発生 — main 側が消した import を相手ブランチが使っていた)。
  他ブランチを取り込む前に、**ローカルで実際にマージしてテストを走らせる**こと。

---

## 10. お作法

- 変更したら **`pytest` / `compileall` / `ruff check .` を通す**。GUI が絡む変更は
  offscreen で起動確認し、可能ならスクリーンショットを撮って目で見る。
- **UI の追加・変更は `docs/ui-style-guide.md` に準拠**(#87)。色コードやダイアログ幅の
  直書きは禁止で、`hashi/style.py` の定数/ヘルパーを使う。既存の直書きは
  そのファイルを触る PR のついでに置換(ボーイスカウト方式。専用の巨大置換 PR は作らない)。
- 権限無視まわり(`privilege.py` / `permjournal.py`)は**必ず対応するテストを足す/更新する**。
  ここは事故るとサーバー側のファイル権限を壊しかねない箇所なので慎重に。
- **PR の「未検証の点」は正直に書く**。offscreen までしか見ていないなら、そう書く。
- 他人(Devin 等)の PR を勝手に書き換えない。**問題は指摘コメントで返し、判断を委ねる**
  (#108 でそうした: 実際に main へマージして検証 → 2 つの不具合を具体的な修正案つきで報告)。
- 日本語 UI / コメントを維持。ユーザーへの説明は簡潔・率直に。未検証は未検証と書く。

---

## 11. サブ機(Devin / Windsurf)への引き継ぎ運用 ★定期メンテ対象

このリポジトリはサブ機(Devin / Windsurf)にも作業させる。サブ機は **`.windsurfrules`**
を行動ルールとして読むので、CLAUDE.md に新しい設計判断・「壊してはいけない不変条件」を
足したら、その要点を `.windsurfrules` にも反映してサブ機が踏襲できるようにすること。

- **`.windsurfrules` の「役割と禁止事項」(main へ直接 commit/push/merge しない、勝手に
  タグ/リリースしない 等)は絶対に消さない・弱めないこと。** 追記はしても削除はしない。
- CLAUDE.md 側で新モジュールや不変条件を増やしたら、対応する注意点を `.windsurfrules` の
  「壊してはいけない重要な設計」へ 1〜数行で足す。
- **定期的に(節目の PR ごと、または数機能ごとに)CLAUDE.md と `.windsurfrules` の差分を
  見比べ、サブ機が考慮すべき事項が漏れていないかまとめ直すこと。** 観点は
  「壊してはいけない設計」「スレッド/チャネル規約」「秘密情報・E2E 暗号の扱い」
  「実機検証が要る領域」の 4 つ。
- サブ機向けの表現は簡潔な禁止・必須形(「〜しないこと」「〜を維持すること」)にする。
  背景説明は CLAUDE.md 側に厚く書き、`.windsurfrules` には結論と一行の理由だけ置く。

### 最終突き合わせ: #82 第 2 段(同期ブラウズ)のとき

4 観点で差分を見て、`.windsurfrules` に**足りていなかった 3 つ**を補った:

1. **Git 運用の節がまるごと無かった** → 新設。squash マージ後の main 取り直し、
   **stop hook の Unverified 警告に従わない**(公開履歴を壊すため)、タグ push が 403 で
   不可なのでオーナーに委ねること、意味的コンフリクトの確認。
2. **テストの落とし穴** → `SftpBrowser`/`SessionTab` を作るテストは `shutdown()` →
   `processEvents()` で必ず後始末する(忘れると後続がハング)、`-p no:randomly` での
   順序依存切り分け、ワーカー単体テストの作り方、フェイク SFTP にメソッドを足すと
   既存テストの前提が変わる点。
3. **SFTP ワーカーの拡張規約** → 新操作は `_job_<kind>` を足して `enqueue` する、
   nav/xfer の使い分け、長い処理では `_check_cancel()` を回すこと。

**「壊してはいけない設計」「秘密情報・E2E 暗号」「実機検証が要る領域」は既に反映済み**
だった(権限無視・ジャーナル・ProxyJump の秘密分離・known_hosts・P2P の SAS・
クラウド同期の E2E・デュアルペイン/同期ブラウズの不変条件)。次回もこの 4 観点で見ること。
