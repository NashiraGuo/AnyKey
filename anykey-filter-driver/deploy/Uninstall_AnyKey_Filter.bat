@echo off
:: AnyKey Filter Driver - One-click uninstaller
:: Double-click to fully remove the filter driver.

cd /d "%~dp0"

net session >nul 2>&1
if %errorlevel% neq 0 (
    echo Requesting administrator privileges...
    powershell -Command "Start-Process '%~f0' -Verb RunAs"
    exit /b
)

echo ==========================================
echo   AnyKey Filter Driver - Uninstaller
echo ==========================================
echo.

powershell -ExecutionPolicy Bypass -File "_uninstall.ps1"

echo.
echo ==========================================
echo   REBOOT to complete uninstall.
echo ==========================================
pause
