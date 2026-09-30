# LucidDesk 安装与更新

运行 `LucidDesk-版本-windows-x64-setup.exe`。安装包使用 Inno Setup，首次安装可选择安装范围：

- **仅为当前用户安装**：默认 `%LOCALAPPDATA%\Programs\LucidDesk`，无需管理员权限。
- **为所有用户安装**：需要确认 Windows 管理员权限提示，默认 `C:\Program Files\LucidDesk`，也可选择其他目录。

快捷方式和卸载入口跟随安装范围。升级自动沿用原安装范围和目录；已有当前用户安装需要切换为所有用户安装时，请先卸载旧安装（配置会保留）。可选择创建桌面快捷方式。

配置、面板布局和备份保存在 `%LOCALAPPDATA%\LucidDesk`，旧数据目录的兼容规则及自定义数据目录保持不变。升级沿用原安装目录，保留配置。安装包不会写入或清理你的桌面文件。

卸载确认弹窗中默认勾选“保留用户配置”，与“是／否”按钮位于同一行。取消勾选并确认卸载后，会在卸载完成时删除当前账户的 `%LOCALAPPDATA%\LucidDesk` 和旧版 `%LOCALAPPDATA%\LucidPane`，包括设置、面板布局和备份。取消卸载不会删除配置。原文件、自定义数据目录、便携版数据和其他账户的数据保留。

静默卸载默认保留配置；显式传入 `/DELETEUSERDATA` 才清理上述目录，`/KEEPUSERDATA` 优先保留。

## 更新

1. 打开“设置 → 关于”，点击“检查更新”。只在手动操作时联网。
2. 点击“打开更新页面”，在 Release 页面查看说明并自行下载安装包。
3. 退出 LucidDesk 后运行安装包，按提示完成升级，最后可重新启动程序。

安装或卸载时若 LucidDesk 正在运行，会提示自动退出，确认后请求正常退出，等待进程完全结束及桌面 DLL 释放后再继续。拒绝自动退出、程序无响应或桌面组件未释放时，不会删除或覆盖程序文件。安装器阻止降级；若目录包含 `portable.marker`，要求选择其他目录。

静默安装（`/VERYSILENT /SUPPRESSMSGBOXES`）自动确认关闭；传入 `/NOCLOSEAPPLICATIONS` 可禁用自动关闭。安装器只关闭 LucidDesk，不结束 Explorer，也不会强制终止无响应的程序。

命令行可用 `/CURRENTUSER` 或 `/ALLUSERS` 指定安装范围；安装到 Program Files 时使用 `/ALLUSERS`。用户配置仍分别保存在各自的 LocalAppData 中，普通运行无需管理员权限。

程序直接加载安装目录中的桌面 DLL，退出时等待回调清理并从 Explorer 卸载。安装器会检查 Explorer 实际加载的组件，包括其他目录或曾被改名的旧组件，确认释放后才替换文件。启动时也会检查，残留组件存在时显示原因并等待重连。若旧版曾永久固定该组件，请退出 LucidDesk，再重启 Windows 资源管理器或注销重新登录后升级。

普通 ZIP 版也可下载并使用安装包，已有 LocalAppData 配置会继续使用。便携版请自行下载便携 ZIP，退出程序后手动覆盖原程序目录并保留 `data`。详情见[便携版说明](portable.md)。

安装包、普通 ZIP、便携 ZIP 都附带 SHA256 文件。`build.json` 记录源码版本、Git 修订、是否有未提交改动，以及主程序和 Hook DLL 的校验值。
