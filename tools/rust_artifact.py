"""Rust候補のCI成果物へ、実行ファイル・版の証跡・依存ライセンスをまとめる。"""
import argparse
import hashlib
import json
import shutil
import subprocess
from pathlib import Path


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def collect_npm_licenses(root, target):
    """Web画面（Tauri）が同梱する実行時のnpm依存のライセンスを集める。開発用依存は同梱しない。"""
    frontend = root / 'rust/crates/hashi-desktop-tauri/frontend'
    lock = frontend / 'package-lock.json'
    if not lock.exists():
        return []
    packages = json.loads(lock.read_text(encoding='utf-8'))['packages']
    target.mkdir(parents=True, exist_ok=True)
    collected = []
    for path, meta in sorted(packages.items()):
        if not path or meta.get('dev') or meta.get('optional') and meta.get('devOptional'):
            continue
        directory = frontend / path
        if not (directory / 'package.json').exists():
            raise ValueError(f'npm依存が未インストールです（npm ci が必要）: {path}')
        manifest = json.loads((directory / 'package.json').read_text(encoding='utf-8'))
        license_name = manifest.get('license') or meta.get('license')
        if isinstance(license_name, dict):
            license_name = license_name.get('type')
        if not license_name:
            raise ValueError(f"ライセンス不明（npm）: {manifest.get('name', path)}")
        name = manifest['name'].replace('/', '__')
        destination = target / f"{name}-{manifest['version']}"
        destination.mkdir(exist_ok=True)
        files = set()
        for pattern in ('LICENSE*', 'LICENCE*', 'COPYING*', 'NOTICE*'):
            files.update(p for p in directory.glob(pattern) if p.is_file())
        for index, file in enumerate(sorted(files)):
            shutil.copy2(file, destination / f'{index:02d}-{file.name}')
        # ライセンス本文を持たない依存でも、作者・ライセンス表記を残す。
        shutil.copy2(directory / 'package.json', destination / 'package.json')
        collected.append({'name': manifest['name'], 'version': manifest['version'], 'license': license_name,
                          'license_files': len(files)})
    return collected


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
    npm_dependencies = collect_npm_licenses(root, notices / 'npm')
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
              'dependencies': dependencies, 'npm_dependencies': npm_dependencies}
    (destination / 'build-info.json').write_text(json.dumps(record, ensure_ascii=False, indent=2), encoding='utf-8')
    print(f"候補成果物を作成: {args.binary.name}, {len(dependencies)} packages, {len(npm_dependencies)} npm packages")


if __name__ == '__main__':
    main()
