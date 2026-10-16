# Rust依存ライブラリ

使う版と有効featuresはCargo.lockと各Cargo.tomlで固定する。配布時は
`cargo metadata --locked` で全依存のライセンスを収集し、配布物へ同梱する。

Fernet互換の実装はMozillaのfernet 0.2.2（MPL-2.0）を使用する。
default featuresを無効にしてrustcrypto backendを選び、OpenSSLランタイムを要求しない。
Hashi側のコードはMITのままで、依存ソースのライセンスを維持する。

- [fernet 0.2.2ソース配布](https://crates.io/api/v1/crates/fernet/0.2.2/download)
- [MPL-2.0本文](licenses/fernet-0.2.2-MPL-2.0.txt)

診断/fixture用の `hashi-migration-check` と `hashi-storage-contract` は開発用であり、
最終利用者向けHashi/HashiMCPの配布対象とは分ける。

ネイティブ候補の日本語fallbackフォントはGoogle Fonts配布のNotoSansJPを埋め込む。
SIL Open Font License 1.1の本文を `crates/hashi-desktop-native/assets/OFL.txt` に保持する。

- source: https://github.com/google/fonts/blob/main/ofl/notosansjp/NotoSansJP%5Bwght%5D.ttf
- asset SHA-256: `c2f3b4d463500a2ddcd3849cded1fceeb9fd6d1c32e6cbecd568453ba50fc68f`

SSH候補はrussh 0.64.1（ring/RSA/flate2）、SFTPはrussh-sftp 3.0.1、
端末はalacritty_terminal 0.26.0、PTYはportable-pty 0.9.0、GUIはeframe 0.36.2。
egui版のネイティブ候補はPython/Qt/WebViewのランタイムを同梱しない。

Web画面版（`crates/hashi-desktop-tauri`）はTauri 2とWindowsのWebView2（OS付属のEdge描画部品）を使う。
画面はReact 19、Fluent UI（`@fluentui/react-components`）、xterm.js（`@xterm/xterm`）で、
`frontend/package-lock.json`で固定する。配布時は`tools/rust_artifact.py`が実行時のnpm依存だけの
ライセンス（本文・package.json）を`licenses/npm/`へ同梱する。開発用依存は同梱しない。

GUIの既定フォントはepaint_default_fonts/fontsのUbuntu Font License、SIL OFL、Hackとemoji-iconの各noticeもCI成果物へ同梱する。

更新確認のHTTPSはureq 3.4.2（rustls、rustls-platform-verifierでOSの証明書ストア、Windowsはsystem proxy）を使う。

AI観測データの秘密除去はregex 1（MIT/Apache-2.0）を使う。MCPのstdioヘルパー`HashiMCP-RustPreview`は
GUIと同じ依存ライセンスを同梱する配布対象で、診断用バイナリとは分ける。
