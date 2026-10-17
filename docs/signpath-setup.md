# SignPath Foundation でのコード署名の始め方(オーナー向け手順)

Hashi.exe に無料でコード署名するための手順。リポジトリ側の準備(署名ポリシー・CI の署名手順・
Artifact Configuration)は済んでいる。**オーナーが手でやること**だけをまとめる。

> SignPath の画面や申し込みフォームは変わることがある。この手順は 2026-10 時点で公開されていた
> 条件(<https://signpath.org/terms>)と GitHub 連携の文書(<https://docs.signpath.io/trusted-build-systems/github>)
> をもとに書いた。食い違ったら公式を優先すること。

## 0. 前提(満たしているか確認)

| 条件 | Hashi の状態 |
|---|---|
| OSI 承認ライセンス、商用デュアルライセンスなし | MIT(`LICENSE`) |
| 公開リポジトリ・無料配布・実際にリリース済み | v2.0.0 を GitHub Releases で公開済み |
| 自分のソースからビルドしたものだけ署名 | `Hashi.exe` と MCP ヘルパーだけ。`WebView2Loader.dll` は署名しない |
| 自動化された再現可能なビルド | `release.yml`(GitHub ホストの windows-latest、`--locked` / `npm ci`) |
| リリースごとの手動承認 | SignPath 側で毎回オーナーが承認する設定にする |
| 「Code signing policy」の表示 | README とリリースノートから `docs/code-signing-policy.md` へリンク済み |
| プライバシーの説明・無効化手段 | ポリシーに記載。更新確認は設定で止められる |
| 役割(Authors / Reviewers / Approvers)の明示 | ポリシーに記載(全部オーナー) |
| GitHub と SignPath の二段階認証 | **オーナーが設定する**(GitHub は Settings → Password and authentication) |

## 1. 申し込む

<https://signpath.org/apply.html> から申し込む。フォームの項目はおおよそ次のとおり(英語で書く)。
返信はメールで来るので、受け取れるアドレスで申し込む。

- **Project / Repository URL**: `https://github.com/shumaimai/Free-SSH_FTP`
- **License**: MIT
- **Download / Release URL**: `https://github.com/shumaimai/Free-SSH_FTP/releases`
- **Code signing policy URL**: `https://github.com/shumaimai/Free-SSH_FTP/blob/main/docs/code-signing-policy.md`
- **Description**(例。必要に応じて直す):

  > Hashi is a free and open-source SSH terminal and SFTP file manager for Windows, written in Rust with
  > Tauri (React + xterm.js UI). It combines a terminal and a file browser in one tabbed window
  > (an alternative to using PuTTY and WinSCP separately), with dual-pane local/remote browsing, port
  > forwarding, a text/hex editor, and an optional AI assistant that only talks to services the user
  > configures. Releases are built from source by GitHub Actions on GitHub-hosted runners and published
  > as a zip containing `Hashi.exe` and a small console helper `HashiMCP-RustPreview.exe`; both are built
  > from this repository and are the files we want to sign. The project is maintained by one developer.

審査で質問が来たら、そのまま転送してくれれば返答案を書く。

## 2. 承認後に SignPath でやること

SignPath から届く案内に従う。そのうえで次の4点を設定する。

1. **Trusted Build System**: 組織の設定で「GitHub.com」を追加し、Hashi のプロジェクトに結び付ける。
2. **Artifact Configuration**: リポジトリの `.signpath/artifact-configuration.xml` の中身を貼る。
   (zip の中の `Hashi.exe` と `HashiMCP-RustPreview.exe` を署名し、製品名 `Hashi` と版を照合する。)
3. **Signing Policy**: release 用のポリシーで、**承認者に自分を指定**する(リリースごとに手動承認)。
4. **API トークン**: 署名要求を出せる権限(submitter)を持つユーザーのトークンを作る。

## 3. GitHub に値を登録する

リポジトリの Settings → Secrets and variables → Actions で登録する。

| 種類 | 名前 | 値 |
|---|---|---|
| Secret | `SIGNPATH_API_TOKEN` | 手順 2-4 の API トークン |
| Variable | `SIGNPATH_ORGANIZATION_ID` | SignPath の組織 ID |
| Variable | `SIGNPATH_PROJECT_SLUG` | プロジェクトのスラッグ |
| Variable | `SIGNPATH_POLICY_SLUG` | 署名ポリシーのスラッグ |

`SIGNPATH_API_TOKEN` が無いあいだ、`release.yml` は署名の手順を飛ばして今までどおり未署名で配布する。

## 4. 署名つきでリリースする

いつもどおり `vX.Y.Z` タグを push する。ビルドが終わると SignPath に署名要求が届くので、
**SignPath の画面で承認する**(60 分以内。過ぎると CI が失敗する。そのときはタグを打ち直せばよい)。
承認後、CI が署名を確認(`Get-AuthenticodeSignature` が Valid)してから zip を作って公開する。

## 5. 署名が始まったら直すところ

- `docs/code-signing-policy.md` 冒頭の「Status: 申し込み準備中」の注記を消す。
- `tools/rust_preview_readme.txt` の「署名が無いため〜」の行を直す。
- 署名しても SmartScreen の警告はすぐには消えない。ダウンロードが増えて評判が貯まると出なくなる。
