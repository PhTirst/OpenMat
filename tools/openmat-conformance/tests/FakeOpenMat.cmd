@echo off
pwsh -NoProfile -File "%~dp0FakeOpenMat.ps1" %*
exit /b %ERRORLEVEL%
