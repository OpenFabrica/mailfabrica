#!/usr/bin/env python3
"""在 macOS 桌面用隔离演示数据验证常驻隐藏、恢复和未读数字，不签名、不公证、不操作真实账户。"""
from pathlib import Path
import json
import plistlib
import shutil
import socket
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent


def main():
    """真实调用窗口关闭钩子，恢复后必须仍能连接原进程，以覆盖 Cocoa 主线程崩溃回归。"""
    if sys.platform != 'darwin':
        raise SystemExit('此验收脚本需要 macOS 图形桌面；普通检查使用 make check。')
    binary = ROOT / 'dist/mailfabrica'
    cli = ROOT / 'dist/mailfabrica-cli'
    with tempfile.TemporaryDirectory(prefix='mailfabrica-native-smoke-') as directory:
        workspace = Path(directory)
        data = workspace / 'data'
        policy_reader = workspace / 'native-policy'
        subprocess.run(['swiftc', str(ROOT / 'scripts/native-policy.swift'), '-o', str(policy_reader)], check=True)
        app = workspace / 'MailFabrica Native Smoke.app'
        executable = app / 'Contents/MacOS/mailfabrica'
        executable.parent.mkdir(parents=True)
        shutil.copy2(binary, executable)
        with (app / 'Contents/Info.plist').open('wb') as info:
            plistlib.dump({'CFBundleName': 'MailFabrica Native Smoke',
                          'CFBundleIdentifier': 'dev.mailfabrica.native-smoke',
                          'CFBundleExecutable': 'mailfabrica', 'CFBundlePackageType': 'APPL',
                          'CFBundleVersion': '0.1.6', 'NSHighResolutionCapable': True}, info)
        subprocess.run([str(cli), '--demo', '--data', str(data), 'exec', 'app.prefs',
                        '{"closeToTray":true,"syncIntervalMinutes":0}'], check=True, stdout=subprocess.DEVNULL)
        with socket.socket() as available:
            available.bind(('127.0.0.1', 0))
            port = available.getsockname()[1]
        with (data / 'native.log').open('w+') as log:
            process = subprocess.Popen([str(executable), '--demo', '--data', str(data), '--control', str(port)], stdout=log, stderr=log)
            sequence = 0

            def call(method, params=None):
                """每次使用独立有界连接；恢复时的 EOF 视为失败，不把进程消失当作窗口已关闭。"""
                nonlocal sequence
                sequence += 1
                with socket.create_connection(('127.0.0.1', port), timeout=2) as channel:
                    channel.settimeout(10)
                    channel.sendall((json.dumps({'id': sequence, 'method': method, 'params': params or {}}) + '\n').encode())
                    line = channel.makefile().readline()
                    if not line:
                        raise RuntimeError(f'{method}: native process disconnected')
                    response = json.loads(line)
                    if not response['ok']:
                        raise RuntimeError(response.get('error', method))
                    return response.get('result')

            def wait_for(predicate, timeout=10):
                """等待可观察的原生状态；启动期间只重试尚未监听的端口。"""
                end = time.monotonic() + timeout
                while time.monotonic() < end:
                    if process.poll() is not None:
                        raise RuntimeError(f'native app exited early: {process.returncode}')
                    try:
                        state = call('ui.tray')
                        if predicate(state):
                            return state
                    except ConnectionRefusedError:
                        pass
                    time.sleep(.1)
                raise RuntimeError('native tray state timed out')

            def execute(command, params):
                """沿用引擎命令设置偏好和邮件标记。"""
                return call('engine.execute', {'command': command, 'params': params})

            def expect_dock(visible):
                """从系统进程信息验证 Dock 策略，允许 macOS 完成策略切换。"""
                expected = '0' if visible else '1'
                end = time.monotonic() + 5
                while time.monotonic() < end:
                    actual = subprocess.check_output([str(policy_reader), str(process.pid)], text=True).strip()
                    if actual == expected:
                        return
                    time.sleep(.1)
                raise RuntimeError(f'Dock policy: expected {expected}, got {actual}')

            try:
                wait_for(lambda s: s['created'] and s['label'] == '5')
                expect_dock(True)
                for _ in range(3):
                    call('ui.closeWindow')
                    wait_for(lambda s: not s['windowVisible'] and s['created'])
                    expect_dock(False)
                    call('ui.showWindow')
                    wait_for(lambda s: s['windowVisible'])
                    expect_dock(True)
                call('ui.closeWindow')
                wait_for(lambda s: not s['windowVisible'])
                expect_dock(False)
                execute('view.select', {'key': 'demo-mailfabrica-dev/5/INBOX'})
                wait_for(lambda s: s['unread'] == 4 and s['label'] == '4' and not s['windowVisible'])
                execute('message.markRead', {'read': False})
                wait_for(lambda s: s['unread'] == 5 and s['label'] == '5')
                expect_dock(False)
                execute('app.prefs', {'closeToTray': False})
                wait_for(lambda s: not s['created'] and s['windowVisible'])
                expect_dock(True)
                execute('app.prefs', {'closeToTray': True})
                wait_for(lambda s: s['created'])
                execute('folder.markAllRead', {'account': 'demo-mailfabrica-dev', 'folder': 'INBOX'})
                wait_for(lambda s: s['unread'] == 0 and s['label'] == '')
                execute('message.markRead', {'read': False})
                wait_for(lambda s: s['unread'] == 1 and s['label'] == '1')
                execute('app.prefs', {'closeToTray': False})
                wait_for(lambda s: not s['created'])
                call('ui.closeWindow')
                if process.wait(timeout=10) != 0:
                    raise RuntimeError('normal close did not exit cleanly')
                print('PASS: native close/hide Dock → background badge → restore window/Dock ×3 → disable/restore → zero badge → normal exit')
            except Exception:
                log.flush()
                log.seek(0)
                print('\n'.join(log.read().splitlines()[:30]), file=sys.stderr)
                raise
            finally:
                if process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait(timeout=5)


if __name__ == '__main__':
    main()
