# Hashi — SSH / SFTP クライアント

![Rust](https://github.com/shumaimai/Free-SSH_FTP/actions/workflows/rust.yml/badge.svg)
![Release](https://img.shields.io/github/v/release/shumaimai/Free-SSH_FTP?sort=semver)
![License](https://img.shields.io/badge/license-MIT-green)

橋 (bridge)。ローカルとリモートをつなぐ、Windows でまともに使える SSH ターミナル + SFTP ファイルブラウザ。
PuTTY + WinSCP を別々に開かなくていいのがコンセプト。ホームで接続先を選ぶと、
**ブラウザ風のタブ**(1 接続 = 1 タブ)でターミナルとファイル操作が並ぶ。

v2.0.0 から Rust + Tauri(画面は React / Fluent UI / xterm.js)で作り直した版を配布しています。
旧 Python 版(v1.x)の使い方は [下のほう](#旧python版v1x) にまとめています。

## ダウンロード

[Releases](https://github.com/shumaimai/Free-SSH_FTP/releases) から `Hashi-vX.Y.Z-windows-x64.zip` を取得し、
展開して `Hashi.exe` を起動します。インストーラーはありません。

- Windows 10 / 11(64bit)。WebView2 ランタイムが必要です(通常は標準で入っています)。
- `Hashi.exe` と同じフォルダの `WebView2Loader.dll`・`HashiMCP-RustPreview.exe` は離さないでください。
- 現在の配布物にはコード署名が無いため、初回は SmartScreen の確認が出ることがあります。
  署名は [SignPath Foundation](https://signpath.org/) へ申し込む準備をしています。
- 旧 Python 版(v1.x)とは保存先が別です。ホームの「旧版から取り込む」で接続先・保存済みパスワード・
  ホスト鍵を読み込めます(旧版の設定は書き換えません)。

**[Code signing policy / コード署名ポリシー](docs/code-signing-policy.md)**(通信とプライバシーの説明も含む)

## 主な機能

### ターミナル
- xterm.js による SSH 端末とローカル CMD。256 色 / truecolor、日本語 IME、全角文字、マウス対応の TUI
- スクロールバック検索、選択コピー、右クリック貼り付け(複数行・制御文字を含む貼り付けは確認する)
- スニペット(よく使うコマンド)、セッションログの自動保存

### ファイル(SFTP)
- 右ドックのファイル一覧。再帰アップロード / ダウンロード、転送キュー、検索、削除(2 段階確認)
- **2 ペイン(ローカル ⇔ リモート)**とペイン間のドラッグ、**同期ブラウズ**(相対的な移動だけを写す)
- 上書き転送は**原本を残したまま**一時ファイルへ書き、確認してから差し替える(途中で失敗しても原本は壊れない)
- テキスト / 16 進エディタ。BOM・改行・文字コードを保ったまま保存し、外部で変わっていたら上書きしない
- **権限補助**(旧「権限無視スイッチ」): 接続ごとに有効にした場合だけ、自分が所有するファイルの権限を
  一時的に緩めて操作し、直後に戻す。緩める前に復旧記録を残すので、強制終了しても次回戻せる

### 接続
- 公開鍵(Ed25519 / ECDSA / RSA、パスフレーズ対応)、SSH エージェント、パスワード
- ホスト鍵の確認(初回確認・変更時は `trust` 入力)、`~/.ssh/config` の Host エイリアス、ProxyJump(多段)、agent 転送
- ポート転送 -L / -R / -D(SOCKS5)、SSH 鍵の生成と `authorized_keys` への登録
- 保存したパスワードは Windows 資格情報マネージャに置く

### AI と外部連携
- **AI 相談**: OpenAI / Anthropic / Command Code / 互換 API を「AI モデルの管理」で複数登録し、右ドックで選んで使う。
  画像やテキストファイルを添付できる。API キーは AI 専用の暗号化ファイルに保存する
- **ChatGPT サインイン**: API キーの代わりに ChatGPT のアカウントでサインインして、ChatGPT プランのモデルを使える
  (ブラウザで許可する。パスワードは Hashi に入力しない)
- AI に端末の操作を頼むときは「読むだけ / 操作ごとに確認 / 自動」を選ぶ。送る内容は送信前に確認・編集できる
- **ローカル MCP**: Claude Desktop / Claude Code からワンクリック登録して、Hashi の端末を操作できる(操作は同じ承認を通る)

### 外観
- テーマ・アクセント色・フォントを選べる。2 ペイン・パネル幅・隠しファイル表示は保存される

## 通信とプライバシー

Hashi が通信するのは、利用者が設定した SSH/SFTP サーバーと、利用者が登録した AI サービスだけです。
テレメトリーはありません。例外は起動時の更新確認(GitHub の Releases 一覧を取得)で、
「設定 → 全般 → 起動時に新しい版を確認する」で止められます。詳しくは[コード署名ポリシー](docs/code-signing-policy.md#privacy-policy--プライバシー)。

## 既知の制限

- **Windows 実機での確認がほとんどできていません。** 動作確認は CI(Windows のビルドとテスト)と
  Linux での実行が中心で、IME・高 DPI・実際のドラッグ&ドロップは未確認です。不具合は Issue で教えてください。
- 公式 Claude Code の内蔵起動(#160)は Tauri 版へは移植しません(ローカル MCP で連携できます)。
- ChatGPT サインインは、偽の認証サーバーを使った試験までで、実際の ChatGPT アカウントでの確認はまだです。
- 旧 Python 版にあった P2P 共有・クラウド同期・sshd / netplan の設定変更・sudo を使う権限補助は未移植です。
- 機能ごとの対応状況と未検証の項目は [互換表](docs/rust-compatibility.md) にまとめています。

## 開発

```bash
# 画面(React)
cd rust/crates/hashi-desktop-tauri/frontend
npm ci && npm run build        # rust/crates/hashi-desktop-tauri/frontend/dist へ出力
npm run dev                     # ブラウザだけで画面を確認(バックエンドはモック)

# Rust
cd rust
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo run --locked --bin Hashi  # 画面を build してから起動する
```

- Rust のツールチェーンは CI と同じ版(`.github/workflows/rust.yml`)を使ってください。Linux で動かすには
  WebKitGTK などの開発パッケージが必要です(`rust.yml` の apt の行を参照)。
- 配布用の zip は手でまとめず、`tools/rust_artifact.py` で作ります(中身の取り違えを検査する)。
- 詳しい設計判断・壊してはいけない不変条件は [CLAUDE.md](CLAUDE.md)、参加の手順は [CONTRIBUTING.md](CONTRIBUTING.md)。
  脆弱性の報告は [SECURITY.md](SECURITY.md) へ。

## リリース

1. `rust/Cargo.toml` の `workspace.package.version` と `rust/crates/hashi-desktop-tauri/tauri.conf.json` の `version` を上げる
2. `CHANGELOG.md` に新しい版の節を足す
3. main へ入れたあと、`vX.Y.Z` タグを push する(タグと Cargo の版が一致しないと CI が止める)

```bash
git fetch origin main
git tag vX.Y.Z origin/main
git push origin vX.Y.Z
```

`.github/workflows/release.yml` が Windows でビルドし、zip を GitHub Release に添付します。

## 構成

```
rust/                         Cargo workspace(v2 の本体)
  crates/hashi-desktop-tauri  Tauri アプリ(Hashi.exe)。frontend/ が React の画面
  crates/hashi-core           SSH / SFTP / 転送 / 権限補助 / 操作仲介(GUI 非依存)
  crates/hashi-terminal       端末モデル
  crates/hashi-storage        設定・ホスト鍵・秘密の保存、旧版からの取り込み
  crates/hashi-ai             AI サービスとの通信・ツール
  crates/hashi-mcp            ローカル MCP(HashiMCP-RustPreview.exe)
  crates/hashi-desktop-native egui 版の候補(配布しない。移行が済むまで残す)
hashi/, main.py               旧 Python 版(v1.x。リリースはしないがコードは残す)
tools/                        配布物の作成・リリースノート・診断
docs/                         移行の記録・互換表・コード署名ポリシー
```

## 旧Python版(v1.x)

最終リリースは [v1.1.2](https://github.com/shumaimai/Free-SSH_FTP/releases/tag/v1.1.2)。
以後はリリースしませんが、コードは `hashi/` に残しています。

```bash
pip install -r requirements.txt
python main.py
```

Python 3.10+。テストは `QT_QPA_PLATFORM=offscreen PYTHON_KEYRING_BACKEND=keyring.backends.fail.Keyring pytest`。
保存先は `%APPDATA%\Hashi\`。ローカル CMD・AI 相談・MCP の使い方は [docs/terminal-ai.md](docs/terminal-ai.md)。

## 開発支援への謝辞

- **ChatGPT (OpenAI)** — ターミナルUI・スクロールバック選択の修正、回帰テスト、リリース準備について開発支援を行いました（例: [#146](https://github.com/shumaimai/Free-SSH_FTP/pull/146), [#147](https://github.com/shumaimai/Free-SSH_FTP/pull/147), [#148](https://github.com/shumaimai/Free-SSH_FTP/pull/148), [#149](https://github.com/shumaimai/Free-SSH_FTP/pull/149)）。
- **Claude Code (Anthropic)** — Rust + Tauri 版への移行(#170〜#183)の実装・検証について開発支援を行いました。

AIによる支援の記録であり、Gitコミットの著者・責任者やOpenAI・Anthropicによる公式な承認を意味しません。
