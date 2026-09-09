@echo off
:: AnyKey Filter Driver - One-click installer
:: Double-click this file to install. UAC will prompt for admin rights.

cd /d "%~dp0"

:: Self-elevate to admin
net session >nul 2>&1
if %errorlevel% neq 0 (
    echo Requesting administrator privileges...
    powershell -Command "Start-Process '%~f0' -Verb RunAs"
    exit /b
)

echo ==========================================
echo   AnyKey Filter Driver - Installer
echo ==========================================
echo.

:: Step 1/4: Ensure Windows Test Signing mode is ON (required for unsigned driver).
:: Single-pass install: if it is OFF, enable it now and CONTINUE — it becomes
:: active on the same reboot that activates the driver. No re-run needed.
echo Step 1/4: Checking Windows Test Signing mode...
bcdedit /enum | find /i "testsigning" | find /i "Yes" >nul
if %errorlevel% equ 0 (
    echo Test signing is already ON. Continuing...
    goto :files
)
echo Test signing is OFF. Enabling it now...
bcdedit /set testsigning on
if %errorlevel% neq 0 (
    echo ERROR: Failed to enable test signing. The most common cause is
    echo        Secure Boot being ENABLED in UEFI firmware. Disable Secure
    echo        Boot first, then run this script again.
    pause
    exit /b 1
)
echo   Test signing enabled. It will activate together with the driver after REBOOT.

:files
:: Check essential files
if not exist "anykey_flt.inf" (
    echo ERROR: anykey_flt.inf not found. Extract all files from the .zip first.
    pause
    exit /b 1
)
if not exist "anykey_flt.sys" (
    echo ERROR: anykey_flt.sys not found.
    pause
    exit /b 1
)

echo.
echo Step 2/4: Staging INF for hotplug support (keyboard + mouse)...
pnputil /add-driver anykey_flt.inf
pnputil /add-driver anykey_flt_mouse.inf

echo.
echo Step 3/4: Installing filter on existing keyboards and mice...
powershell -ExecutionPolicy Bypass -File "_install_anykey_device.ps1"
if %errorlevel% neq 0 (
    echo ERROR: Device installation failed.
    pause
    exit /b 1
)

echo.
echo Step 4/4: Done.
echo.
echo ==========================================
echo   Installation complete.
echo   REBOOT your computer now to activate.
echo ==========================================
pause
