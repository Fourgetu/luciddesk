@echo off
setlocal
cd /d "%~dp0"
set "LUCIDPANE_SHARED_PANE_TREE=0"
set "LUCIDPANE_DISABLE_BACKDROP=1"
set "LUCIDPANE_RENDER_TRACE=%~dp0render-C.log"
start "" /wait "%~dp0luciddesk.exe"
endlocal
