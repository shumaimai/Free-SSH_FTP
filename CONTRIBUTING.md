# コントリビュートガイド

Hashi への参加ありがとうございます。小さな修正から大歓迎です。
まず [README.md](README.md) で全体像を、[CLAUDE.md](CLAUDE.md) で設計判断・ハマりどころ・
壊してはいけない不変条件を読んでください(CLAUDE.md は AI/人間どちらの新規参加者向けにも書かれた開発ガイドです)。

v2.0.0 から配布物は **Rust + Tauri 版**(`rust/`)です。新しい変更は基本的にこちらへ入れてください。
旧 Python 版(`hashi/`、v1.x)はリリースしませんが、コードとテストは残しています。

## セットアップ

```bash
git clone https://github.com/shumaimai/Free-SSH_FTP.git
cd Free-SSH_FTP

# 画面(React / Fluent UI / xterm.js)
cd rust/crates/hashi-desktop-tauri/frontend
npm ci                 # package-lock.json で固定。install スクリプトは実行しない設定
npm run build          # Tauri が読む dist/ を作る
npm run dev            # ブラウザだけで画面を確認(バックエンドはモック)
cd ../../..

# Rust(ツールチェーンの版は .github/workflows/rust.yml に合わせる)
cargo run --locked --bin Hashi
```

Linux で Tauri を動かすには WebKitGTK などの開発パッケージが必要です
(`.github/workflows/rust.yml` の `apt-get install` の行を参照)。

## テストと確認(PR の前に必ず)

```bash
cd rust
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cd crates/hashi-desktop-tauri/frontend && npm run build   # 型検査を含む
```

旧 Python 版や `tools/` を触った場合は、こちらも通してください。

```bash
QT_QPA_PLATFORM=offscreen PYTHON_KEYRING_BACKEND=keyring.backends.fail.Keyring pytest
python -m compileall main.py hashi tools
ruff check .
```

- 画面の変更は、可能なら実際に起動して確認してください。Windows 実機で確認できた場合は PR に書いてもらえると助かります
  (メンテナーの手元に Windows 実機が無く、実機での確認が一番足りていません)。
- 配布用の zip は `tools/rust_artifact.py` で作ります。手でまとめないでください。

## 何から手を付けるか

- [Issues](https://github.com/shumaimai/Free-SSH_FTP/issues) を見てください。`good first issue` ラベルが入りやすいです。
- Windows 実機での動作報告(IME・高 DPI・ドラッグ&ドロップなど)だけでも大きな助けになります。
- 大きめの変更(アーキテクチャ・保存形式・通信先に触るもの)は、着手前に Issue で方針を相談してください。

## お作法

- **コード内のコメント・コミットメッセージは日本語**で統一しています。踏襲してください。
- **画面の文言は日本語を原文にして `t('…')` で書き、英訳を `frontend/src/i18n/en/` に足します**(日本語の直書きは `npm run build` が止めます)。
  Rust が日本語で返すエラー文などは `frontend/src/i18n/rust/*.json` の台帳に英訳を足してください(`python tools/rust_messages.py check` が検査します。
  画面に出ない文は値を `null` にします)。
- 安全確認(ホスト鍵・権限補助・AI へ送る内容・外部操作の承認)を 1 クリックで済ませる変更はしないでください。
- 権限補助・転送の確定処理・復旧記録(`rust/crates/hashi-core` の権限 / 転送まわり)を触るときは、
  **必ず対応するテストを追加・更新**してください。事故るとサーバー側のファイルを壊しかねない箇所です。
- 新しい通信先を増やす変更は、[コード署名ポリシー](docs/code-signing-policy.md) のプライバシーの節も更新してください。
- 未検証のことは未検証と正直に書く(PR 説明・コード内コメントとも)。
- 1 PR = 1 テーマ。無関係なリファクタを混ぜない。

## PR の流れ

1. フォークしてブランチを切る(例: `fix/terminal-ime-xxx`)。
2. 変更 + テスト。上記のコマンドを通す。
3. PR を作成。テンプレートに沿って「何を・なぜ・どう確認したか」を書く。
4. CI(GitHub Actions)が通ることを確認。メンテナーがレビューしてからマージします。

AI のコーディング支援(Claude Code など)を使った変更も歓迎します。PR の内容に責任を持てる範囲で使ってください。

## バグ報告・機能要望

[Issue テンプレート](https://github.com/shumaimai/Free-SSH_FTP/issues/new/choose) からどうぞ。
再現手順・環境(Windows の版・Hashi の版・接続先サーバーの種類)があると助かります。

## セキュリティ

脆弱性(認証情報の漏えい、権限復元の不備など)は公開 Issue ではなく
[SECURITY.md](SECURITY.md) の手順で報告してください。
