#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
build_driver_release.py — 编译 AnyKey 内核过滤驱动并部署到 deploy/ 发行目录。

流程：
    1. 调用 anykey-filter-driver/build_driver.bat（cl/link + 测试证书签名）
    2. 部署 BIN\\X64\\RELEASE\\ANYKEY_FLT.SYS -> deploy/anykey_flt.sys（原子替换）
    3. 从证书存储导出测试证书 -> deploy/anykey_flt.cer（找不到证书时保留现有文件并告警）

用法：
    python build_driver_release.py           # 编译 + 部署
    python build_driver_release.py --check   # 仅检查产物/部署文件是否存在，不编译

依赖：
    - VS2022 Community + Windows Kits 10.0.28000.0 + KMDF 1.15（build_driver.bat 内部处理）
    - build_driver.bat 退出码：0=成功；1=编译/链接失败；2=编译成功但签名失败（不部署）
"""
import os
import sys
import subprocess

HERE = os.path.dirname(os.path.abspath(__file__))
DRIVER_DIR = os.path.join(HERE, "..", "anykey-filter-driver")
BUILD_BAT = os.path.join(DRIVER_DIR, "build_driver.bat")
SRC_SYS = os.path.join(DRIVER_DIR, "BIN", "X64", "RELEASE", "ANYKEY_FLT.SYS")
DEPLOY_DIR = os.path.join(DRIVER_DIR, "deploy")
DEPLOY_SYS = os.path.join(DEPLOY_DIR, "anykey_flt.sys")
DEPLOY_CER = os.path.join(DEPLOY_DIR, "anykey_flt.cer")

# 与 build_driver.bat 中 signtool /sha1 一致
CERT_THUMBPRINT = "5B1F1FD5EEBCE1880DD21D38F503A1C615EE796D"


def _fmt_size(n):
    if n >= 1024 * 1024:
        return "%.2f MiB" % (n / 1024.0 / 1024.0)
    return "%.1f KiB" % (n / 1024.0)


def check_only():
    print("编译产物 (%s): %s" % (SRC_SYS, os.path.exists(SRC_SYS)))
    print("部署 .sys   (%s): %s" % (DEPLOY_SYS, os.path.exists(DEPLOY_SYS)))
    print("部署 .cer   (%s): %s" % (DEPLOY_CER, os.path.exists(DEPLOY_CER)))


def export_cert():
    """从证书存储按指纹导出测试证书到 deploy/anykey_flt.cer。"""
    ps = (
        "$ErrorActionPreference='Stop';"
        "$c = Get-ChildItem Cert:\\CurrentUser\\My,Cert:\\LocalMachine\\My "
        "-ErrorAction SilentlyContinue | "
        "Where-Object { $_.Thumbprint -eq '%s' } | Select-Object -First 1;"
        "if ($c) { Export-Certificate -Cert $c -Type CERT -FilePath '%s' | Out-Null; 'EXPORTED' } "
        "else { 'NOTFOUND' }" % (CERT_THUMBPRINT, DEPLOY_CER)
    )
    r = subprocess.run(
        ["powershell", "-NoProfile", "-Command", ps],
        capture_output=True, text=True, errors="replace",
    )
    out = (r.stdout or "").strip()
    if "EXPORTED" in out:
        print("[OK] 已导出测试证书 -> %s" % DEPLOY_CER)
    else:
        print("[WARN] 证书存储中找不到指纹 %s 的测试证书，保留现有 .cer" % CERT_THUMBPRINT)
        if not os.path.exists(DEPLOY_CER):
            print("[ERROR] deploy/anykey_flt.cer 不存在，驱动安装脚本需要它。", file=sys.stderr)
            sys.exit(1)


def deploy():
    if not os.path.exists(SRC_SYS):
        print("[ERROR] 编译产物缺失: %s" % SRC_SYS, file=sys.stderr)
        sys.exit(1)

    # 原子替换：先写临时名再 replace
    tmp_sys = DEPLOY_SYS + ".new"
    try:
        import shutil
        shutil.copy2(SRC_SYS, tmp_sys)
        if os.path.exists(DEPLOY_SYS):
            os.remove(DEPLOY_SYS)
        os.replace(tmp_sys, DEPLOY_SYS)
    except OSError as e:
        print("[ERROR] 部署复制失败: %s (%s)" % (DEPLOY_SYS, e), file=sys.stderr)
        if os.path.exists(tmp_sys):
            try:
                os.remove(tmp_sys)
            except OSError:
                pass
        sys.exit(1)

    export_cert()
    print("\n[OK] 已部署: %s (%s)" % (DEPLOY_SYS, _fmt_size(os.path.getsize(DEPLOY_SYS))))
    print("      release 打包（build_release_package.py）会从 deploy/ 白名单取用。")


def main():
    try:
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
        sys.stderr.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass

    args = sys.argv[1:]
    if "--check" in args:
        check_only()
        return

    if not os.path.exists(BUILD_BAT):
        print("[ERROR] 找不到驱动构建脚本: %s" % BUILD_BAT, file=sys.stderr)
        sys.exit(1)

    print("驱动源码目录 : %s" % DRIVER_DIR)
    print("部署目标     : %s" % DEPLOY_SYS)

    print("\n=== 调用 build_driver.bat（cl/link + 测试签名）===")
    r = subprocess.run(["cmd", "/c", BUILD_BAT], cwd=DRIVER_DIR)
    if r.returncode == 2:
        print("[ERROR] 驱动编译成功但测试签名失败（exit 2），未部署。", file=sys.stderr)
        print("        请检查证书存储中的测试证书（指纹 %s）。" % CERT_THUMBPRINT, file=sys.stderr)
        sys.exit(2)
    if r.returncode != 0:
        print("[ERROR] 驱动编译失败 (exit %d)。" % r.returncode, file=sys.stderr)
        sys.exit(r.returncode)

    deploy()


if __name__ == "__main__":
    main()
