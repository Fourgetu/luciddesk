@echo off
setlocal
cd /d "%~dp0"
set "LUCIDDESK_SHARED_PANE_TREE=0"
set "LUCIDDESK_DISABLE_BACKDROP=1"
set "LUCIDDESK_RENDER_TRACE=%~dp0render-C.log"
start "" /wait "%~dp0luciddesk.exe"
endlocal
