"""核对四种原生包后上传到草稿 Release；不覆盖既有资产或自动公开发布。"""
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import tomllib


def run(*arguments):
    """用参数数组调用 Git/GitHub CLI，错误保留原始诊断但不打印凭据。"""
    return subprocess.check_output(arguments, text=True, encoding="utf-8").strip()


def digest(path):
    """校验最终上传的文件内容。"""
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main():
    """确保版本、标签、源码、架构与归档校验一致，重复运行时只复用完全相同的资产。"""
    version = tomllib.loads(Path("Cargo.toml").read_text(encoding="utf-8"))["workspace"]["package"]["version"]
    assert "-dev" not in version
    tag = f"v{version}"
    revision = run("git", "rev-parse", "HEAD")
    assert run("git", "rev-parse", f"{tag}^{{commit}}") == revision
    subprocess.run(["git", "merge-base", "--is-ancestor", tag, "origin/master"], check=True)
    repository = "OpenFabrica/mailfabrica"
    files = []
    for platform in ["windows", "linux"]:
        for arch in ["amd64", "arm64"]:
            name = f"MailFabrica-{version}-{platform}-{arch}"
            extension = "zip" if platform == "windows" else "tar.gz"
            matches = list(Path("dist/artifacts").rglob(f"{name}.{extension}"))
            assert len(matches) == 1, matches
            archive = matches[0]
            record, checksum = archive.parent / f"{name}.json", archive.parent / f"{name}.sha256"
            manifest = json.loads(record.read_text(encoding="utf-8"))
            assert manifest["version"] == version and manifest["revision"] == revision and not manifest["dirty"]
            assert (manifest["platform"], manifest["arch"]) == (platform, arch)
            assert digest(archive) == checksum.read_text().split()[0]
            files.extend([archive, record, checksum])
    lookup = subprocess.run(["gh", "release", "view", tag, "--repo", repository, "--json", "isDraft,assets"], text=True, encoding="utf-8", capture_output=True)
    if lookup.returncode:
        with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8", suffix=".md") as notes:
            notes.write(f"MailFabrica {version}\n\n跨平台发布资产已构建并通过原生测试。完整说明与 macOS 公证包待发布前统一核验。\n")
            notes.flush()
            run("gh", "release", "create", tag, "--repo", repository, "--verify-tag", "--draft", "--title", f"MailFabrica {version}", "--notes-file", notes.name)
        release = json.loads(run("gh", "release", "view", tag, "--repo", repository, "--json", "isDraft,assets"))
    else:
        release = json.loads(lookup.stdout)
    assert release["isDraft"], "Refusing to change a published release"
    existing = {asset["name"]: asset for asset in release["assets"]}
    for path in files:
        expected = f"sha256:{digest(path)}"
        asset = existing.get(path.name)
        if asset is not None:
            assert asset.get("digest") == expected and asset["size"] == path.stat().st_size, "Existing release asset differs"
        else:
            run("gh", "release", "upload", tag, str(path), "--repo", repository)
    release = json.loads(run("gh", "release", "view", tag, "--repo", repository, "--json", "isDraft,assets"))
    assert release["isDraft"]
    for path in files:
        asset = next(asset for asset in release["assets"] if asset["name"] == path.name)
        assert asset.get("digest") == f"sha256:{digest(path)}"
    print(f"VERIFIED: uploaded {len(files)} immutable assets to draft {tag}")


if __name__ == "__main__":
    main()
