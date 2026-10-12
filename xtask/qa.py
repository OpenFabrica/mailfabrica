"""在原生 CI runner 解包并运行发布资产，记录实际界面；只使用内存演示邮箱。"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import tarfile
import tempfile
import time
import zipfile


def digest(path):
    """流式校验实际文件而非构建任务状态。"""
    checksum = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1 << 20), b""):
            checksum.update(chunk)
    return checksum.hexdigest()


def unpack(archive, target):
    """只解包普通文件和目录；拒绝路径穿越、符号链接和过大资产。"""
    def destination(name):
        path = target / name
        if not path.resolve().is_relative_to(target.resolve()):
            raise ValueError("Archive path escapes its directory")
        return path
    if archive.suffix == ".zip":
        with zipfile.ZipFile(archive) as package:
            for item in package.infolist():
                path = destination(item.filename)
                if item.is_dir():
                    path.mkdir(parents=True, exist_ok=True)
                else:
                    if item.file_size > 256 << 20:
                        raise ValueError("Archive member is too large")
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_bytes(package.read(item))
    else:
        with tarfile.open(archive, "r:gz") as package:
            for item in package.getmembers():
                path = destination(item.name)
                if item.isdir():
                    path.mkdir(parents=True, exist_ok=True)
                elif item.isfile() and item.size <= 256 << 20:
                    stream = package.extractfile(item)
                    if stream is None:
                        raise ValueError("Missing archive member")
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_bytes(stream.read())
                    path.chmod(item.mode & 0o755)
                else:
                    raise ValueError("Unsupported archive member")


def architecture(path, platform, arch):
    """检查 PE/ELF 的真实机器类型，Windows 还必须包含图标与版本资源。"""
    data = path.read_bytes()
    if platform == "windows":
        offset = struct.unpack_from("<I", data, 0x3C)[0]
        assert data[:2] == b"MZ" and data[offset:offset + 4] == b"PE\0\0"
        assert struct.unpack_from("<H", data, offset + 4)[0] == {"amd64": 0x8664, "arm64": 0xAA64}[arch]
        if path.name == "mailfabrica.exe":
            optional = offset + 24
            assert struct.unpack_from("<H", data, optional)[0] == 0x20B
            rva, size = struct.unpack_from("<II", data, optional + 112 + 16)
            assert size > 0
            count = struct.unpack_from("<H", data, offset + 6)[0]
            assert count <= 96
            sections = optional + struct.unpack_from("<H", data, offset + 20)[0]
            root = None
            for index in range(count):
                virtual_size, virtual, raw_size, raw = struct.unpack_from("<IIII", data, sections + index * 40 + 8)
                if virtual <= rva < virtual + max(virtual_size, raw_size):
                    root = raw + rva - virtual
                    break
            assert root is not None
            named, identifiers = struct.unpack_from("<HH", data, root + 12)
            assert named + identifiers <= 256
            kinds = {struct.unpack_from("<I", data, root + 16 + index * 8)[0] for index in range(named + identifiers)}
            assert {3, 14, 16}.issubset(kinds), "Missing PE icon or version resources"
    else:
        assert data[:4] == b"\x7fELF" and data[4:6] == b"\x02\x01"
        assert struct.unpack_from("<H", data, 18)[0] == {"amd64": 62, "arm64": 183}[arch]


def main():
    """校验归档及包内哈希，运行 CLI，并从真实桌面程序获取两张截图。"""
    parser = argparse.ArgumentParser()
    parser.add_argument("platform", choices=["windows", "linux"])
    parser.add_argument("arch", choices=["amd64", "arm64"])
    args = parser.parse_args()
    extension = "zip" if args.platform == "windows" else "tar.gz"
    archives = list(Path("dist").glob(f"*/MailFabrica-*-{args.platform}-{args.arch}.{extension}"))
    assert len(archives) == 1, archives
    archive = archives[0]
    record = next(archive.parent.glob("*.json"))
    manifest = json.loads(record.read_text(encoding="utf-8"))
    assert (manifest["platform"], manifest["arch"]) == (args.platform, args.arch)
    assert not manifest["dirty"]
    assert manifest["revision"] == subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
    assert digest(archive) == next(archive.parent.glob("*.sha256")).read_text().split()[0]
    screenshots = Path("dist/qa").resolve()
    screenshots.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="mailfabrica-update-qa-") as temporary:
        unpack(archive, Path(temporary))
        roots = list(Path(temporary).iterdir())
        assert len(roots) == 1
        payload = roots[0]
        for filename, expected in manifest["sha256"].items():
            path = payload / filename
            assert digest(path) == expected
            architecture(path, args.platform, args.arch)
        suffix = ".exe" if args.platform == "windows" else ""
        gui, cli = payload / f"mailfabrica{suffix}", payload / f"mailfabrica-cli{suffix}"
        if args.platform == "linux":
            for path in [gui, cli]:
                result = subprocess.run(["ldd", str(path)], text=True, capture_output=True, check=True)
                assert "not found" not in result.stdout, result.stdout
        version = subprocess.check_output([str(gui), "--version"], text=True, encoding="utf-8", timeout=30).strip()
        assert version == f"MailFabrica {manifest['version']}"
        response = subprocess.check_output([str(cli), "--demo", "list", "--limit", "2", "--json"], text=True, encoding="utf-8", timeout=60)
        assert json.loads(response)
        with socket.socket() as reserve:
            reserve.bind(("127.0.0.1", 0))
            port = reserve.getsockname()[1]
        log_path = screenshots / f"{args.platform}-{args.arch}.log"
        with log_path.open("wb") as log:
            process = subprocess.Popen([str(gui), "--demo", "--control", str(port)], stdout=log, stderr=log)
            try:
                deadline = time.monotonic() + 120
                while True:
                    if process.poll() is not None:
                        raise RuntimeError(log_path.read_text(errors="replace"))
                    try:
                        connection = socket.create_connection(("127.0.0.1", port), timeout=1)
                        break
                    except OSError:
                        if time.monotonic() >= deadline:
                            raise TimeoutError("GUI control server did not start")
                        time.sleep(0.5)
                with connection:
                    connection.settimeout(15)
                    stream = connection.makefile("rb")
                    sequence = 0
                    def call(method, params=None):
                        """通过应用控制通道发起真实界面请求，显式检查错误。"""
                        nonlocal sequence
                        sequence += 1
                        connection.sendall((json.dumps({"id": sequence, "method": method, "params": params or {}}) + "\n").encode())
                        result = json.loads(stream.readline())
                        if not result.get("ok"):
                            raise RuntimeError(result)
                        return result.get("result")
                    time.sleep(2)
                    inspection = call("ui.inspect")
                    assert inspection["messages"] > 0
                    for name in ["main", "updates"]:
                        if name == "updates":
                            call("engine.execute", {"command": "app.dialog", "params": {"name": "updates"}})
                        path = screenshots / f"{args.platform}-{args.arch}-{name}.png"
                        for attempt in range(20):
                            try:
                                call("ui.screenshot", {"path": str(path)})
                                break
                            except RuntimeError:
                                if attempt == 19:
                                    raise
                                time.sleep(0.5)
                        assert path.stat().st_size > 1000
                    call("app.quit")
                process.wait(timeout=20)
                assert process.returncode == 0
            finally:
                if process.poll() is None:
                    process.terminate()
                    process.wait(timeout=20)
        result = subprocess.run(["cargo", "xtask", "update-qa", str(archive.resolve()), manifest["version"], str(gui), str(port)],
                                text=True, encoding="utf-8", capture_output=True, timeout=180)
        (screenshots / f"{args.platform}-{args.arch}-automatic-update.log").write_text(result.stdout + result.stderr, encoding="utf-8")
        assert result.returncode == 0, result.stdout + result.stderr
    print(f"VERIFIED: {archive.name}; native architecture, hashes, CLI, GUI screenshots and automatic replacement")


if __name__ == "__main__":
    main()
