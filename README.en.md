# Hashi — SSH / SFTP client for Windows

[日本語](README.md) | **English**

![Rust](https://github.com/shumaimai/Free-SSH_FTP/actions/workflows/rust.yml/badge.svg)
![Release](https://img.shields.io/github/v/release/shumaimai/Free-SSH_FTP?sort=semver)
![License](https://img.shields.io/badge/license-MIT-green)

Hashi (橋, "bridge") is an SSH terminal and an SFTP file browser in **one window** — no more opening PuTTY and
WinSCP side by side. Each connection is a browser-style **tab** with the terminal and the file browser together.
It is built with Rust and Tauri 2 (React / Fluent UI / xterm.js, russh, alacritty_terminal), is MIT-licensed,
and has no telemetry.

> **Status:** v2.x is a ground-up rewrite in Rust + Tauri. It is young: I have no Windows machine of my own,
> so testing relies on CI and bug reports from users. Please read [Known limitations](#known-limitations).
> The UI is available in **English and Japanese** (Settings → Language; "Automatic" follows your system language).
> English text has only been checked in a browser mock so far, not on a real Windows build, and some strings are still
> Japanese-only: see [Known limitations](#known-limitations).

## Download

Get `Hashi-vX.Y.Z-windows-x64.zip` from [Releases](https://github.com/shumaimai/Free-SSH_FTP/releases),
extract it, and run `Hashi.exe`. There is no installer.

- Windows 10 / 11 (64-bit). The WebView2 runtime is required (normally preinstalled).
- Keep `WebView2Loader.dll` and `HashiMCP-RustPreview.exe` in the same folder as `Hashi.exe`.
- **The build is not code-signed**, so SmartScreen may show a warning the first time. You can check that the zip
  was built by this repository's GitHub Actions with the attached `SHA256SUMS.txt` and the build attestation:
  see [Verifying your download](docs/verify-download.md). (These are attached from the release after v2.0.0.)
- The v1.x Python version stores its data elsewhere. "Import from the old version" on the home screen reads
  its connections, saved passwords and host keys (it never modifies the old data).

## Features

### Terminal
- SSH terminal and local CMD on xterm.js. 256 colors / truecolor, IME input, full-width characters, mouse-aware TUIs
- Scrollback search, copy-on-select, right-click paste (multi-line or control-character pastes ask for confirmation)
- Snippets (frequently used commands) and automatic session logs

### Files (SFTP)
- Dockable file list with recursive upload / download, a transfer queue, search, and two-step delete confirmation
- **Dual pane (local ⇔ remote)** with drag between panes and **synchronized browsing** (mirrors relative moves only)
- Overwriting transfers write to a temp file and **keep the original** until the new file is verified —
  a failed transfer never corrupts the original
- Text / hex editor that preserves BOM, line endings and encoding, and refuses to overwrite a file that changed
  remotely in the meantime
- **Permission assist** (opt-in per connection): temporarily loosens the mode of files you own, operates, then restores
  it. A recovery record is written before loosening, so even a crash can be undone on the next start

### Connections
- Public keys (Ed25519 / ECDSA / RSA, passphrases), SSH agent, passwords
- Host-key verification (a changed key requires typing `trust`), `~/.ssh/config` host aliases, multi-hop ProxyJump,
  agent forwarding
- Port forwarding `-L` / `-R` / `-D` (SOCKS5), SSH key generation and `authorized_keys` registration
- Saved passwords live in the Windows Credential Manager

### AI assistant and MCP
- **AI assistant**: register OpenAI / Anthropic / Command Code / compatible APIs in "AI model management" and pick one
  in the right-hand dock. You can attach images and text files. API keys are kept in a dedicated encrypted file.
- **Sign in with ChatGPT**: use ChatGPT-plan models without an API key (you approve in the browser;
  your password is never typed into Hashi).
- When the AI may act on a terminal you choose *read only*, *confirm each action*, or *auto*.
  Anything sent to the model is shown to you for review and editing first.
- **Local MCP server**: register Hashi with Claude Desktop / Claude Code in one click and let them operate your
  terminals — every action goes through the same approval step.

### Appearance
- Themes, accent color and fonts; dual-pane layout, panel widths and hidden-file visibility are remembered.

## Privacy

Hashi talks only to the SSH/SFTP servers you configure and the AI services you register. There is no telemetry.
The one exception is a startup update check against the GitHub Releases list, which you can turn off in
Settings → General. Details: [privacy section](docs/code-signing-policy.md#privacy-policy--プライバシー).

## Known limitations

- **Little testing on real Windows machines.** Development is verified by CI (Windows build and tests) and on Linux;
  IME behavior, high-DPI scaling and real drag & drop are unverified. Please report problems in
  [Issues](https://github.com/shumaimai/Free-SSH_FTP/issues).
- English UI: the OS-native exit-warning dialog, already-open tab titles and on-screen notices (until recreated) stay in the
  language they were created in, and documentation other than this README, SECURITY and CONTRIBUTING is Japanese.
- Launching the official Claude Code CLI inside Hashi is not ported (use the local MCP integration instead).
- Sign in with ChatGPT has been tested against a fake auth server only, not a real ChatGPT account yet.
- P2P sharing, cloud sync, sshd / netplan configuration and sudo-based permission assist from the old Python
  version are not ported.
- See the [compatibility table](docs/rust-compatibility.md) (Japanese) for per-feature status.

## Development

```bash
# UI (React)
cd rust/crates/hashi-desktop-tauri/frontend
npm ci && npm run build
npm run dev                     # preview the UI in a browser with a mocked backend

# Rust
cd rust
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo run --locked --bin Hashi
```

Use the same Rust toolchain as CI (`.github/workflows/rust.yml`). On Linux, Tauri needs WebKitGTK development
packages (see the `apt` line in `rust.yml`). Design decisions and invariants that must not be broken are in
[CLAUDE.md](CLAUDE.md) (Japanese); contributing is in [CONTRIBUTING.md](CONTRIBUTING.md); report vulnerabilities
via [SECURITY.md](SECURITY.md). Issues and pull requests in English are welcome.

## Layout

```
rust/                         Cargo workspace (the v2 app)
  crates/hashi-desktop-tauri  Tauri app (Hashi.exe); frontend/ is the React UI
  crates/hashi-core           SSH / SFTP / transfers / permission assist / operation broker (GUI-independent)
  crates/hashi-terminal       terminal model
  crates/hashi-storage        settings, host keys, secret storage, import from the old version
  crates/hashi-ai             AI service clients and tools
  crates/hashi-mcp            local MCP helper (HashiMCP-RustPreview.exe)
hashi/, main.py               the old Python version (v1.x; no longer released, code kept)
tools/                        release artifact, release notes, diagnostics
docs/                         migration notes, compatibility table, download verification, distribution
```

## Acknowledgements

Development was assisted by ChatGPT (OpenAI) and Claude Code (Anthropic). This records AI assistance only; it does
not imply that either company endorses the project or is responsible for the code.
