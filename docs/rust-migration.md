# Rust移行の実装と検証

計画と完了条件は [Issue #170](https://github.com/shumaimai/Free-SSH_FTP/issues/170)。
最終版はPython版とは別の保存先を使う。

| 版 | Windows設定ルート | SSH / AIのサービス名 |
| --- | --- | --- |
| Python | `%APPDATA%\Hashi` | `Hashi` / `Hashi.AI` |
| Rust preview | `%APPDATA%\Hashi-RustPreview` | `Hashi.RustPreview` / `Hashi.RustPreview.AI` |
| Rust native | `%APPDATA%\Hashi-Rust` | `Hashi.Rust` / `Hashi.Rust.AI` |

## 現在の実装

`rust/` はCargo workspace。`hashi-storage` はGUIを参照せず、保存先と移行ガードを持つ。
`hashi-migration-check` はガードの診断用CLIで、SSH接続・データ取り込みは行わない。
旧Python版は通常起動とエディタ単独起動の両方で実行中マーカーを作成する。
RustネイティブGUIの候補 `Hashi-RustPreview` はeframe/egui、端末モデルは
alacritty_terminal、SSH/SFTPはrussh/russh-sftp、ローカルPTYはportable-ptyで動く。
候補の実行にPython・Qt・WebViewは必要ない。既存Pythonアプリの全機能はまだ揃っていない。
機能と実測の扱いは [互換表](rust-compatibility.md) に記録する。

保存層にはPython版の全Profileフィールドと設定の取り込み、known_hostsの追加のみの
マージ、公開鍵blobからの種別/指紋生成、版付きstate.jsonのatomic保存を追加する。
旧プロファイル/設定/信頼情報は読み取りのみで、破損時は取り込みを中止する。
SSH/AIの暗号化ファイルは別の鍵とロックを持ち、破損・鍵欠損を空データへ格下げしない。
Windowsはサービス名TargetNameと複合TargetNameの両方を読み、UserNameを照合する。
新規書き込みは新Rustサービスへ限定する。

クラウド/エクスポートv1のscrypt（n=2^14, r=8, p=1, salt16バイト）とFernetは
Python/Rust両方向の合成データで試験する。Unicodeの正規化は行わず、未来版は拒否する。
OAuth登録の移管、Qtの旧ロック形式の共有、クラウドバックエンドの変更は行わない。
Windows資格情報のテストはランダムな試験専用サービスだけを使い、実際のHashi保存情報へ
触れない。[依存ライセンス](../rust/THIRD_PARTY.md)も配布検証の対象にする。

## 実行中マーカーの契約 v1

- Windowsは `%LOCALAPPDATA%\Hashi-Migration\runtime`、Unixは
  `$XDG_RUNTIME_DIR`、なければ `$XDG_CONFIG_HOME` / `~/.config` の下に
  `Hashi-Migration/runtime` を置く。保存データのルートとは別の調停領域。
- `migration.guard` は同じファイルを保持し、削除・置換しない。Pythonが起動中に
  共有ロックを、移行処理が排他ロックを保持する。双方とも非ブロッキング取得。
  Linuxはflock、WindowsはLockFileExでオフセット0から最大範囲をロックする。
  Rust fs2 0.4.3とPython側の範囲を揃える。ガード取得失敗時は旧版を起動しない。
- `python-<instance_id>.json` はプロセスごとにatomic保存してfsyncする。
  フィールドは `schema_version`、`implementation`、`application_version`、`pid`、
  `process_start`、`instance_id`、`executable`、`data_root`。
  秘密や接続情報は含めない。正常終了は自分のJSONだけ削除する。
- `process_start` はWindowsの作成FILETIME、Linuxのboot_idと/procのstarttimeを使う。
  PID再利用とOS再起動を区別する。DarwinのPython側はpsのlstartを記録するが、
  Rust側のDarwin生存照会は未実装で、残存記録がある場合に自動移行しない。
- 強制終了後のJSONは残りうる。OSロックが解放されても、Rust側はプロセス開始時刻で
  生存を確認する。破損、未知スキーマ、未完了のtmp、照会失敗はエラーにし、勝手に削除しない。
- マーカーが無いことは、マーカー未対応の過去版の停止を証明しない。診断CLIも
  その停止を認定しない。データ移行UIには、対応版への更新または明示的な旧版停止確認が必要。
- runtimeルートは利用者本人の領域に置く。Unixは所有者を確認して0700/0600を使い、
  リンクを拒否する。Windowsのreparse pointも拒否する。Windowsの資格情報・MCP endpointの
  ACL検証は別の実装/検証項目であり、このマーカー試験で確認済みとしない。

## 開発時の確認

```sh
cd rust
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo build --locked --workspace --bins
cd ..
HASHI_RUST_GUARD_BINARY="$PWD/rust/target/debug/hashi-migration-check" \
QT_QPA_PLATFORM=offscreen PYTHON_KEYRING_BACKEND=keyring.backends.fail.Keyring \
HASHI_RUST_STORAGE_BINARY="$PWD/rust/target/debug/hashi-storage-contract" \
HASHI_RUST_SSH_BINARY="$PWD/rust/target/debug/hashi-ssh-check" \
pytest tests/test_runtime_guard.py tests/test_rust_storage_compat.py tests/test_rust_ssh_live.py
```

Windows CIは.exeを指定して同じ双方向試験を実行する。通常のPython CIも継続する。
Python側の接続/エディタ処理はガードの内側に置く。新しいデータスキーマへ自動変換したり、旧資格情報を上書きしない。

## 残る検証

Windows CIのプロセス間ロック試験と、Windows実機のIME/ConPTY/資格情報試験は別物。
実機は現在接続されていない。実装とCIを先行するが、未確認の項目を移行完了とは扱わない。
GUI/端末/SSH候補の選定結果、機能互換表、旧形式の両方向fixture、起動/メモリ/配布サイズの
比較もPhase 0以降で記録する。

## ネイティブ候補の起動

```sh
cd rust
cargo run --locked --bin Hashi-RustPreview
```

通常のLinux画面かWindowsで起動する。CIのWindows成果物はdebugビルドの候補であり、
安定版リリースや移行完了の証拠ではない。初回のRust設定は空で、旧版の自動取り込みは
まだ行わない。接続先を手入力し、鍵/パスワード/agent認証を選べる。
ホスト鍵承認は接続先・踏み台・接続世代に結び付け、終了した要求から保存しない。
SFTPの一覧と転送は別チャネル。転送は32KiBごとに処理し、ライブラリの先読みも1本に抑える。
キャンセルと古いイベントの破棄をコアに持つ。画面側の全操作導線は互換表で区別する。

Linuxでは管理下loopback OpenSSHへの実鍵認証、exec、1MiBの転送往復と再開を確認する。
仮想Xサーバーで日本語GUI表示とローカルシェルの入力/echoも確認した。
WindowsはCIのConPTY・ビルド検証を進め、実機のIME・DPI・資格情報バックエンド検証は
保留のまま記録する。russhが使うssh-key/RSA依存にはRC版が含まれるため、
全面的な暗号互換の採用判断は互換マトリクスの検証後に行う。

SSH設定は標準の`~/.ssh/config`を読み、HostName/User/Port/IdentityFile/ProxyJump/ForwardAgentを解決する。
踏み台はそれぞれのIdentityFile、agent、専用passwordを使う。接続先の秘密は使い回さない。
GUIの「ポート転送」から-L/-R/-Dを開始/停止できる。初期の待受は127.0.0.1で、
公開アドレスへの変更は利用者が選ぶ。SOCKS5は認証なしのCONNECTだけに対応する。
独立execには開始から終了まで共通の締切があり、締切/取消は専用チャネルだけを閉じる。
遠隔プロセスが確実に終了したとは返さない。

ローカルの「編集」とSFTPの「編集」で、テキストまたは固定長16進モデルを開く。
`cargo run --locked --bin Hashi-RustPreview -- --edit FILE` はローカル編集の起動経路。
テキストのBOM/UTF-16 endian/改行を保持し、未編集のファイルはそのままのバイト列で返す。
ローカルはatomic rename、リモートは明示のPOSIX rename拡張を使う。
変更前の内容が外部で変わっていれば保存を拒否し、失敗や結果不明でも編集内容を残す。
上書き専用HEXでは長さの変更を拒否する。未保存のウィンドウ/アプリ終了には確認を置く。

「ファイル・転送」からローカル/リモートの二つの一覧、再帰転送、検索、削除確認、
進捗・中止を操作できる。既存ファイルは中止/上書き/再開/スキップを明示して追加する。
標準は上書きしない。ファイル・フォルダのdrop送信は受信面と最前面layerを照合し、
入力イベントを一度だけ消費する。実OSのD&Dは未検証として互換表に残す。
