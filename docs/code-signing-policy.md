# Code signing policy / コード署名ポリシー

Free code signing provided by [SignPath.io](https://about.signpath.io/), certificate by [SignPath Foundation](https://signpath.org/).

> Status: the application to SignPath Foundation was **not accepted** (2026-10; the project is new and has
> few users so far). Releases are published **unsigned**. This page is kept for a future application, and the
> privacy section below applies today. To check a download without a signature, see
> [verify-download.md](verify-download.md).
> 状態: SignPath Foundation への申し込みは**承認されなかった**(2026-10。公開して間もなく、利用者がまだ少ないため)。
> リリースは**未署名**。このページは再申し込みのために残し、プライバシーの節は現在も有効。
> 署名が無い配布物の確認方法は [verify-download.md](verify-download.md)。

## What is signed / 署名するもの

Only binaries built from this repository's own source code are signed:

| File | Description |
|---|---|
| `Hashi.exe` | The desktop application (Rust + Tauri, `rust/crates/hashi-desktop-tauri`) |
| `HashiMCP-RustPreview.exe` | The local MCP helper that connects Claude Desktop / Claude Code to a running Hashi (`rust/crates/hashi-mcp`) |

Third-party files shipped in the same zip (`WebView2Loader.dll` from Microsoft) are included as-is and are
not signed by this project.

## How releases are built / ビルドと署名の流れ

1. The maintainer pushes a `vX.Y.Z` tag. The tag must match `workspace.package.version` in `rust/Cargo.toml`.
2. [`.github/workflows/release.yml`](../.github/workflows/release.yml) builds the binaries from source on a
   GitHub-hosted `windows-latest` runner (`cargo build --locked --release`, `npm ci` with a lockfile).
3. The unsigned binaries are uploaded as a GitHub Actions artifact and submitted to SignPath.
   The artifact configuration is kept in [`.signpath/artifact-configuration.xml`](../.signpath/artifact-configuration.xml)
   and restricts the product name (`Hashi`) and product version (the release version).
4. **Every signing request is manually approved by the maintainer** in SignPath before it is signed.
5. The workflow verifies the Authenticode signatures, then packages the signed binaries with a
   `build-info.json` (source commit, SHA-256 of each binary, dependency licenses) and publishes the zip on
   [GitHub Releases](https://github.com/shumaimai/Free-SSH_FTP/releases).

Build scripts and CI configuration are part of this repository and are reviewed like any other source change.

## Team roles / 役割

Hashi is maintained by a single person.

| Role | Members |
|---|---|
| Authors (may commit) | [@shumaimai](https://github.com/shumaimai) |
| Reviewers (review changes from others) | [@shumaimai](https://github.com/shumaimai) |
| Approvers (approve each signing request) | [@shumaimai](https://github.com/shumaimai) |

Changes are made through pull requests. Some changes are prepared with AI coding assistants
(e.g. Claude Code) working under the maintainer's direction; the maintainer is responsible for every change
merged into `main`, and outside contributions are reviewed before merging (see [CONTRIBUTING.md](../CONTRIBUTING.md)).
The maintainer uses multi-factor authentication for GitHub and SignPath.

## Privacy policy / プライバシー

Hashi does not collect telemetry and does not send information to the project or to any third party.
It only connects to systems the user explicitly configures:

- **SSH / SFTP servers** that the user adds as connections (including jump hosts and port forwards).
- **AI services** that the user registers in "AI models" (OpenAI, Anthropic, Command Code, or a URL the user enters).
  When the user chooses "Sign in with ChatGPT", Hashi opens the system browser to `auth.openai.com` and talks to
  `auth.openai.com` / `api.openai.com` for that account; tokens are stored only in the encrypted file on the PC.
  Conversation text and any terminal output the user chooses to share are sent only to the selected service,
  after the user reviews them.
- **Local MCP**: the helper talks to Hashi only over the loopback interface (`127.0.0.1`) of the same PC.

One exception is enabled by default:

- **Update check**: at startup Hashi asks GitHub (`api.github.com`) for the list of releases of this
  repository to tell the user when a newer version exists. The request contains only standard HTTP
  information (IP address and the User-Agent `Hashi-Rust/<version>`). It can be turned off in
  **Settings → General → "起動時に新しい版を確認する"**. With it turned off, Hashi will not transfer any
  information to other networked systems unless specifically requested by the user.

Hashi has no installer: it is a zip you extract, and it does not change system settings.
Settings and saved secrets stay on the user's PC: SSH passwords are stored in Windows Credential Manager
and AI API keys in an encrypted file in the settings directory. Both can be deleted from within the app.

### 日本語の要約

- テレメトリーは送らない。プロジェクトや第三者へ情報を送らない。
- 通信するのは、利用者が設定した SSH/SFTP サーバー、利用者が登録した AI サービス(送る内容は送信前に確認できる。
  ChatGPT サインインを選んだときは auth.openai.com / api.openai.com)、
  同じ PC 内の MCP(127.0.0.1)だけ。
- 例外は起動時の**更新確認**(既定でオン)。GitHub の Releases 一覧を取得するだけで、送るのは通常の通信情報
  (IP アドレス・User-Agent)のみ。**設定 → 全般 → 「起動時に新しい版を確認する」**で止められる。
- インストーラーは無い(zip を展開して使う)。システム設定は変えない。保存したパスワードは Windows 資格情報マネージャ、
  AI の API キーは設定フォルダの暗号化ファイルに置く。どちらもアプリから削除できる。
