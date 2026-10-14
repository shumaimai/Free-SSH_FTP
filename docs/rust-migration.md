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
全Rust版のGUI、SSH/SFTP、追加機能は後続の実装対象。

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
pytest tests/test_runtime_guard.py
```

Windows CIは.exeを指定して同じ双方向試験を実行する。通常のPython CIも継続する。
GUIの変更は起動失敗時の日本語エラー表示だけで、既存の接続/エディタ処理は
ガードの内側に置く。新しいデータスキーマへ自動変換したり、旧資格情報を上書きしない。

## 残る検証

Windows CIのプロセス間ロック試験と、Windows実機のIME/ConPTY/資格情報試験は別物。
実機は現在接続されていない。実装とCIを先行するが、未確認の項目を移行完了とは扱わない。
GUI/端末/SSH候補の選定結果、機能互換表、旧形式の両方向fixture、起動/メモリ/配布サイズの
比較もPhase 0以降で記録する。
