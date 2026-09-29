@echo off
setlocal
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0Collect-Explorer-Error.ps1"
pause
endlocal
