"""在一次性离线邮箱中验证 macOS WKWebView；不使用真实账号、密码或邮件服务器。"""

import argparse
import base64
import http.server
import json
from pathlib import Path
import socket
import subprocess
import tempfile
import threading
import time
from email.message import EmailMessage

PNG = base64.b64decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aHh8AAAAASUVORK5CYII=")


def rpc(port, method, params=None):
    """每次建立独立回环连接，读取完整 JSON 行并传播应用错误。"""
    if method in {"app.prefs", "ui.readerMode", "ui.readerReload", "ui.resize"}:
        rpc(port, "ui.focusWindow")
    with socket.create_connection(("127.0.0.1", port), timeout=10) as connection:
        connection.sendall((json.dumps({"id": 1, "method": method, "params": params or {}}) + "\n").encode())
        response = json.loads(connection.makefile("rb").readline())
    assert response.get("ok"), response
    return response.get("result")


def wait(port, predicate, timeout=15):
    """按可观察状态等待，不以任务已发出作为完成证据。"""
    until = time.monotonic() + timeout
    latest = None
    while time.monotonic() < until:
        latest = rpc(port, "ui.inspect")
        if predicate(latest):
            return latest
        if latest.get("visibility", {}).get("occluded"):
            rpc(port, "ui.focusWindow")
        time.sleep(0.05)
    raise AssertionError(f"Timed out: {latest['reader'] if latest else None}; focused={latest.get('focused') if latest else None}; visibility={latest.get('visibility') if latest else None}")


def seed(root, resource_port, original):
    """生成仅含公共测试数据的本地索引；可选原始邮件只复制到临时目录，绝不写入源码。"""
    account = "browser-qa"
    folder_hash = 0xCBF29CE484222325
    for byte in b"INBOX":
        folder_hash = ((folder_hash ^ byte) * 0x100000001B3) & ((1 << 64) - 1)
    folder = root / account / f"f-{folder_hash:016x}"
    folder.mkdir(parents=True)
    server = {"host": "127.0.0.1", "port": 9, "security": "none"}
    (root / "accounts.json").write_text(json.dumps([{"id": account, "email": "preview@example.invalid", "imap": server, "smtp": server}]))
    (root / "prefs.json").write_text(json.dumps({"syncIntervalMinutes": 0, "markReadOnOpen": False, "groupBySubject": False}))
    (root / account / "folders.json").write_text(json.dumps([{"account": account, "path": "INBOX", "role": "inbox", "selectable": True}]))
    url = f"http://127.0.0.1:{resource_port}"
    bodies = [
        f'<style>.card{{display:grid;grid-template-columns:1fr 1fr;background:#eef5ff;padding:24px}}@media(max-width:480px){{.card{{display:block}}}}</style>'
        f'<h2>Browser preview</h2><div class="card"><p>原生浏览器正文</p><p>CSS Grid</p></div><img src="cid:logo" width="24" height="24">'
        f'<img src="{url}/image" width="24" height="24"><div style="height:24px;background-image:url({url}/background)">Background</div>'
        f'<style>@import url({url}/stylesheet);</style><script>fetch("{url}/script")</script><iframe src="{url}/frame"></iframe>'
        '<div style="height:1400px"></div><p id="tail">End of email</p>',
        '<div style="display:none">Hidden content</div>',
        '<p>' + 'x' * (2 << 20) + '</p>',
        '<img src="cid:logo" width="100" height="100">',
        f'<a href="#privacy" style="display:block;width:240px;height:60px"><span>Privacy Policy</span></a>'
        f'<p>Slow images must not block readable text</p><img src="{url}/slow" width="24" height="24">',
        None,
    ]
    messages = []
    for uid, html in enumerate(bodies, 1):
        mail = EmailMessage()
        mail["From"] = "Preview <preview@example.invalid>"
        mail["To"] = "preview@example.invalid"
        mail["Subject"] = f"Browser QA {uid}"
        mail.set_content("Plain fallback works. 纯文本回退正常。")
        if html is not None:
            mail.add_alternative(html, subtype="html")
            mail.get_payload()[-1].add_related(PNG, maintype="image", subtype="png", cid="<logo>")
        if uid == 1:
            for index in range(20):
                mail.add_attachment(b"QA attachment", maintype="application", subtype="octet-stream", filename=f"attachment-{index}.txt")
        (folder / f"{uid}.eml").write_bytes(mail.as_bytes())
        messages.append({"key": {"account": account, "folder": "INBOX", "uid": uid}, "subject": f"Browser QA {uid}", "flags": {"seen": True}})
    if original:
        (folder / "7.eml").write_bytes(original.read_bytes())
        messages.append({"key": {"account": account, "folder": "INBOX", "uid": 7}, "subject": "Original email", "flags": {"seen": True}})
    (folder / "index.json").write_text(json.dumps({"uidvalidity": 1, "messages": messages}))


def main():
    """启动当前构建，验证真实浏览器反馈、资源请求和模式隔离，然后退出并回收测试邮箱。"""
    parser = argparse.ArgumentParser()
    parser.add_argument("binary", type=Path)
    parser.add_argument("--original", type=Path)
    parser.add_argument("--result", type=Path, required=True)
    parser.add_argument("--start-file", type=Path, help="等待原生窗口前置后创建此文件，适用于限制后台激活的 macOS 会话")
    parser.add_argument("--bundle", type=Path, help="通过 Launch Services 启动一次性测试应用，避免直接执行 Mach-O 时窗口持续被系统判为遮挡")
    args = parser.parse_args()
    requests = []

    class Handler(http.server.BaseHTTPRequestHandler):
        """只提供测试图片并记录实际请求，不访问外网。"""
        def do_GET(self):
            """资源探针；脚本、iframe 和外部样式表不得请求到这里。"""
            requests.append({"path": self.path, "cookie": self.headers.get("Cookie")})
            if self.path == "/slow":
                time.sleep(3)
            self.send_response(200)
            self.send_header("Content-Type", "image/png")
            self.end_headers()
            try:
                self.wfile.write(PNG)
            except (BrokenPipeError, ConnectionResetError):
                # 快速换信会正常取消旧 WebView 的慢图请求。
                pass

        def log_message(self, *_):
            """避免把 HTTP 日志混入测试结果。"""

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    with socket.socket() as reserve:
        reserve.bind(("127.0.0.1", 0))
        port = reserve.getsockname()[1]
    results = {}
    with tempfile.TemporaryDirectory(prefix="mailfabrica-browser-qa-") as temporary:
        root = Path(temporary)
        seed(root, server.server_port, args.original)
        with (root / "runtime.log").open("wb") as log:
            command = [str(args.binary.resolve())]
            if args.bundle:
                assert (args.bundle / "Contents/MacOS" / args.binary.name).resolve() == args.binary.resolve(), "Binary does not belong to bundle"
                command = ["/usr/bin/open", "-W", "-n", "-a", str(args.bundle.resolve()), "--args"]
            process = subprocess.Popen(command + ["--data", str(root), "--control", str(port)], stdout=log, stderr=log)
            try:
                for _ in range(100):
                    try:
                        rpc(port, "ui.inspect")
                        break
                    except (OSError, ValueError):
                        assert process.poll() is None, (root / "runtime.log").read_text()
                        time.sleep(0.1)
                rpc(port, "view.scope", {"account": "browser-qa", "folder": "INBOX"})

                def select(uid, status="ready"):
                    """等待当前邮件的真实 HTML 状态，防止读到上一封邮件的就绪状态。"""
                    key = f"browser-qa/{uid}/INBOX"
                    before = rpc(port, "ui.inspect")["reader"]["browser"]["generation"]
                    rpc(port, "ui.focusWindow")
                    rpc(port, "view.select", {"key": key})
                    return wait(port, lambda v: v["focused"] == key and v["reader"]["browser"]["generation"] > before and v["reader"]["browser"]["status"] == status)

                assert rpc(port, "app.prefs")["remoteImages"] is True
                select(1)
                if args.start_file:
                    deadline = time.monotonic() + 60
                    while not args.start_file.exists():
                        assert time.monotonic() < deadline, "Waiting for native window activation timed out"
                        time.sleep(0.1)
                enabled = wait(port, lambda v: (v["reader"]["browser"]["metrics"] or {}).get("loadedImages") == 2)["reader"]["browser"]
                assert enabled["remoteImages"], enabled
                assert {r["path"] for r in requests} == {"/image", "/background"}, requests
                assert all(r["cookie"] is None for r in requests), requests
                results["defaultEnabledImages"] = enabled
                # 通过正式全局设置表单关闭，再检查当前邮件立即重载且不再请求网络。
                rpc(port, "ui.dialog", {"name": "settings"})
                rpc(port, "ui.dialog.set", {"field": "remoteImages", "value": False})
                rpc(port, "ui.dialog.confirm")
                assert rpc(port, "app.prefs")["remoteImages"] is False
                assert json.loads((root / "prefs.json").read_text())["remoteImages"] is False
                requests.clear()
                first = wait(port, lambda v: (v["reader"]["browser"]["metrics"] or {}).get("loadedImages") == 1)["reader"]["browser"]
                assert first["metrics"]["height"] > 1000 and first["metrics"]["loadedImages"] == 1, first
                assert first["rect"][3] > 100, first
                results["manyAttachmentsKeepBodyVisible"] = first["rect"][3]
                time.sleep(0.2)
                assert not requests, requests
                results["globalSettingBlocksAndKeepsCid"] = first
                rpc(port, "ui.readerSelect", {"start": 0, "end": 7})
                wait(port, lambda v: (v["reader"]["browser"]["metrics"] or {}).get("selectedChars") == 7)
                rpc(port, "ui.readerCopy")
                wait(port, lambda v: (v["reader"]["browser"]["metrics"] or {}).get("copiedChars") == 7)
                rpc(port, "ui.readerScroll", {"top": 600})
                wait(port, lambda v: (v["reader"]["browser"]["metrics"] or {}).get("scrollTop", 0) >= 500)
                results["selectionCopyScroll"] = True
                rpc(port, "app.prefs", {"remoteImages": True})
                allowed = wait(port, lambda v: v["reader"]["browser"]["remoteImages"] and v["reader"]["browser"]["status"] == "ready" and (v["reader"]["browser"]["metrics"] or {}).get("loadedImages") == 2)["reader"]["browser"]
                assert allowed["metrics"]["loadedImages"] == 2, allowed
                assert {r["path"] for r in requests} == {"/image", "/background"}, requests
                assert all(r["cookie"] is None for r in requests), requests
                results["authorizedResourcesOnly"] = list(requests)
                rpc(port, "ui.readerMode", {"mode": "plain"})
                wait(port, lambda v: v["reader"]["mode"] == "plain" and not v["reader"]["browser"]["visible"])
                rpc(port, "ui.readerMode", {"mode": "html"})
                wait(port, lambda v: v["reader"]["browser"]["visible"])
                rpc(port, "ui.dialog", {"name": "about"})
                wait(port, lambda v: v["dialog"] is not None and not v["reader"]["browser"]["visible"])
                rpc(port, "ui.dialog.cancel")
                wait(port, lambda v: v["dialog"] is None and v["reader"]["browser"]["visible"])
                results["plainAndDialogIsolation"] = True
                rpc(port, "view.select", {"key": "browser-qa/6/INBOX"})
                wait(port, lambda v: v["focused"] == "browser-qa/6/INBOX" and v["reader"]["mode"] == "plain" and not v["reader"]["browser"]["visible"])
                assert rpc(port, "app.prefs")["remoteImages"] is True
                assert select(1)["reader"]["browser"]["remoteImages"]
                results["plainMessageKeepsGlobalSetting"] = True
                hidden = select(2, "failed")
                assert not hidden["reader"]["browser"]["visible"]
                wait(port, lambda v: v["reader"]["mode"] == "plain" and "reader.retryHtml" in v["targets"])
                results["hiddenFallback"] = True
                large = select(3, "failed")["reader"]["browser"]
                assert "2 MiB" in large["error"], large
                results["oversizeFallback"] = True
                image = select(4)["reader"]["browser"]
                assert image["metrics"]["loadedImages"] == 1, image
                results["imageOnly"] = True
                reset = select(1)["reader"]["browser"]
                assert reset["remoteImages"], reset
                rpc(port, "ui.resize", {"width": 900, "height": 600})
                narrow = wait(port, lambda v: (v["reader"]["browser"]["metrics"] or {}).get("width", 9999) < 480)["reader"]["browser"]
                results["globalSettingAndResize"] = narrow
                rpc(port, "app.prefs", {"remoteImages": False})
                select(5)
                started = time.monotonic()
                rpc(port, "app.prefs", {"remoteImages": True})
                wait(port, lambda v: v["reader"]["browser"]["remoteImages"] and v["reader"]["browser"]["status"] == "ready", timeout=2)
                results["slowImagesDoNotBlockText"] = {"readySeconds": time.monotonic() - started}
                def hit_test(x, y, expected):
                    """使用生产悬停路径的只读命中脚本；不会移动系统指针或把样式当成原生光标证明。"""
                    rpc(port, "ui.readerHitTest", {"x": x, "y": y})
                    return wait(port, lambda v: v["reader"]["browser"].get("hitTest") == {"point": [x, y], "cursor": expected})

                pending_link = hit_test(24, 24, "pointer")["reader"]["browser"]
                assert pending_link["metrics"]["pending"] and pending_link["metrics"]["loadedImages"] == 0, pending_link
                results["linkWhileImagePending"] = pending_link
                wait(port, lambda v: (v["reader"]["browser"]["metrics"] or {}).get("loadedImages") == 1)
                hit_test(24, 24, "pointer")
                for _ in range(12):
                    select(4)
                    hit_test(24, 24, "default")
                    select(5)
                    link = hit_test(24, 24, "pointer")["reader"]["browser"]
                    assert link["metrics"]["pending"] and link["metrics"]["loadedImages"] == 0, link
                results["linkHitTestSurvives24MessageSwitches"] = True
                if args.original:
                    rpc(port, "app.prefs", {"remoteImages": False})
                    original = select(7)["reader"]["browser"]
                    assert original["metrics"]["height"] > 500, original
                    results["originalMessage"] = original
            except Exception:
                args.result.with_suffix(".runtime.log").write_text((root / "runtime.log").read_text())
                raise
            finally:
                try:
                    rpc(port, "app.quit")
                    process.wait(timeout=10)
                except (OSError, ValueError, subprocess.TimeoutExpired):
                    process.terminate()
                    process.wait(timeout=10)
                server.shutdown()
    args.result.write_text(json.dumps(results, ensure_ascii=False, indent=2))
    print(f"PASS: {len(results)} browser scenarios; {args.result}")


if __name__ == "__main__":
    main()
