#!/usr/bin/env python3
"""将当前 Go 构建装配成本机可运行产物，支持显式 macOS 签名与公证；不自动发布。"""
from pathlib import Path
import argparse
import os
import platform
import plistlib
import shutil
import subprocess
import tarfile
import zipfile

ROOT = Path(__file__).resolve().parent.parent
DIST = ROOT / 'dist'
VERSION = (ROOT / 'VERSION').read_text().strip()


def run(*args):
    """在仓库根目录执行明确的打包工具，失败立即退出。"""
    subprocess.run(args, cwd=ROOT, check=True)


def main():
    """仅装配当前操作系统与架构，不将交叉编译冒充原生验收。"""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target', choices=['macos', 'windows', 'linux'])
    parser.add_argument('--arch', choices=['amd64', 'arm64'])
    parser.add_argument('--bin-dir', type=Path, default=DIST)
    parser.add_argument('--output-dir', type=Path, default=DIST)
    parser.add_argument('--identity', default=os.getenv('APPLE_SIGN_IDENTITY', ''))
    parser.add_argument('--notary-profile', default=os.getenv('APPLE_NOTARY_PROFILE', ''))
    parser.add_argument('--notarize', action='store_true')
    args = parser.parse_args()
    system = {'macos': 'Darwin', 'windows': 'Windows', 'linux': 'Linux'}.get(args.target, platform.system())
    arch = args.arch or {'aarch64': 'arm64', 'x86_64': 'amd64', 'AMD64': 'amd64'}.get(platform.machine(), platform.machine())
    binary_dir = args.bin_dir.resolve()
    output_dir = args.output_dir.resolve()
    output_dir.mkdir(parents=True, exist_ok=True)
    if system != 'Darwin' and (args.identity or args.notary_profile or args.notarize):
        parser.error('Windows/Linux 只生成未签名包，不接受 Apple 签名或公证参数')
    if system == 'Darwin' and args.notary_profile and not args.identity:
        parser.error('公证 profile 需要 Developer ID Application 签名身份')
    if system == 'Darwin' and (args.identity or args.notarize):
        if platform.system() != 'Darwin':
            parser.error('macOS 签名、公证必须在 macOS 上运行')
        from macos import package
        package(binary_dir, output_dir, VERSION, 'x86_64' if arch == 'amd64' else arch,
                args.identity, args.notary_profile, args.notarize)
        return
    if system == 'Darwin':
        app = DIST / 'MailFabrica.app'
        if app.exists():
            shutil.rmtree(app)
        binary = app / 'Contents' / 'MacOS'
        resources = app / 'Contents' / 'Resources'
        binary.mkdir(parents=True)
        resources.mkdir(parents=True)
        shutil.copy2(DIST / 'mailfabrica', binary / 'mailfabrica')
        info = {'CFBundleName': 'MailFabrica', 'CFBundleDisplayName': 'MailFabrica',
                'CFBundleIdentifier': 'dev.mailfabrica.MailFabrica', 'CFBundleExecutable': 'mailfabrica',
                'CFBundlePackageType': 'APPL', 'CFBundleShortVersionString': VERSION.split('-')[0],
                'CFBundleVersion': VERSION.split('-')[0], 'CFBundleGetInfoString': f'MailFabrica {VERSION}',
                'CFBundleIconFile': 'MailFabrica.icns', 'NSHighResolutionCapable': True,
                'NSCalendarsUsageDescription': '在日历视图中显示你的系统日程。',
                'NSCalendarsFullAccessUsageDescription': '在日历视图中和邮件一起显示你的系统日程。'}
        with (app / 'Contents' / 'Info.plist').open('wb') as handle:
            plistlib.dump(info, handle)
        run('go', 'run', './cmd/mailfabrica-icon', '--out', str(DIST / 'MailFabrica.iconset'))
        run('/usr/bin/iconutil', '-c', 'icns', str(DIST / 'MailFabrica.iconset'), '-o', str(resources / 'MailFabrica.icns'))
        run('/usr/bin/codesign', '--force', '--deep', '--sign', '-', str(app))
        print(app)
    elif system == 'Windows':
        for name in ('mailfabrica', 'mailfabrica-cli'):
            path = binary_dir / name
            if path.exists():
                path.replace(binary_dir / f'{name}.exe')
        archive = output_dir / f'MailFabrica-{VERSION}-windows-{arch}.zip'
        with zipfile.ZipFile(archive, 'w', zipfile.ZIP_DEFLATED) as output:
            for name in ('mailfabrica.exe', 'mailfabrica-cli.exe'):
                output.write(binary_dir / name, name)
            for name in ('LICENSE-MIT', 'LICENSE-APACHE'):
                output.write(ROOT / name, name)
            output.writestr('README.txt', 'MailFabrica ' + VERSION + '\nRun mailfabrica.exe. WebView2 is required. This Windows package is unsigned.\nFor isolated testing: mailfabrica.exe --demo\n')
        print(archive)
    else:
        archive = output_dir / f'MailFabrica-{VERSION}-linux-{arch}.tar.gz'
        with tarfile.open(archive, 'w:gz') as output:
            for name in ('mailfabrica', 'mailfabrica-cli'):
                output.add(binary_dir / name, name)
            for name in ('LICENSE-MIT', 'LICENSE-APACHE'):
                output.add(ROOT / name, name)
            if (binary_dir / 'dependencies.txt').is_file():
                output.add(binary_dir / 'dependencies.txt', 'dependencies.txt')
        print(archive)


if __name__ == '__main__':
    main()
