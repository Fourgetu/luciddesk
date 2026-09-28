# LucidDesk 免安装预览版

主要验证平台为 Windows 11 x64。本包用于试用，尚不是稳定发行版。

开发者：**Yuchen95**

解压整个目录，双击 `luciddesk.exe`。`desktop_hook.dll` 必须与程序放在一起；不要混用其他版本的 DLL。重复启动会唤起已有实例。退出可使用托盘菜单。

配置默认存放在 `%LOCALAPPDATA%\LucidDesk`，不会写入程序目录。免安装不表示配置随程序目录携带；开发时可通过 `LUCIDDESK_DATA_DIR` 指定独立数据目录。设置中的“备份与恢复”可以导出、恢复配置；备份不包含原文件。

## 兼容性

| 场景 | 当前行为 |
| --- | --- |
| 提供所需视图接口的 Explorer | 连接桌面分组；拖入只改变分组引用 |
| Hook DLL 缺失、视图过滤接口不支持或 Explorer 暂不可用 | 文件夹和搜索面板仍可使用；保留桌面分组配置并定期重连 |
| Explorer 重启 | 检测连接失效，等待重连并重建面板；不同系统版本仍需实机验证 |
| 显示器组合改变 | 等待变化稳定后恢复对应布局，将不可见面板移入可用屏幕；混合 DPI 和拔插仍需实机覆盖 |
| Everything 未安装或未运行 | 搜索页显示状态；先启动 Everything，再在设置中检测路径或选择程序并启用搜索面板 |
| PowerToys Peek 不可用 | 不影响分组和文件操作，可在设置中关闭或指定路径 |

Everything、PowerToys 和 QuickLook 不包含在本包中。Windows 10、ARM64 和远程桌面环境尚未经过完整验证。

开发版使用 `config.toml` 和 `workspace.db`，不迁移旧库。旧版数据请使用独立目录保留；本版不会读取 `hook-desktop.db`。

快捷键、文件夹映射等操作见 `usage.md`。`build.json` 记录版本、Git 修订和两个二进制文件的 SHA-256；ZIP 同目录的 `.sha256` 可核对下载完整性。

品牌更名兼容：若新的数据目录不存在且旧目录 `%LOCALAPPDATA%\LucidPane` 已存在，继续使用旧目录。`LUCIDDESK_DATA_DIR` 优先，旧变量 `LUCIDPANE_DATA_DIR` 仍受支持；详见[品牌规范](brand.md)。
