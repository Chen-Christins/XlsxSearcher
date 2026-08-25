"""应用自更新 - 从 GitHub Releases 检查、下载并安装新版本。

纯标准库实现（urllib + json + zipfile），避免给打包引入额外依赖。
更新源为 GitHub Releases：走列表接口过滤 draft，仅认已发布的正式版本。
"""
import json
import os
import re
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request
import zipfile

GITHUB_API = "https://api.github.com"
DEFAULT_REPO = "Chen-Christins/XlsxSearcher"
_USER_AGENT = "XlsxSearcher-Updater"


def parse_version(version):
    """把 'v1.4.4' / '1.4.4' / '1.4.4-beta1' 解析为 (major, minor, patch) 元组。"""
    if not version:
        return (0, 0, 0)
    cleaned = str(version).strip().lstrip("vV")
    parts = re.split(r"[.\-+]", cleaned)
    nums = []
    for part in parts:
        match = re.match(r"(\d+)", part)
        nums.append(int(match.group(1)) if match else 0)
    while len(nums) < 3:
        nums.append(0)
    return tuple(nums[:3])


def compare_versions(a, b):
    """比较两个版本号，返回 1 / 0 / -1。"""
    a_t, b_t = parse_version(a), parse_version(b)
    return (a_t > b_t) - (a_t < b_t)


def _api_get(url, token=None, timeout=15):
    """GET 一个 GitHub API 端点，返回解析后的 JSON。"""
    headers = {
        "User-Agent": _USER_AGENT,
        "Accept": "application/vnd.github+json",
    }
    if token:
        headers["Authorization"] = f"token {token}"
    request = urllib.request.Request(url, headers=headers)
    with urllib.request.urlopen(request, timeout=timeout) as resp:
        return json.loads(resp.read().decode("utf-8"))


def get_latest_release(repo=DEFAULT_REPO, prerelease=False, token=None):
    """返回最新一个已发布（非 draft）release，无可用版本时返回 None。

    用列表接口而非 /releases/latest：后者只认已发布的最新版，但这里需要
    在保留 draft 审批流程的前提下，过滤 draft 后取最高版本。
    """
    url = f"{GITHUB_API}/repos/{repo}/releases?per_page=30"
    releases = _api_get(url, token=token)

    candidates = []
    for rel in releases:
        if rel.get("draft"):
            continue
        if rel.get("prerelease") and not prerelease:
            continue
        tag = rel.get("tag_name", "")
        if not tag:
            continue
        candidates.append((parse_version(tag), rel))

    if not candidates:
        return None
    candidates.sort(key=lambda item: item[0], reverse=True)
    return candidates[0][1]


def pick_asset(release):
    """按当前平台从 release 资产里挑出对应 zip。"""
    platform_token = {
        "darwin": "macos",
        "win32": "windows",
        "linux": "linux",
    }.get(sys.platform, "")
    if not platform_token:
        return None

    for asset in release.get("assets", []):
        name = asset.get("name", "")
        if name.lower().endswith(".zip") and platform_token in name.lower():
            return asset
    return None


def download(url, dest, progress_cb=None, token=None):
    """流式下载 url 到 dest，progress_cb(downloaded, total)。"""
    headers = {
        "User-Agent": _USER_AGENT,
        "Accept": "application/octet-stream",
    }
    if token:
        headers["Authorization"] = f"token {token}"
    request = urllib.request.Request(url, headers=headers)

    with urllib.request.urlopen(request, timeout=30) as resp:
        content_length = resp.headers.get("Content-Length")
        total = int(content_length) if content_length else None
        downloaded = 0
        with open(dest, "wb") as f:
            while True:
                chunk = resp.read(256 * 1024)
                if not chunk:
                    break
                f.write(chunk)
                downloaded += len(chunk)
                if progress_cb:
                    progress_cb(downloaded, total)


def extract_zip(zip_path, extract_dir):
    """解压 zip 到目标目录。"""
    with zipfile.ZipFile(zip_path) as zf:
        zf.extractall(extract_dir)


def locate_new_executable(extracted_dir):
    """在解压目录里定位新版本：macOS 返回 .app，其余返回可执行文件路径。"""
    if sys.platform == "darwin":
        for name in os.listdir(extracted_dir):
            if name.lower().endswith(".app"):
                return os.path.join(extracted_dir, name)
        return None

    exe_name = "XlsxSearcher.exe" if sys.platform == "win32" else "XlsxSearcher"
    for root, _dirs, files in os.walk(extracted_dir):
        for f in files:
            if f.lower() == exe_name.lower():
                return os.path.join(root, f)
    return None


def get_current_install_path():
    """返回当前安装位置：macOS 为 .app 目录，其余为可执行文件路径。

    非打包环境（非 frozen）返回 None，跳过自更新。
    """
    if not getattr(sys, "frozen", False):
        return None

    exe = sys.executable
    if sys.platform == "darwin":
        # .../XlsxSearcher.app/Contents/MacOS/XlsxSearcher -> .app
        bundle = exe
        for _ in range(3):
            bundle = os.path.dirname(bundle)
        return bundle
    return exe


def write_replace_helper(pid, new_path, target_path):
    """生成自包含的替换脚本，返回脚本路径。

    脚本逻辑：等待当前进程退出 -> 覆盖安装 -> 重新启动 -> 自删。
    路径直接内嵌，避免命令行参数空格转义的坑。
    """
    tmpdir = tempfile.mkdtemp(prefix="xlsxsearcher_update_")

    if sys.platform == "win32":
        helper_path = os.path.join(tmpdir, "replace.cmd")
        content = (
            "@echo off\r\n"
            ":loop\r\n"
            "timeout /t 1 /nobreak >nul\r\n"
            f'tasklist /FI "PID eq {pid}" 2>nul | find "{pid}" >nul\r\n'
            "if errorlevel 1 goto replace\r\n"
            "goto loop\r\n"
            ":replace\r\n"
            f'move /Y "{new_path}" "{target_path}"\r\n'
            f'start "" "{target_path}"\r\n'
            'del "%~f0"\r\n'
        )
        with open(helper_path, "w", encoding="utf-8", newline="\r\n") as f:
            f.write(content)
        return helper_path

    helper_path = os.path.join(tmpdir, "replace.sh")
    content = (
        "#!/bin/bash\n"
        f'while kill -0 "{pid}" 2>/dev/null; do sleep 1; done\n'
    )
    if sys.platform == "darwin":
        content += (
            f'rm -rf "{target_path}"\n'
            f'mv "{new_path}" "{target_path}"\n'
            f'open "{target_path}"\n'
        )
    else:
        content += (
            f'mv -f "{new_path}" "{target_path}"\n'
            f'chmod +x "{target_path}"\n'
            f'"{target_path}" &\n'
        )
    with open(helper_path, "w", encoding="utf-8") as f:
        f.write(content)
    return helper_path


def launch_replace(helper_path):
    """在独立会话里启动替换脚本。"""
    if sys.platform == "win32":
        DETACHED_PROCESS = 0x00000008
        subprocess.Popen(
            ["cmd", "/c", helper_path],
            creationflags=DETACHED_PROCESS,
            close_fds=True,
        )
    else:
        os.chmod(helper_path, 0o755)
        subprocess.Popen(
            [helper_path],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            start_new_session=True,
        )
