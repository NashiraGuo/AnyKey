@echo off
title AnyKey Build

echo ==========================================
echo  AnyKey - Build Script (onedir)
echo  GUI: PyInstaller / Engine+Tray: cargo / Driver: WDK cl+link
echo ==========================================

pushd "%~dp0"

:: locate python
set PYTHON=
if exist "C:\Users\proje\AppData\Local\Programs\Python\Python312\python.exe" set PYTHON=C:\Users\proje\AppData\Local\Programs\Python\Python312\python.exe
if "%PYTHON%"=="" for %%p in (python python3) do (where %%p >nul 2>&1 && set PYTHON=%%p)
if "%PYTHON%"=="" (
    echo ERROR: Python not found
    pause
    exit /b 1
)
echo Python: %PYTHON%

:: 1. Build kernel filter driver (compile + test-sign + deploy to deploy\)
echo [1/7] Building kernel filter driver...
"%PYTHON%" build_driver_release.py
if %errorlevel% neq 0 (
    echo ====== Driver build FAILED ======
    pause
    goto :end
)

:: 2. Build Rust engine (compile + deploy to ..\engines\rust\)
echo [2/7] Building Rust engine...
"%PYTHON%" build_engine_release.py
if %errorlevel% neq 0 (
    echo ====== Engine build FAILED ======
    pause
    goto :end
)

:: clean old PyInstaller temp files
echo [3/7] Cleaning old PyInstaller build...
rmdir /s /q build 2>nul
rmdir /s /q dist  2>nul

echo [4/7] Building GUI (PyInstaller, one exe)...
"%PYTHON%" -m PyInstaller --noconfirm --clean anykey.spec
if %errorlevel% neq 0 (
    echo ====== Build FAILED ======
    pause
    goto :end
)

:: 5. Build Rust tray (compile + deploy to dist\AnyKey\anykey-tray.exe)
echo [5/7] Building Rust tray...
"%PYTHON%" build_tray_release.py
if %errorlevel% neq 0 (
    echo ====== Tray build FAILED ======
    pause
    goto :end
)

echo [6/7] Assembling install directory...
:: engine
mkdir dist\AnyKey\engines\rust  2>nul
if exist ..\engines\rust\*.exe xcopy /y /q ..\engines\rust\*.exe dist\AnyKey\engines\rust\ >nul

:: config (copy dev config as template if present, else empty)
if exist ..\anykey_config.json (
    copy /y /q ..\anykey_config.json dist\AnyKey\anykey_config.json >nul
) else (
    if not exist dist\AnyKey\anykey_config.json echo {}> dist\AnyKey\anykey_config.json
)

:: Code signing (optional, does not block release packaging)
where signtool >nul 2>&1
if errorlevel 1 (
    echo   WARN: signtool.exe not found; skipping sign
    goto :after_sign
)
set "SIGN_SUBJECT=AnyKey Project"
set "_SUBJECT="
if defined CERT_SUBJECT set "_SUBJECT=%CERT_SUBJECT%"
if not defined _SUBJECT set "_SUBJECT=%SIGN_SUBJECT%"
if defined CERT_PATH (
    if not defined CERT_PWD set "CERT_PWD="
    signtool sign /tr http://timestamp.digicert.com /td SHA256 /fd SHA256 /f "%CERT_PATH%" /p "%CERT_PWD%" "dist\AnyKey\anykey-tray.exe" "dist\AnyKey\anykey-gui.exe" 2>nul
) else if defined _SUBJECT (
    signtool sign /tr http://timestamp.digicert.com /td SHA256 /fd SHA256 /n "%_SUBJECT%" "dist\AnyKey\anykey-tray.exe" "dist\AnyKey\anykey-gui.exe" 2>nul
) else (
    echo   Skipped (no cert configured)
)
:after_sign

:: Assemble release package (release\anykey + release\anykeyFilterDriver + launcher)
echo [7/7] Assembling release package...
"%PYTHON%" build_release_package.py
if %errorlevel% neq 0 (
    echo ====== Release package FAILED ======
    pause
    goto :end
)

:done
echo.
echo ======== Build OK ========
echo Output:
echo   dist\AnyKey\
echo     anykey-gui.exe
echo     anykey-tray.exe     (Rust, ~1MB)
echo     _internal\   (shared)
echo     anykey_config.json
echo     engines\rust\
echo   anykey-filter-driver\deploy\anykey_flt.sys  (fresh driver)
echo   release\     (distributable: anykey\ + anykeyFilterDriver\ + 安装驱动.bat)
echo   Zip manually: python build_release_package.py --zip ^<version^>
echo ==========================

:end
popd
pause
