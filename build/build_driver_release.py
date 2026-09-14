#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
build_driver_release.py — 编译 AnyKey 内核过滤驱动并部署到 deploy/ 发行目录。

流程：
    1. 调用 anykey-filter-driver/build_driver.bat（cl/link + 测试证书签名）
       bat 自己探测 WDK/KMDF/VS/证书，并把最终使用的证书指纹写进
       BIN\\X64\\RELEASE\\signing_thumbprint.txt
    2. 部署 BIN\\X64\\RELEASE\\ANYKEY_FLT.SYS -> deploy/anykey_flt.sys（原子替换）
    3. 按该指纹从证书存储导出证书 -> deploy/anykey_flt.cer
       （找不到时保留现有文件并告警）

用法：
    python build_driver_release.py           # 编译 + 部署
    python build_driver_release.py --check   # 仅检查产物/部署文件是否存在，不编译

依赖：
    - VS2022（任意版本）+ WDK + KMDF：均由 build_driver.bat 自动探测，可用
      WKROOT / ANYKEY_WDK_VERSION / ANYKEY_KMDF_VERSION / ANYKEY_VCVARS /
      ANYKEY_SIGN_THUMBPRINT 环境变量覆盖
    - build_driver.bat 退出码：0=成功；1=探测/编译/链接失败；2=编译成功但签名失败（不部署）
"""
import os
import sys
import subprocess

HERE = os.path.dirname(os.path.abspath(__file__))
DRIVER_DIR = os.path.join(HERE, "..", "anykey-filter-driver")
BUILD_BAT = os.path.join(DRIVER_DIR, "build_driver.bat")
BIN_DIR = os.path.join(DRIVER_DIR, "BIN", "X64", "RELEASE")
SRC_SYS = os.path.join(BIN_DIR, "ANYKEY_FLT.SYS")

# build_driver.bat 把实际使用的证书指纹写在这里，避免指纹在两处各存一份
THUMB_FILE = os.path.join(BIN_DIR, "signing_thumbprint.txt")

DEPLOY_DIR = os.path.join(DRIVER_DIR, "deploy")
DEPLOY_SYS = os.path.join(DEPLOY_DIR, "anykey_flt.sys")
DEPLOY_CER = os.path.join(DEPLOY_DIR, "anykey_flt.cer")


def _fmt_size(n):
    if n >= 1024 * 1024:
        return "%.2f MiB" % (n / 1024.0 / 1024.0)
    return "%.1f KiB" % (n / 1024.0)


def read_thumbprint():
    """读取 build_driver.bat 记录的签名证书指纹；缺失时返回 None。"""
    try:
        with open(THUMB_FILE, "r", encoding="utf-8", errors="replace") as f:
            tp = f.read().strip()
        return tp or None
    except OSError:
        return None


def check_only():
    tp = read_thumbprint()
    print("编译产物 (%s): %s" % (SRC_SYS, os.path.exists(SRC_SYS)))
    print("部署 .sys   (%s): %s" % (DEPLOY_SYS, os.path.exists(DEPLOY_SYS)))
    print("部署 .cer   (%s): %s" % (DEPLOY_CER, os.path.exists(DEPLOY_CER)))
    print("签名指纹    (%s): %s" % (THUMB_FILE, tp or "（缺失，需先构建）"))


def export_cert(thumbprint):
    """按指纹定位证书所在存储并导出到 deploy/anykey_flt.cer。"""
    # 注意：路径拼接用 Test-Path/Get-Item，避免 Get-ChildItem 传多路径时
    # 第二个路径被绑定成 -Filter（证书提供程序不支持筛选器）。
    ps = (
        "$ErrorActionPreference='Stop';"
        "$tp='%s';$out='%s';$done=$false;"
        "foreach ($s in @('CurrentUser\\My','LocalMachine\\My')) {"
        "  if (Test-Path ('Cert:\\' + $s + '\\' + $tp)) {"
        "    Export-Certificate -Cert (Get-Item ('Cert:\\' + $s + '\\' + $tp)) "
        "-Type CERT -FilePath $out | Out-Null;"
        "    Write-Output ('EXPORTED ' + $s);$done=$true;break"
        "  }"
        "}"
        "if (-not $done) { Write-Output 'NOTFOUND' }" % (thumbprint, DEPLOY_CER)
    )
    r = subprocess.run(
        ["powershell", "-NoProfile", "-Command", ps],
        capture_output=True, text=True, errors="replace",
    )
    out = (r.stdout or "").strip()
    if out.startswith("EXPORTED"):
        print("[OK] 已导出签名证书（%s）-> %s" % (out.split(" ", 1)[1], DEPLOY_CER))
    else:
        print("[WARN] 证书存储中找不到指纹 %s，保留现有 .cer" % thumbprint)
        if not os.path.exists(DEPLOY_CER):
            print("[ERROR] deploy/anykey_flt.cer 不存在，驱动发布包需要它。", file=sys.stderr)
            sys.exit(1)


def deploy(thumbprint):
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

    if thumbprint:
        export_cert(thumbprint)
    else:
        print("[WARN] 未找到 %s，跳过证书导出（构建脚本可能未成功运行）。" % THUMB_FILE)

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

    # 每次构建前清掉旧记录，避免读到上一次的指纹
    try:
        if os.path.exists(THUMB_FILE):
            os.remove(THUMB_FILE)
    except OSError:
        pass

    print("驱动源码目录 : %s" % DRIVER_DIR)
    print("部署目标     : %s" % DEPLOY_SYS)

    print("\n=== 调用 build_driver.bat（cl/link + 测试签名）===")
    r = subprocess.run(["cmd", "/c", BUILD_BAT], cwd=DRIVER_DIR)
    if r.returncode == 2:
        print("[ERROR] 驱动编译成功但测试签名失败（exit 2），未部署。", file=sys.stderr)
        tp = read_thumbprint()
        if tp:
            print("        使用的证书指纹: %s" % tp, file=sys.stderr)
        print("        可用 ANYKEY_SIGN_THUMBPRINT 指定其它证书，或不设置该变量"
              "让构建脚本自动创建测试证书。", file=sys.stderr)
        sys.exit(2)
    if r.returncode != 0:
        print("[ERROR] 驱动编译失败 (exit %d)。" % r.returncode, file=sys.stderr)
        sys.exit(r.returncode)

    deploy(read_thumbprint())


if __name__ == "__main__":
    main()
