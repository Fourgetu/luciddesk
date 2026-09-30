# 保留下一次崩溃转储

这些脚本配置 Windows Error Reporting，为后续崩溃保留转储，并支持恢复原设置。

1. 解压到一个固定目录。右键 Enable-Crash-Dumps.cmd，选择“以管理员身份运行”。
2. 保留此目录和生成的 crash-dump-settings-backup.json。正常使用即可，无需反复触发崩溃。
3. 下次出现问题后，在正常登录用户下运行 Collect-Existing-Dumps.cmd，得到 ZIP。
4. 排查结束，右键 Restore-Crash-Dumps.cmd，以管理员身份运行，恢复之前的设置。

配置仅写入 HKLM 下 LocalDumps 的 explorer.exe 和 luciddesk.exe 子项，使用小型转储、数量上限 5。
输出位置是发生崩溃的用户的 %LOCALAPPDATA%\CrashDumps。
不会重启程序、修改全局转储设置或自动上传；恢复配置不会删除已保存的转储。
若使用另一个管理员账户运行采集工具，请直接从发生崩溃的账户的上述目录获取 .dmp。
转储可能包含进程内存中的私人信息，请确认后再分享。

机制说明：https://learn.microsoft.com/en-us/windows/win32/wer/collecting-user-mode-dumps
