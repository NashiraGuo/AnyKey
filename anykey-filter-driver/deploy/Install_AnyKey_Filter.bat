@echo off
:: AnyKey Filter Driver - One-click installer
:: Double-click this file to install. UAC will prompt for admin rights.
::
:: TWO-PASS INSTALL. Test signing only takes effect after a reboot, and we must
:: never write a device UpperFilter while the driver cannot be loaded - that
:: would leave the user with no keyboard and no mouse after a reboot. So:
::     pass 1  test signing OFF  -> enable it, STOP, ask for a reboot
::     pass 2  test signing ON   -> install the driver
:: Run this script again after the reboot.

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

:: ---------------------------------------------------------------
:: Step 1/4: Test signing gate. Exit codes from _check_test_signing.ps1:
::     0 = test signing ON (Secure Boot off)        -> install now
::    10 = test signing was OFF, just enabled       -> reboot, then run again
::     1 = cannot make test signing effective       -> abort, install nothing
:: Nothing below this point runs unless the gate returned 0.
:: ---------------------------------------------------------------
echo Step 1/4: Checking Windows Test Signing mode...
powershell -ExecutionPolicy Bypass -NoProfile -File "_check_test_signing.ps1"
set "TS=%errorlevel%"

if "%TS%"=="10" (
    echo.
    echo ==================================================
    echo   REBOOT REQUIRED - nothing has been installed.
    echo.
    echo   Test signing has now been enabled, but it only
    echo   starts working after a reboot.
    echo.
    echo   Your keyboard and mouse were NOT touched, and no
    echo   driver was installed. There is nothing to undo.
    echo.
    echo   REBOOT NOW, then run this installer again.
    echo ==================================================
    pause
    exit /b 0
)
if not "%TS%"=="0" (
    echo.
    echo ==================================================
    echo   INSTALL ABORTED - nothing was installed.
    echo.
    echo   Test signing is not active, so the filter driver
    echo   could not be loaded. Installing it anyway would
    echo   leave you with no keyboard and no mouse after a
    echo   reboot. No device setting was touched.
    echo.
    echo   See the message above for the reason and the fix.
    echo ==================================================
    pause
    exit /b 1
)
echo   Test signing is ON. Continuing...

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
if not exist "_boot_guard.ps1" (
    echo ERROR: _boot_guard.ps1 not found. Extract all files from the .zip first.
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
echo.
echo   If the driver fails to load, a one-shot guard
echo   registered during this install will undo it and
echo   reboot your machine automatically.
echo ==========================================
pause
