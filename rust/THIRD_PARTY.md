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
