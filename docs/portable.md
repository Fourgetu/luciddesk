# LucidDesk 便携版

桌面面板与文件整理 · 开发者 Yuchen95

## 启动

完整解压到有写入权限的文件夹，退出已运行的旧版，再双击 `luciddesk.exe`。请将 `luciddesk_desktop.dll` 和 `portable.marker` 保留在程序旁。优先维护 Windows 11 x64，Windows 10 已由用户完成实机验证。

## 数据随程序携带

`portable.marker` 启用便携模式。首次运行自动在程序旁创建 `data`，保存配置、布局和备份；初始包不含个人数据。退出程序后，复制整个目录即可一并携带设置。面板引用的文件和文件夹仍在原位置，不包含在便携数据中。

更新时先退出程序，保留原 `data` 目录，再替换程序文件。首次便携启动不会自动导入旧版 LocalAppData 数据，需要时可通过设置中的备份与恢复导入。

如果设置了 `LUCIDDESK_DATA_DIR` 或旧变量 `LUCIDPANE_DATA_DIR`，其指定目录优先于便携目录。不要从 ZIP 内直接运行，也不要放在禁止写入的目录。

## 使用与校验

操作见 [使用说明](usage.md)，本次变化见随包提供的 `CHANGELOG.md`（中文）与 `CHANGELOG.en.md`（英文）。Everything、PowerToys Peek 和 QuickLook 需另行安装，默认关闭相关功能。

`build.json` 保留实际构建来源和二进制文件校验值。ZIP 旁的 `.sha256` 用于验证包完整性。
