# ダウンロードの確認 / Verifying your download

Hashi の配布物にはまだコード署名がありません(SmartScreen の確認が出ることがあります)。
代わりに、**ダウンロードした zip が公式の GitHub Actions で作られたものか**を自分で確かめられます。
これは署名の代わりではなく、SmartScreen の表示も変わりません。

The release zip is not code-signed yet, so SmartScreen may warn you. You can still verify that the zip you
downloaded was built by this repository's GitHub Actions workflow. This does not replace code signing and
does not change SmartScreen's behaviour.

## 1. SHA-256

Release に添付の `SHA256SUMS.txt` と、手元の zip のハッシュを比べます。

```powershell
Get-FileHash .\Hashi-vX.Y.Z-windows-x64.zip -Algorithm SHA256
```

## 2. ビルド証明 / Build provenance

[GitHub CLI](https://cli.github.com/) で、zip が `shumaimai/Free-SSH_FTP` の release workflow で作られたことを確認します。

```powershell
gh attestation verify .\Hashi-vX.Y.Z-windows-x64.zip --repo shumaimai/Free-SSH_FTP
```

成功すると、元のコミットと workflow の実行が表示されます。失敗したら、その zip は使わないでください。
On success it prints the source commit and workflow run. If it fails, do not use that zip.

## 3. パッケージマネージャ / Package managers

winget / Scoop への登録はオーナーが提出する形です(手順は [distribution.md](distribution.md))。
登録後は `winget install shumaimai.Hashi` / `scoop install hashi` で入れられるようになります。
未登録のあいだは使えません。
