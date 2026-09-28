# LucidDesk 品牌规范

产品名称为 **LucidDesk**，连写且 L、D 大写。暂不设置中文名，不使用 Lucid Desk 或 Luciddesk 作为展示名称。

## 产品表达

- 功能说明：桌面分组与文件整理。
- 主标语：把 Windows 桌面整理成顺手的工作区。
- 简介：将桌面图标拖入分组，把常用文件夹留在手边，用 Everything 随时查找文件。
- EXE 文件描述：LucidDesk（任务管理器显示名称）。

中文功能说明是产品描述，不是中文品牌名。文案优先说明具体操作与用途，避免将文件引用收纳描述成移动真实文件。

## 视觉与命名

沿用已选定的蓝青色三面板图标，EXE、窗口、托盘和关于页共用同一资源。图标原稿和资源文件保留原文件名，以保证可追溯性。

应用文件名与 Cargo 包名为 `luciddesk`；可执行文件为 `luciddesk.exe`。预览包使用 `LucidDesk-版本-preview-构建标识-windows-x64-时间戳`。作者为 Yuchen95，版本号从 Cargo 自动写入 EXE 文件属性。

## 兼容约定

程序旁存在 `portable.marker` 时，使用同目录下的 `data` 保存配置；显式设置的数据目录环境变量优先。便携包命名为 `LucidDesk-版本-windows-x64-portable.zip`。

新安装使用 `%LOCALAPPDATA%\LucidDesk`。若新目录不存在且旧目录 `%LOCALAPPDATA%\LucidPane` 已存在，则继续使用旧目录，保留配置、布局和备份，不自动搬移文件。

`LUCIDDESK_DATA_DIR` 为首选数据目录变量，兼容旧变量 `LUCIDPANE_DATA_DIR`；同时设置时新变量优先。旧版单实例标识和 Hook 通信标识保持兼容，避免两个品牌版本同时接管桌面。

历史研究记录、图标原稿和内部诊断变量可以保留旧名；用户界面、当前使用文档和发布产物统一使用 LucidDesk。
