#!/usr/bin/env python3
"""在本机交叉构建 Windows，并通过隔离 Linux 容器构建未签名 portable 包。"""
from pathlib import Path
import argparse
import os
import subprocess

ROOT = Path(__file__).resolve().parent.parent


def run(*args, **kwargs):
    """固定工作目录并直接执行参数列表，任一步失败立即停止。"""
    subprocess.run([str(arg) for arg in args], cwd=ROOT, check=True, **kwargs)


def main():
    """前端须预先构建；每个平台使用独立输出目录，禁止覆盖 macOS 二进制。"""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target', choices=['windows', 'linux'], required=True)
    parser.add_argument('--arch', choices=['amd64', 'arm64'], required=True)
    args = parser.parse_args()
    output = ROOT / 'dist' / f'{args.target}-{args.arch}'
    output.mkdir(parents=True, exist_ok=True)
    version = (ROOT / 'VERSION').read_text().strip()
    flags = f'-s -w -X github.com/openfabrica/mailfabrica/internal/engine.Version={version}'
    if args.target == 'windows':
        environment = dict(os.environ, GOOS='windows', GOARCH=args.arch, CGO_ENABLED='0')
        for name in ['mailfabrica', 'mailfabrica-cli']:
            run('go', 'build', '-trimpath', '-ldflags', flags + (' -H=windowsgui' if name == 'mailfabrica' else ''), '-o', output / f'{name}.exe', f'./cmd/{name}', env=environment)
    else:
        image = f'mailfabrica-linux-build:{args.arch}'
        run('docker', 'build', '--platform', f'linux/{args.arch}', '-t', image, '-f', 'scripts/linux/Dockerfile', 'scripts/linux')
        # 源码只读挂载；Go 缓存留在临时容器，只有平台输出目录可写。
        shell = '''set -eu
          go vet ./internal/... ./cmd/mailfabrica-cli
          go test ./internal/... ./cmd/mailfabrica-cli
          go build -trimpath -ldflags "$BUILD_FLAGS" -o /out/mailfabrica ./cmd/mailfabrica
          go build -trimpath -ldflags "$BUILD_FLAGS" -o /out/mailfabrica-cli ./cmd/mailfabrica-cli
          mkdir -p /tmp/verification/scripts /tmp/verification/dist
          cp /src/scripts/smoke.py /tmp/verification/scripts/
          ln -s /out/mailfabrica /tmp/verification/dist/mailfabrica
          python3 /tmp/verification/scripts/smoke.py
          file /out/mailfabrica /out/mailfabrica-cli
          ldd /out/mailfabrica > /out/dependencies.txt
        '''
        run('docker', 'run', '--rm', '--platform', f'linux/{args.arch}',
            '-v', f'{ROOT}:/src:ro', '-v', f'{output}:/out', '-e', f'BUILD_FLAGS={flags}', image, 'sh', '-c', shell)
    run('python3', 'scripts/package.py', '--target', args.target, '--arch', args.arch,
        '--bin-dir', output, '--output-dir', ROOT / 'dist/packages')


if __name__ == '__main__':
    main()
