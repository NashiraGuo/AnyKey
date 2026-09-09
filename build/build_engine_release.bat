@echo off
cd /d "%~dp0"
python build_engine_release.py %*
if errorlevel 1 pause
