Hashi v2 (Rust版・Tauri 2) — SSH端末 + SFTPファイル管理

起動: Hashi.exe をダブルクリック。
  ・同じフォルダの WebView2Loader.dll（Microsoft WebView2 SDKのローダー）が必要です。exeと離さないでください。
  ・WebView2 ランタイム（Windows 10/11 にはほぼ標準で入っています）が必要です。
  ・コード署名はまだ無いため、初回は SmartScreen / Defender の確認で時間がかかることがあります。
    zip が公式のビルドか確かめる手順: https://github.com/shumaimai/Free-SSH_FTP/blob/main/docs/verify-download.md
  ・保存先は旧Python版(v1.x)とは別です。旧版の設定は「旧版から取り込む」で読み込めます(旧版の設定は書き換えません)。
  ・外部連携(MCP)を使う場合は HashiMCP-RustPreview.exe を同じフォルダに置いたままにしてください。
    Hashiの「外部連携(MCP)」で、Claude Desktop / Claude Codeへ最初の1回だけ登録できます。
    登録の前にClaude Desktopを完全に終了してください（起動中だと設定が上書きされます）。

通信について:
  ・Hashi が通信するのは、利用者が設定した SSH/SFTP サーバーと、利用者が登録した AI サービスだけです。
    テレメトリー(利用状況の送信)はありません。
  ・例外として、起動時に GitHub(api.github.com)へ新しい版があるかを問い合わせます。
    「設定 → 全般 → 起動時に新しい版を確認する」で止められます。
  ・詳しくは https://github.com/shumaimai/Free-SSH_FTP/blob/main/docs/code-signing-policy.md
