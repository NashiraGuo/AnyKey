#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
build_tray_release.py — 编译 AnyKey Rust 托盘并部署到安装目录。

用法：
    python build_tray_release.py            # 编译 release 并部署到 dist/AnyKey/
    python build_tray_release.py --build-only  # 只编译，不部署
    python build_tray_release.py --check       # 仅检查编译产物是否存在

依赖：
    - cargo 必须在 PATH 中（与 build_engine_release.py 相同）
"""
import os
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
TRAY_DIR = os.path.join(HERE, "..", "anykey-tray")
SRC_EXE = os.path.join(TRAY_DIR, "target", "release", "anykey-tray.exe")
DEPLOY_DIR = os.path.join(HERE, "dist", "AnyKey")
DEPLOY_EXE = os.path.join(DEPLOY_DIR, "anykey-tray.exe")


def _which(cmd):
    return shutil.which(cmd) is not None


def main():
    # 统一输出编码为 UTF-8，避免非 UTF-8 控制台中文乱码
    try:
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
        sys.stderr.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass

    args = sys.argv[1:]
    build_only = "--build-only" in args
    check_only = "--check" in args

    print("托盘源码目录 : %s" % TRAY_DIR)
    print("部署目标     : %s" % DEPLOY_EXE)

    if check_only:
        print("  release exe 存在 : %s" % os.path.exists(SRC_EXE))
        return

    if not _which("cargo"):
        print("[ERROR] 找不到 cargo，请确认 Rust 已安装且 cargo 在 PATH 中。", file=sys.stderr)
        sys.exit(1)
    if not os.path.isdir(TRAY_DIR):
        print("[ERROR] 找不到托盘目录: %s" % TRAY_DIR, file=sys.stderr)
        sys.exit(1)

    # 先结束运行中的托盘进程，避免 exe 被占用无法覆盖
    _CNW = 0x08000000
    subprocess.run(["taskkill", "/f", "/im", "anykey-tray.exe"],
                   capture_output=True, creationflags=_CNW)

    print("\n=== 编译 release ===")
    r = subprocess.run(["cargo", "build", "--release"], cwd=TRAY_DIR)
    if r.returncode != 0:
        sys.exit(r.returncode)

    if not os.path.exists(SRC_EXE):
        print("[ERROR] 编译产物缺失: %s" % SRC_EXE, file=sys.stderr)
        sys.exit(1)

    if build_only:
        print("\n[OK] 编译完成（未部署）: %s (%.2f KiB)" % (SRC_EXE, os.path.getsize(SRC_EXE) / 1024.0))
        return

    os.makedirs(DEPLOY_DIR, exist_ok=True)
    tmp_exe = DEPLOY_EXE + ".new"
    try:
        shutil.copy2(SRC_EXE, tmp_exe)
        if os.path.exists(DEPLOY_EXE):
            os.remove(DEPLOY_EXE)
        os.replace(tmp_exe, DEPLOY_EXE)
    except OSError as e:
        print("[错误] 部署复制失败: %s (%s)" % (DEPLOY_EXE, e), file=sys.stderr)
        if os.path.exists(tmp_exe):
            os.remove(tmp_exe)
        sys.exit(1)

    print("\n[OK] 已部署: %s (%.2f KiB)" % (DEPLOY_EXE, os.path.getsize(DEPLOY_EXE) / 1024.0))
    print("      下次启动 GUI 会自动加载 Rust 托盘。")


if __name__ == "__main__":
    main()
