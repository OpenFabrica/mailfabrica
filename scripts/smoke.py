#!/usr/bin/env python3
"""启动真实 Go Web 宿主，验证隔离演示数据上的收件、富文本发送与导出。"""
from pathlib import Path
import base64
import json
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parent.parent


def main():
    """所有数据在临时目录中，退出时回收自己启动的进程。"""
    with socket.socket() as available:
        available.bind(('127.0.0.1', 0))
        port = available.getsockname()[1]
    executable = ROOT / 'dist' / 'mailfabrica'
    if not executable.exists():
        executable = executable.with_suffix('.exe')
    with tempfile.TemporaryDirectory(prefix='mailfabrica-smoke-') as temporary:
        directory = Path(temporary)
        with (directory / 'server.log').open('wb') as log:
            process = subprocess.Popen([str(executable), '--demo', '--data', str(directory / 'mail'), '--web', str(port)], stdout=log, stderr=log)
        endpoint = f'http://127.0.0.1:{port}/api/'

        def call(method, args):
            """通过与前端相同的 HTTP 边界读写会话。"""
            request = urllib.request.Request(endpoint + method, data=json.dumps(args).encode(), headers={'Content-Type': 'application/json'})
            with urllib.request.urlopen(request, timeout=5) as response:
                result = json.load(response)
            if 'error' in result:
                raise RuntimeError(result['error'])
            return result['result']

        def execute(command, params=None):
            """调用唯一命令入口。"""
            return call('Execute', [command, json.dumps(params or {})])

        def settle():
            """等待可观察的后台状态完成，不把开始任务当作成功。"""
            deadline = time.monotonic() + 15
            while time.monotonic() < deadline:
                snapshot = call('Snapshot', [])
                if not snapshot['status']['busy']:
                    if snapshot['status']['errors']:
                        raise RuntimeError(snapshot['status']['errors'])
                    return snapshot
                time.sleep(.05)
            raise TimeoutError('background task did not finish')

        try:
            deadline = time.monotonic() + 15
            while True:
                try:
                    snapshot = call('Snapshot', [])
                    break
                except (OSError, urllib.error.URLError):
                    if process.poll() is not None or time.monotonic() > deadline:
                        raise RuntimeError((directory / 'server.log').read_text(encoding='utf-8', errors='replace'))
                    time.sleep(.05)
            assert snapshot['demo'] and len(snapshot['view']['messages']) >= 5
            assert snapshot['prefs']['mailListDensity'] == 'compact'
            execute('view.select', {'key': 'demo-mailfabrica-dev/1/INBOX'})
            settle()
            message = execute('message.get', {'key': 'demo-mailfabrica-dev/1/INBOX'})
            assert message['state'] == 'ready' and message['body']['html']
            compose = execute('compose.reply', {'background': True})['id']
            execute('compose.setHtml', {'id': compose, 'html': '<p>你好，<strong>收到🙂</strong></p>'})
            execute('compose.attach', {'id': compose, 'name': '验证.txt', 'base64': base64.b64encode('真实附件'.encode()).decode()})
            execute('compose.send', {'id': compose})
            settle()
            sent = execute('message.list', {'account': 'demo-mailfabrica-dev', 'folder': 'Sent'})['messages']
            assert len(sent) == 1
            key = sent[0]['key']
            key = f"{key['account']}/{key['uid']}/{key['folder']}"
            body = execute('message.get', {'key': key})['body']
            assert '收到🙂' in body['text'] and 'font-weight:700' in body['html']
            assert any(item['name'] == '验证.txt' for item in body['attachments'])
            execute('invoice.export', {'dir': str(directory / 'invoices')})
            snapshot = settle()
            assert snapshot['invoice']['files'] == 1 and Path(snapshot['invoice']['csv']).is_file()
            days = execute('calendar.days', {'since': '2026-10-01', 'until': '2026-10-31'})
            assert len(days['days']) == 31 and days['supported'] is False
            assert all(day['lunar'] for day in days['days'])
            print('PASS: Go host -> command engine -> store -> MIME send -> sent mailbox -> invoice files -> calendar')
        finally:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)


if __name__ == '__main__':
    main()
