@echo off
setlocal
cd /d "%~dp0"
"%~dp0bin\node.exe" "%~dp0launch.mjs" %*
if errorlevel 1 pause
