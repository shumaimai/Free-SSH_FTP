# 配布の手順(winget / Scoop)

オーナー向けの手順書。**提出はオーナーが行う**(エージェントは他リポジトリへ PR を出さない)。
2026-10 時点では**まだどちらにも提出していない**。ここのマニフェストは下書きで、**実際に `winget install` /
`scoop install` が通るかは未確認**。

## 仕組み

- `packaging/` にテンプレート(`@VERSION@` `@SHA256@` を置換する)。
- `tools/package_manifests.py X.Y.Z --zip <zip> <出力>` が提出用の一式を作る。
- release.yml はタグのたびにこれを実行し、workflow の成果物 `package-manifests-vX.Y.Z` に置く。

## winget

1. Release が公開されたら、成果物 `package-manifests-vX.Y.Z` の `winget/` を取得する。
2. [winget-pkgs](https://github.com/microsoft/winget-pkgs) をフォークし、
   `manifests/s/shumaimai/Hashi/X.Y.Z/` に3ファイルを置く。
3. `winget validate --manifest <フォルダ>` を通してから PR を出す(Windows が要る。オーナーの手元には無いので、
   [wingetcreate](https://github.com/microsoft/winget-create) か Windows の友人・CI で検証する)。
4. 2回目以降は `wingetcreate update shumaimai.Hashi --version X.Y.Z --urls <zipのURL> --submit` で済む。

未確認の点:
- zip(portable)の `Hashi.exe` はシンボリックリンク経由で起動される。`WebView2Loader.dll`
  と `HashiMCP-RustPreview.exe` を本体の隣から見つけられるか(外部連携の登録も含む)は実機で確認していない。
- 署名の無い exe は、winget-pkgs の検証(Defender / SmartScreen の自動スキャン)で止まることがある。

## Scoop

1. 自分のバケット用リポジトリ(例: `scoop-hashi`)を作り、`bucket/hashi.json` に `scoop/hashi.json` を置く。
2. 利用者は `scoop bucket add hashi https://github.com/shumaimai/scoop-hashi` → `scoop install hashi`。
3. `checkver` / `autoupdate` は `SHA256SUMS.txt`(Release に添付)を読む。

メイン bucket や extras への登録は、公開後しばらくして利用者が付いてから検討する。
