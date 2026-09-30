@echo off
setlocal
cd /d "%~dp0"
set "LUCIDDESK_SHARED_PANE_TREE=1"
set "LUCIDDESK_DISABLE_BACKDROP=0"
set "LUCIDDESK_RENDER_TRACE=%~dp0render-B.log"
start "" /wait "%~dp0luciddesk.exe"
endlocal
