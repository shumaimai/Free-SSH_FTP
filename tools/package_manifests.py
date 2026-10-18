"""winget / Scoop 用のマニフェストをテンプレートから作る。

    python tools/package_manifests.py 2.0.0 <zipのSHA-256> <出力フォルダ>
    python tools/package_manifests.py 2.0.0 --zip Hashi-v2.0.0-windows-x64.zip <出力フォルダ>

テンプレートは packaging/ にある。出力は winget-pkgs / Scoop バケットへ出す提出物の下書き。
release.yml のリリース時にも実行して、workflow の成果物 package-manifests に置く。
"""
import argparse
import hashlib
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TEMPLATES = [
    (ROOT / "packaging/winget/shumaimai.Hashi.yaml.tpl", "winget/shumaimai.Hashi.yaml"),
    (ROOT / "packaging/winget/shumaimai.Hashi.installer.yaml.tpl", "winget/shumaimai.Hashi.installer.yaml"),
    (ROOT / "packaging/winget/shumaimai.Hashi.locale.en-US.yaml.tpl", "winget/shumaimai.Hashi.locale.en-US.yaml"),
    (ROOT / "packaging/scoop/hashi.json.tpl", "scoop/hashi.json"),
]


def main() -> int:
    for stream in (sys.stdout, sys.stderr):
        stream.reconfigure(encoding="utf-8")
    parser = argparse.ArgumentParser()
    parser.add_argument("version", help="X.Y.Z(先頭の v は付けない)")
    parser.add_argument("sha256", nargs="?", help="zip の SHA-256(16進64桁)")
    parser.add_argument("--zip", help="SHA-256 をこの zip から計算する")
    parser.add_argument("output")
    args = parser.parse_args()

    if not re.fullmatch(r"(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)", args.version):
        parser.error("version は X.Y.Z 形式にしてください")
    if args.zip:
        digest = hashlib.sha256(Path(args.zip).read_bytes()).hexdigest()
    elif args.sha256 and re.fullmatch(r"[0-9a-fA-F]{64}", args.sha256):
        digest = args.sha256.lower()
    else:
        parser.error("SHA-256(16進64桁)か --zip が必要です")

    out = Path(args.output)
    for src, name in TEMPLATES:
        text = src.read_text(encoding="utf-8")
        text = text.replace("@VERSION@", args.version).replace("@SHA256@", digest.upper() if name.endswith(".yaml") else digest)
        dest = out / name
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_text(text, encoding="utf-8", newline="\n")
        print("wrote", dest)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
