"""构建带 Developer ID 和 Apple 公证票据的 macOS 应用及 DMG。"""
from pathlib import Path
import datetime
import hashlib
import json
import os
import plistlib
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent


def run(*args, capture=False, timeout=120):
    """执行参数数组，不展开 shell；公证凭据只使用钥匙串 profile。"""
    result = subprocess.run([str(arg) for arg in args], cwd=ROOT, check=True,
                            capture_output=capture, text=True, timeout=timeout)
    return result.stdout.strip() if capture else None


def digest(path):
    """计算文件摘要，用于最终产物核验。"""
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def provenance():
    """记录包含未提交源文件的构建身份，避免同一 Git revision 冒充相同源码。"""
    revision = run('git', 'rev-parse', '--short=12', 'HEAD', capture=True)
    dirty = bool(run('git', 'status', '--porcelain', capture=True))
    paths = run('git', 'ls-files', '--cached', '--others', '--exclude-standard', '-z', capture=True).split('\0')
    source = hashlib.sha256()
    for name in sorted(set(paths)):
        path = ROOT / name
        if name and path.is_file():
            source.update(name.encode() + b'\0' + bytes.fromhex(digest(path)))
    return {'revision': revision, 'dirty': dirty, 'sourceSHA256': source.hexdigest()}


def notarize(path, profile, audit):
    """先保存提交 UUID，再等待 Accepted；失败保留 Apple 日志供重查。"""
    receipt = json.loads(run('xcrun', 'notarytool', 'submit', path,
                             '--keychain-profile', profile, '--output-format', 'json',
                             '--no-progress', capture=True, timeout=300))
    submission = receipt['id']
    (audit / f'{submission}-submission.json').write_text(json.dumps(receipt, indent=2) + '\n')
    print(f'Apple submission: {submission} ({path.name})', flush=True)
    status = json.loads(run('xcrun', 'notarytool', 'wait', submission,
                            '--keychain-profile', profile, '--timeout', '15m',
                            '--output-format', 'json', '--no-progress', capture=True, timeout=920))
    (audit / f'{submission}-status.json').write_text(json.dumps(status, indent=2) + '\n')
    run('xcrun', 'notarytool', 'log', submission, '--keychain-profile', profile,
        audit / f'{submission}-log.json')
    if status.get('status') != 'Accepted':
        raise RuntimeError(f'Apple 公证未通过：{submission}，详见 {audit}')
    print(f'Apple Accepted: {submission}', flush=True)
    return submission


def staple(path, app):
    """装订并验证离线票据，再检查 Gatekeeper 判定。"""
    run('xcrun', 'stapler', 'staple', path)
    run('xcrun', 'stapler', 'validate', path)
    args = ['/usr/sbin/spctl', '--assess', '--verbose=2', '--type', 'execute' if app else 'open']
    if not app:
        args += ['--context', 'context:primary-signature']
    run(*args, path)


def package(binary_dir, output_dir, version, arch, identity, profile, notarized):
    """从当前二进制装配唯一输出目录，不覆盖仍运行的开发应用。"""
    if not identity.startswith('Developer ID Application:'):
        raise ValueError('macOS 分发需要 Developer ID Application 身份')
    if notarized and not profile:
        raise ValueError('公证需要 APPLE_NOTARY_PROFILE 或 --notary-profile')
    source = provenance()
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%SZ')
    name = f'MailFabrica-{version}-{source["sourceSHA256"][:8]}-{stamp}-{arch}'
    destination = output_dir / name
    destination.mkdir(parents=True, exist_ok=False)
    audit = destination / 'notary'
    audit.mkdir()
    app = destination / 'MailFabrica.app'
    binaries = app / 'Contents/MacOS'
    resources = app / 'Contents/Resources'
    binaries.mkdir(parents=True)
    resources.mkdir()
    for executable in ['mailfabrica', 'mailfabrica-cli']:
        shutil.copy2(binary_dir / executable, binaries / executable)
    info = {'CFBundleName': 'MailFabrica', 'CFBundleDisplayName': 'MailFabrica',
            'CFBundleIdentifier': 'dev.mailfabrica.MailFabrica', 'CFBundleExecutable': 'mailfabrica',
            'CFBundlePackageType': 'APPL', 'CFBundleShortVersionString': version.split('-')[0],
            'CFBundleVersion': version.split('-')[0], 'CFBundleGetInfoString': f'MailFabrica {version}',
            'CFBundleIconFile': 'MailFabrica.icns', 'NSHighResolutionCapable': True,
            'NSPrincipalClass': 'NSApplication', 'LSMinimumSystemVersion': '13.0',
            'LSApplicationCategoryType': 'public.app-category.productivity',
            'NSCalendarsUsageDescription': '在日历视图中显示你的系统日程。',
            'NSCalendarsFullAccessUsageDescription': '在日历视图中和邮件一起显示你的系统日程。'}
    with (app / 'Contents/Info.plist').open('wb') as handle:
        plistlib.dump(info, handle)
    for license_name in ['LICENSE-MIT', 'LICENSE-APACHE']:
        shutil.copy2(ROOT / license_name, resources / license_name)
    submissions = []
    with tempfile.TemporaryDirectory(prefix='mailfabrica-package-') as temporary:
        stage = Path(temporary)
        iconset = stage / 'MailFabrica.iconset'
        run('go', 'run', './cmd/mailfabrica-icon', '--out', iconset)
        run('/usr/bin/iconutil', '-c', 'icns', iconset, '-o', resources / 'MailFabrica.icns')
        entitlements = stage / 'entitlements.plist'
        with entitlements.open('wb') as handle:
            plistlib.dump({'com.apple.security.personal-information.calendars': True}, handle)
        for path in [binaries / 'mailfabrica-cli', app]:
            run('/usr/bin/codesign', '--force', '--sign', identity, '--timestamp',
                '--options', 'runtime', '--entitlements', entitlements, path)
        run('/usr/bin/codesign', '--verify', '--deep', '--strict', app)
        for executable in ['mailfabrica', 'mailfabrica-cli']:
            run('/usr/bin/lipo', '-verify_arch', arch, binaries / executable)
        if notarized:
            archive = stage / 'application.zip'
            run('/usr/bin/ditto', '-c', '-k', '--keepParent', app, archive)
            submissions.append(notarize(archive, profile, audit))
            staple(app, True)
        payload = stage / 'payload'
        payload.mkdir()
        run('/usr/bin/ditto', app, payload / 'MailFabrica.app')
        os.symlink('/Applications', payload / 'Applications')
        (payload / '安装说明.txt').write_text(f'MailFabrica {version}\n\n将 MailFabrica.app 拖入 Applications 文件夹后启动。\n原有账户与邮件目录沿用，无需重复添加账户。\n命令行：MailFabrica.app/Contents/MacOS/mailfabrica-cli\n')
        dmg = destination / f'MailFabrica-{version}-{arch}.dmg'
        run('/usr/bin/hdiutil', 'create', '-volname', 'MailFabrica', '-srcfolder', payload,
            '-format', 'UDZO', dmg)
        run('/usr/bin/codesign', '--force', '--sign', identity, '--timestamp', dmg)
        run('/usr/bin/codesign', '--verify', '--strict', dmg)
        if notarized:
            submissions.append(notarize(dmg, profile, audit))
            staple(dmg, False)
        run('/usr/bin/hdiutil', 'verify', dmg)
    manifest = {**source, 'version': version, 'architecture': arch, 'createdUTC': stamp,
                'signature': identity, 'notarized': notarized, 'submissions': submissions,
                'sha256': {'mailfabrica': digest(binaries / 'mailfabrica'),
                           'mailfabrica-cli': digest(binaries / 'mailfabrica-cli'), 'dmg': digest(dmg)}}
    (destination / 'build.json').write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + '\n')
    print(f'安装包：{dmg}', flush=True)
    return destination
