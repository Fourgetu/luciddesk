@echo off
setlocal
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0Collect-Existing-Dumps.ps1"
pause
endlocal
