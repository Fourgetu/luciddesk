@echo off
setlocal
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0Configure-Crash-Dumps.ps1" -Mode Enable
pause
endlocal
