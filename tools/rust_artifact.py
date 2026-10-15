"""Rust候補のCI成果物へ、実行ファイル・版の証跡・依存ライセンスをまとめる。"""
import argparse
import hashlib
import json
import shutil
import subprocess
from pathlib import Path


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--helper', type=Path, action='append', default=[],
                        help='同梱するconsole-modeのヘルパー（HashiMCP等）')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--cargo', default='cargo')
    parser.add_argument('--profile', choices=['debug', 'release'], default='debug')
    parser.add_argument('--tag', default='', help='公開するrust-preview-vタグ。CI成果物では空')
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    metadata = json.loads(subprocess.check_output(
        [args.cargo, 'metadata', '--locked', '--format-version', '1',
         '--manifest-path', str(root / 'rust/Cargo.toml')], text=True))
    destination = args.output.resolve()
    destination.mkdir(parents=True, exist_ok=True)
    shutil.copy2(args.binary, destination / args.binary.name)
    for helper in args.helper:
        shutil.copy2(helper, destination / helper.name)
    notices = destination / 'licenses'
    notices.mkdir(exist_ok=True)
    dependencies = []
    for package in sorted(metadata['packages'], key=lambda p: (p['name'], p['version'])):
        if not package.get('license') and not package.get('license_file'):
            raise ValueError(f"ライセンス不明: {package['name']}")
        info = {field: package.get(field) for field in ('name', 'version', 'license', 'source', 'repository')}
        directory = Path(package['manifest_path']).parent
        license_paths = set()
        for pattern in ('LICENSE*', 'LICENCE*', 'COPYING*', 'NOTICE*', 'LICENSES/*'):
            license_paths.update(path for path in directory.glob(pattern) if path.is_file())
        if package['name'] == 'epaint_default_fonts':
            license_paths.update((directory / 'fonts').glob('*.txt'))
        if package.get('license_file'):
            license_paths.add(directory / package['license_file'])
        target = notices / f"{package['name']}-{package['version']}"
        target.mkdir(exist_ok=True)
        for index, path in enumerate(sorted(license_paths)):
            shutil.copy2(path, target / f'{index:02d}-{path.name}')
        (target / 'package.json').write_text(json.dumps(info, indent=2, ensure_ascii=False), encoding='utf-8')
        dependencies.append(info)
    shutil.copy2(root / 'LICENSE', notices / 'Hashi-LICENSE')
    shutil.copy2(root / 'rust/crates/hashi-desktop-native/assets/OFL.txt', notices / 'NotoSansJP-OFL.txt')
    shutil.copy2(root / 'rust/THIRD_PARTY.md', destination / 'THIRD_PARTY.md')
    record = {'channel': 'rust-preview', 'release': bool(args.tag), 'tag': args.tag or None,
              'profile': args.profile,
              'source_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip(),
              'binary': args.binary.name, 'binary_sha256': sha256(args.binary),
              'helpers': {helper.name: sha256(helper) for helper in args.helper},
              'cargo_lock_sha256': sha256(root / 'rust/Cargo.lock'),
              'font_sha256': sha256(root / 'rust/crates/hashi-desktop-native/assets/NotoSansJP.ttf'),
              'dependencies': dependencies}
    (destination / 'build-info.json').write_text(json.dumps(record, ensure_ascii=False, indent=2), encoding='utf-8')
    print(f"候補成果物を作成: {args.binary.name}, {len(dependencies)} packages")


if __name__ == '__main__':
    main()
