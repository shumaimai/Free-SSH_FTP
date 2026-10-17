Hashi Preview (Rust版・Web画面 / Tauri 2) — 動作確認用ビルド

起動: Hashi-Preview.exe をダブルクリック。
  ・同じフォルダの WebView2Loader.dll（Microsoft WebView2 SDKのローダー）が必要です。exeと離さないでください。
  ・WebView2 ランタイム（Windows 10/11 にはほぼ標準で入っています）が必要です。
  ・署名が無いため、初回は SmartScreen / Defender の確認で時間がかかることがあります。
  ・保存先は旧Python版とは別（Hashi-RustPreview）です。旧版の設定は読み込みも上書きもしません。
  ・外部連携(MCP)を使う場合は HashiMCP-RustPreview.exe を同じフォルダに置いたままにしてください。

未確認: Windows実機での動作（IME・拡大表示・ドラッグ・WebView2）。気づいた点を教えてください。
