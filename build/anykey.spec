# -*- mode: python ; coding: utf-8 -*-
"""PyInstaller spec — 仅打包 GUI，输出到 dist/AnyKey/
Rust 组件（anykey-engine / anykey-tray）由 build.bat 中的 cargo 构建。
"""
from PyInstaller.utils.hooks import collect_all

# ── Common data for both programs ──

gui_datas = [
    ('../assets/help.md', '.'),
    ('../assets/icon.ico', '.'),
    ('../assets/icon.png', '.'),
]
hiddenimports = ['winreg', 'app_bar']

tmp_ret = collect_all('customtkinter')
gui_datas += tmp_ret[0]
gui_binaries = list(tmp_ret[1])
hiddenimports += tmp_ret[2]

# ── GUI Analysis ──

a_gui = Analysis(
    ['../gui/main.py'],
    pathex=['../gui'],
    binaries=gui_binaries,
    datas=gui_datas,
    hiddenimports=hiddenimports,
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    excludes=['unittest', 'test', 'doctest', 'pydoc', 'ensurepip', 'turtledemo'],
    noarchive=False,
    optimize=0,
)
pyz_gui = PYZ(a_gui.pure)

exe_gui = EXE(
    pyz_gui,
    a_gui.scripts,
    [],
    exclude_binaries=True,
    name='anykey-gui',
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=False,
    console=False,
    disable_windowed_traceback=False,
    argv_emulation=False,
    target_arch=None,
    codesign_identity=None,
    entitlements_file=None,
    icon=['../assets/icon.ico'],
)

coll = COLLECT(
    exe_gui,
    a_gui.binaries,
    a_gui.zipfiles,
    a_gui.datas,
    strip=False,
    upx=False,
    upx_exclude=[],
    name='AnyKey',
)
