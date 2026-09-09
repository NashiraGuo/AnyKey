#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
build_release_package.py — 组装 AnyKey 发行标准目录并压缩为 zip。

产出 release/ 结构（zip 根即此三项）：
    release/
    ├── anykey/                  ← dist/AnyKey 全部 + 空白 anykey_config.json 模板
    ├── anykeyFilterDriver/      ← 驱动分发（deploy 白名单文件）
    └── 安装驱动.bat             ← 相对路径启动器（call anykeyFilterDriver\\Install_AnyKey_Filter.bat）

用法：
    python build_release_package.py                # 组装目录，不压缩
    python build_release_package.py --zip          # 组装 + 压缩为 AnyKey_v<版本>.zip
    python build_release_package.py --zip 1.0.0    # 指定版本号

依赖：build.bat 先跑完（dist/AnyKey 与 deploy 驱动文件须存在）。
zip 输出到 <build>/AnyKey_v<版本>.zip；release/ 保留为未压缩标准目录。
"""
import os
import sys
import shutil
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
DIST_DIR = os.path.join(HERE, "dist", "AnyKey")
DEPLOY_DIR = os.path.join(REPO, "anykey-filter-driver", "deploy")
RELEASE_DIR = os.path.join(HERE, "release")

# 驱动分发白名单：只带干净文件，排除 .log / .bak / bin 残留
DRIVER_FILES = [
    "anykey_flt.sys",
    "anykey_flt.inf",
    "anykey_flt_mouse.inf",
    "anykey_flt.cer",
    "Install_AnyKey_Filter.bat",
    "Uninstall_AnyKey_Filter.bat",
    "_install_anykey_device.ps1",
    "_uninstall.ps1",
]

LAUNCHER_BAT = (
    "@echo off\r\n"
    ":: AnyKey - Install Filter Driver (launcher)\r\n"
    ":: Double-click to run. UAC elevation is handled by the target script.\r\n"
    'call "%~dp0anykeyFilterDriver\\Install_AnyKey_Filter.bat"\r\n'
)


def _die(msg):
    print("[ERROR] %s" % msg, file=sys.stderr)
    sys.exit(1)


def assemble():
    if not os.path.isdir(DIST_DIR):
        _die("dist/AnyKey 不存在，请先运行 build.bat 完成构建。")
    for name in DRIVER_FILES:
        if not os.path.exists(os.path.join(DEPLOY_DIR, name)):
            _die("驱动文件缺失: %s（请确认 deploy 目录完整）" % name)

    if os.path.isdir(RELEASE_DIR):
        shutil.rmtree(RELEASE_DIR)

    # 1. anykey/：GUI 产物 + 空白配置模板
    anykey_dir = os.path.join(RELEASE_DIR, "anykey")
    shutil.copytree(DIST_DIR, anykey_dir)
    with open(os.path.join(anykey_dir, "anykey_config.json"), "w", encoding="utf-8") as f:
        f.write("{}")

    # 2. anykeyFilterDriver/：白名单复制
    driver_dir = os.path.join(RELEASE_DIR, "anykeyFilterDriver")
    os.makedirs(driver_dir)
    for name in DRIVER_FILES:
        shutil.copy2(os.path.join(DEPLOY_DIR, name), os.path.join(driver_dir, name))

    # 3. 根目录启动器（CRLF）
    with open(os.path.join(RELEASE_DIR, "安装驱动.bat"), "wb") as f:
        f.write(LAUNCHER_BAT.encode("ascii"))

    print("[OK] 已组装 %s" % RELEASE_DIR)


def make_zip(version):
    if not os.path.isdir(RELEASE_DIR):
        _die("release/ 不存在，请先组装。")
    zip_path = os.path.join(HERE, "AnyKey_v%s.zip" % version)
    if os.path.exists(zip_path):
        os.remove(zip_path)
    with zipfile.ZipFile(zip_path, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as zf:
        for root, _dirs, files in os.walk(RELEASE_DIR):
            for name in files:
                full = os.path.join(root, name)
                rel = os.path.relpath(full, RELEASE_DIR)
                zf.write(full, rel)
    size = os.path.getsize(zip_path)
    print("[OK] 已生成 %s (%.1f MB)" % (zip_path, size / 1024.0 / 1024.0))


def main():
    try:
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
        sys.stderr.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass

    args = [a for a in sys.argv[1:]]
    do_zip = "--zip" in args
    rest = [a for a in args if not a.startswith("--")]
    version = rest[0] if rest else "dev"

    assemble()
    if do_zip:
        make_zip(version)


if __name__ == "__main__":
    main()
