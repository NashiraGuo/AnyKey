#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
build_engine_release.py — 编译 AnyKey Rust 引擎并部署到 GUI 的 engines/rust/ 目录。

用法：
    python build_engine_release.py            # 编译 release 并部署
    python build_engine_release.py --test      # 先跑 cargo test，通过后再部署
    python build_engine_release.py --check     # 仅检查目录/产物是否存在，不编译

依赖：
    - cargo 必须在 PATH 中（rustup 默认安装时会加入用户 PATH）
    - Python 3（项目已用管理版 Python）

部署目标（与 config_manager.RUST_ENGINE_PATH 一致）：
    <本脚本所在目录>/engines/rust/anykey-engine.exe
"""
import os
import sys
import shutil
import subprocess

HERE = os.path.dirname(os.path.abspath(__file__))
ENGINE_DIR = os.path.join(HERE, "..", "anykey-engine")
SRC_EXE = os.path.join(ENGINE_DIR, "target", "release", "anykey-engine.exe")
DEPLOY_DIR = os.path.join(HERE, "..", "engines", "rust")
DEPLOY_EXE = os.path.join(DEPLOY_DIR, "anykey-engine.exe")


def _which(cmd):
    from shutil import which
    return which(cmd) is not None


def _run(cmd, cwd):
    print(">>> " + " ".join(cmd))
    r = subprocess.run(cmd, cwd=cwd)
    if r.returncode != 0:
        print("[ERROR] 命令失败 (exit %d): %s" % (r.returncode, " ".join(cmd)), file=sys.stderr)
        sys.exit(r.returncode)
    return r.returncode


def _fmt_size(n):
    if n >= 1024 * 1024:
        return "%.2f MiB" % (n / 1024.0 / 1024.0)
    return "%.1f KiB" % (n / 1024.0)


def main():
    # 统一输出编码为 UTF-8，避免在非 UTF-8 控制台（如默认 cmd）下中文乱码
    try:
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
        sys.stderr.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass

    args = sys.argv[1:]
    do_test = "--test" in args
    check_only = "--check" in args

    print("引擎源码目录 : %s" % ENGINE_DIR)
    print("部署目标     : %s" % DEPLOY_EXE)

    if check_only:
        print("  engines/rust 存在 : %s" % os.path.isdir(DEPLOY_DIR))
        print("  release exe 存在  : %s" % os.path.exists(SRC_EXE))
        return

    if not _which("cargo"):
        print("[ERROR] 找不到 cargo，请确认 Rust 已安装且 cargo 在 PATH 中。", file=sys.stderr)
        print("        安装: https://rustup.rs/  （默认会把 cargo 加入用户 PATH）", file=sys.stderr)
        sys.exit(1)

    if not os.path.isdir(ENGINE_DIR):
        print("[ERROR] 找不到引擎目录: %s" % ENGINE_DIR, file=sys.stderr)
        sys.exit(1)

    if do_test:
        print("\n=== 运行 cargo test ===")
        _run(["cargo", "test"], cwd=ENGINE_DIR)

    print("\n=== 编译 release ===")
    _run(["cargo", "build", "--release"], cwd=ENGINE_DIR)

    if not os.path.exists(SRC_EXE):
        print("[ERROR] 编译产物缺失: %s" % SRC_EXE, file=sys.stderr)
        sys.exit(1)

    os.makedirs(DEPLOY_DIR, exist_ok=True)

    # 健壮部署：先复制到临时名再原子替换；若目标正被占用（如运行的 GUI 加载了旧引擎）
    # 则给出清晰提示，而不是抛出原始 traceback。
    tmp_exe = DEPLOY_EXE + ".new"
    try:
        if os.path.exists(DEPLOY_EXE):
            # 探测目标是否被占用（独占打开）
            try:
                _fd = os.open(DEPLOY_EXE, os.O_RDWR | os.O_EXCL)
                os.close(_fd)
            except OSError as e:
                if e.errno in (13, 32):  # EACCES / ERROR_SHARING_VIOLATION
                    print("[错误] 无法覆盖部署目标: %s" % DEPLOY_EXE, file=sys.stderr)
                    print("        该文件正被其他进程占用（通常是正在运行的 AnyKey GUI 加载了旧引擎）。", file=sys.stderr)
                    print("        请先关闭 AnyKey GUI，或在任务管理器结束 anykey-engine.exe，再重新运行本脚本。", file=sys.stderr)
                    sys.exit(1)
                raise
        shutil.copy2(SRC_EXE, tmp_exe)
        if os.path.exists(DEPLOY_EXE):
            os.remove(DEPLOY_EXE)
        os.replace(tmp_exe, DEPLOY_EXE)
    except OSError as e:
        print("[错误] 部署复制失败: %s" % DEPLOY_EXE, file=sys.stderr)
        print("        %s (errno=%s)" % (e, getattr(e, "winerror", e.errno)), file=sys.stderr)
        print("        若提示文件被占用，请先关闭 AnyKey GUI 后重试。", file=sys.stderr)
        if os.path.exists(tmp_exe):
            try:
                os.remove(tmp_exe)
            except OSError:
                pass
        sys.exit(1)

    print("\n[OK] 已部署: %s (%s)" % (DEPLOY_EXE, _fmt_size(os.path.getsize(DEPLOY_EXE))))
    if do_test:
        print("      （已通过 cargo test）")
    print("      下次启动 GUI 会自动加载此引擎。")


if __name__ == "__main__":
    main()
