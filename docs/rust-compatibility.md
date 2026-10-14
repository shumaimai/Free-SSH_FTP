# Rust候補とPython版の互換表

基準はPython版main `692ebe6509517a1f438272d4125d271299f818ac`（1.1.2）。
Issue #170の完了条件をこの表で省略しない。候補に存在する機能と、実機で確認できた機能は別に扱う。

| 機能 | Rust候補の実装 | 残る実装・検証 |
| --- | --- | --- |
| 旧版との保存先分離・実行中マーカー | shared/exclusive OSロック、PID+開始時刻、全起動経路 | マーカー非対応旧版停止の取り込みUI確認、Darwin照会 |
| Profile/Settings/known_hosts/export | 全旧フィールド、atomic state、v1、追加のみmerge | 全設定UI、旧データ取り込み・ロールバックUI |
| SSH/AI秘密・クラウド暗号 | 分離ファイル、scrypt/Fernet、WinVaultの2種類のTargetName | OSストア/暗号化ファイルの選択UI、実Python keyring双方互換 |
| 日本語ネイティブGUI | eframe/egui、NotoSansJP、接続先フォーム、タブ | 旧UIの操作・テーマ・DPI互換、IME実機、終了中ワーカー回収 |
| SSH認証・ホスト鍵 | key/password/agent、RSA SHA-2、公開鍵blob指紋、対象付き認証応答 | 全鍵形式・暗号行列、SSH config、踏み台の鍵/password、agent forwarding |
| ProxyJump | 複数hopのdirect-tcpip、hop別ホスト鍵・agent認証 | Host alias、接続失敗時の全hop回収、実多段行列 |
| ローカル端末 | portable-pty、Windows JobObject、専用入力スレッド、bounded出力 | ConPTY Windows CI、子孫/連続終了/高負荷の検証 |
| 端末解析・描画 | alacritty、UTF-8分割、全角、代替画面、履歴、選択コピー、IME導線 | IME・絵文字/結合文字・リフロー実機、検索・URL・右クリック・マウス報告 |
| 端末安全性 | OSC52拒否、OSC8機能起動なし、入力/出力queue上限 | 全制御列fuzz、大量結合文字のメモリ上限、故障注入 |
| SFTP一覧/単一ファイル転送 | nav/xfer別、32KiB、fsync、明示overwrite、prefix照合resume、取消 | 再帰コピー、転送queue・進捗/取消/overwrite導線、symlink/名前互換 |
| リモート再帰削除 | 確認語delete、lstat、リンク追跡なし、安全な子名 | UI導線と権限/名前/失敗行列 |
| デュアルペイン・W端末 | 未実装 | 同期ブラウズ・D&D・ブックマーク・再接続・接続モード |
| 編集・ログ・snippets | snippets保存形式のみ | text/hex、BOM/改行保持、リモートatomic保存、ログ・検索 |
| 権限・サーバー設定 | 未実装 | chmod前のfsync journal、参照数、sudo、深い順復元、sshd/netplan rollback |
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
