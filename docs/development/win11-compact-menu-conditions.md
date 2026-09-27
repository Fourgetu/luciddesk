# Win11 精简菜单：最小可行条件与后续设计

2026-09-15，本机二进制分析及可见原型对照。本文区分“能够弹出”和“能够可靠执行命令”；不把当前私有实现当作所有 Win11 版本的公开契约。

最新结论（2026-09-15）：已定位独立宿主遗漏的 `FILE_EXPLORER_CONTEXTMENU_INVOKEMENUITEM → IContextMenuPresenter::Invoke` 转发。补齐后，独立原型的精简菜单重命名、属性和 Esc 关闭均通过。当前方向是保留 Pane 外观，使用 Explorer 内独立菜单宿主，无需完整 Explorer 文件窗口。机制、代码入口和最新验收见 [命令路由技术文档](../win11-compact-menu-command-routing.md)。下文失败对照保留作为修复前的历史证据，不代表现状；全局最小必要条件仍未完成消融验证。

## 已验证的菜单显示组合

**Explorer 进程 + 独立 OLE STA + 一个可见宿主窗口 + 一个文件的原生 Shell 视图 + 已启用的原生 presenter + 消息循环**，已实际显示 Win11 精简菜单。

这是菜单显示的充分组合，不是对每个组成部分都完成消融实验后的全局最小集合，也不足以保证点击执行。这组独立视图实验没有使用真实桌面选中项或恢复桌面图标。

| 条件 | 证据与边界 |
| --- | --- |
| presenter 初始化时启用精简路径 | 当前 `ContextMenuPresenter_Old::Initialize` 的启用表达式包含：请求启用、`IsProcessAnExplorer()` 为真、Shell server mode 不等于 6。初始化 S_OK 不代表精简路径已启用。 |
| Shell 认可的宿主进程 | `Windows.Storage.dll` 中的 `IsProcessAnExplorer` 会检查进程完整路径，对比展开后的系统 Shell 路径，并缓存结果；仅改普通程序文件名不足以满足。实验使用真实 Explorer 进程，没有修改该判断、进程模式或系统文件。 |
| 正确的项目上下文 | 原生 `IShellView` 提供选中项及命令回调，`IServiceProvider` 提供 presenter。一个真实文件就足够显示项目精简菜单。 |
| 菜单请求通过原生判定 | 当前 `CDefView::ShouldShowMiniMenu` 检查位置上下文、经典菜单请求状态及 presenter 可用性。正常鼠标右键进入此路径。 |
| presenter 调用链可用 | 运行日志记录 `ready=1`、`prepare=S_OK`、`show`，随后观察实际 XAML 精简菜单。单看这些返回值仍不足以判定菜单样式。 |
| 存活的窗口和消息循环 | 原型保留 Shell 视图、回调和 presenter，通过 Shell/WinUI 输入预处理和消息派发驱动 UI；菜单关闭不销毁宿主。可见窗口是此次已通过的形态，完全隐藏宿主仍未验证。 |

## 决定性的对照

| 实验 | 结果 |
| --- | --- |
| 普通 `shell_pane_probe.exe`，可见文件夹视图，私有 presenter 初始化成功 | 经典菜单；日志同样为 ready=1、prepare=S_OK、show |
| 普通进程中测试两种宿主参数、鼠标和显式请求 | 观察到的都是经典菜单 |
| 同一原型放入 Explorer 的独立线程，独立文件夹只包含一个测试文件 | Win11 精简菜单，含顶部剪切/复制/重命名和“显示更多选项” |
| 独立 STA，仅让 Shell 预处理自身的键盘消息 | 用户手动确认仍无法点击或关闭；不能把问题仅归因于鼠标误送入 Shell 的 TranslateAccelerator |
| Explorer 桌面 STA、自建嵌套消息循环、宿主参数 0 | 用户手动确认仍无法点击或关闭；使用原桌面线程不是充分条件 |
| 上一项直接传递系统原始 COM 接口，不包装 presenter 或回调 | 仍能显示精简菜单，自动点击未进入重命名；未取得此项单独的手动验收 |
| 正常资源管理器打开同一临时文件夹 | 用户手动确认精简菜单可以进入重命名；同一文件在系统完整宿主中可用 |
| 桌面 STA 创建常驻视图后立即返回，使用 Explorer 原有消息循环 | 用户手动确认没有反应、菜单未关闭；原有消息循环仍不是充分条件 |
| 完整 Explorer 窗口，只用窗口区域裁剪到原生文件视图 | 三次打开精简菜单，两次点击重命名进入原生编辑框；Esc 取消编辑、点击菜单外关闭均通过。仅临时文件，未提交文件改名 |

2026-09-15 对照中发现自动化控件索引有坐标映射错误：点击可能落到原型外，曾出现快捷方式菜单和桌面关闭对话框，均已取消。此类结果不能用于判断原型命令链。改用原型及其菜单的独立截图坐标后，已核对菜单对应文本文件且原型收到菜单服务查询。随后用户确认原有循环版本仍然无法交互；不能把早先的坐标错误解释成 Explorer 必然分派给桌面。

历史包装日志出现了 `callback.invoke command=0` 和 dismiss；这是取消路径的证据，不是重命名成功。可见原型的定位版本移除了包装层。后续实际 Pane 在补齐 Invoke 转发之后加入命令回调适配，将标准 rename 转给自绘 Pane 的输入框；用户已确认编辑框出现、属性窗口可打开和关闭，详见命令路由技术文档。

普通进程日志：`target/shell-pane-trace.log`。Explorer 内日志：`target/shell-pane-presenter-trace.log`。运行中的 Explorer PID 为 80676，测试使用 `target/shell-pane-probe/80676/A/`，未触碰用户文件。

当前分析涉及的 `Windows.Storage.dll` 文件版本为 `10.0.26100.8972`，`Windows.UI.FileExplorer.dll` 文件版本为 `10.0.10011.16384`。分析使用已有本机符号缓存和系统 DLL；原型调用通过 COM 接口和导出名解析，不依赖代码 RVA。

## 不需要绑定进菜单宿主的内容

- 不需要完整桌面项目列表，更不需要复制全部桌面坐标。
- 不需要修改真实桌面 ListView 的成员来弹菜单。
- 不需要把 Pane 的标题栏、分组配置和视觉样式交给 Shell。
- 不需要每次右键重建 Shell 宿主。
- 不需要完整的 Explorer 导航栏、标签页和主窗口框架。

## 围绕这些条件处理其他要求

| 要求 | 方案 | 状态 |
| --- | --- | --- |
| 保留 Win11 精简菜单 | Explorer 内提供专用菜单宿主，通过真实 Shell 项目上下文调用 presenter | 独立原型已通过显示、重命名、属性和取消；Pane 联测状态见最新技术文档 |
| 桌面不闪、不重新补回 Pane 成员 | 菜单使用独立视图，真实桌面的 Hook 过滤状态持续保持 | 原型不修改真实桌面；与正式过滤后端联测仍待做 |
| 每次右键不重建 | 常驻 STA；同一目标集重新验证后复用视图，不同目标集重建视图 | 已接入生产后端；不缓存过期的目标身份 |
| 保留 Pane 外观 | LucidPane 保留外壳和配置；菜单服务通过明确的目标身份和坐标协议调用 | 设计方案。跨进程父子窗口/焦点不能未经验证直接接入 |
| 由 Shell 绘制 Pane 内容 | 在 Explorer 内承载可见 Shell 内容窗口，再适配 Pane 的尺寸、显示与输入关系 | 可见内容原型已运行；正式嵌入未实现 |
| 混合目录分组 | 用结果集合保存目标 `IShellItem`，为重命名、移动、删除补充增量成员同步 | 混合集合可显示、可执行重命名；名称同步未完成 |
| 自绘图标配原生菜单 | 保留自绘内容，向专用 Shell 宿主传递选中身份 | 设计方案；隐藏宿主输入问题仍需解决 |
| 无转圈、菜单可点击关闭 | 先固定可见宿主作为对照，核对原生回调和输入路由，再优化初始化与缓存 | 未完成性能测量；自动点击存在激活窗口导致取消的干扰 |
| 跨版本兼容 | 启动时检查 COM/导出能力，私有 ABI 独立封装并做实际显示与命令检查 | 不能仅凭 QueryInterface 成功承诺全部 Win11 版本兼容 |

## 复现

```powershell
cargo build -p desktop-shell --features desktop-menu-diagnostics --example desktop_filter_probe --example desktop_filter_probe_hook
target/debug/examples/desktop_filter_probe.exe --in-process --visible-shell
```

命令使用现有 WH_GETMESSAGE 诊断入口，只负责在 Explorer 内启动原型 STA。入口返回后卸下消息 Hook；包含回调的诊断 DLL 保持映射到 Explorer 退出，避免异步 Shell 回调访问已卸载代码。关闭原型窗口只退出专用线程，不退出 Explorer。

原线程与消息循环对照：

```powershell
# 原桌面线程，自建嵌套消息循环（手动交互失败）
target/debug/examples/desktop_filter_probe.exe --in-process --visible-shell-desktop-sta
# 原桌面线程，创建后立即返回 Explorer 自身消息循环（手动交互同样失败）
target/debug/examples/desktop_filter_probe.exe --in-process --visible-shell-original-loop
```

原有循环模式将宿主保存在该线程，关闭时延迟释放宿主，绝不向 Explorer 原循环发送 WM_QUIT。不要在未关闭上一原型时再启动不同 DLL 版本；诊断 DLL 会保留到 Explorer 退出。该模式现已收到手动失败反馈。

## 完整 Shell 裁剪对照（2026-09-15）

`full_shell_pane_probe` 从完整 Explorer 宿主出发，不自行创建私有 presenter，不改真实桌面的成员或坐标。通过 `SetWindowRgn` 只显示其文件视图矩形，保持原来的窗口关系、Shell 服务、线程和命令处理。菜单属于独立弹出窗口，实际验证可以显示到裁剪区域外并接收点击。

```powershell
cargo run -p desktop-shell --example full_shell_pane_probe
```

原型先尝试微软文档中的 `CLSID_ShellBrowserWindow`。本机返回 E_FAIL，因此使用 `explorer.exe /n,<unique-fixture>`，随后只接管启动前不存在、且当前目录精确匹配本次唯一临时目录的新窗口。控制条可切换裁剪/完整模式，关闭控制条会解除区域并关闭本次窗口，不遍历关闭其他 Explorer 窗口。

通过的运行：控制程序 PID 62056，Explorer PID 57912，文件视图线程 49948，目录 `target/full-shell-pane-probe/62056`。日志 `target/full-shell-pane-probe.log`。三次菜单使用同一个 HWND/视图，日志仅一次 `ready` 与一次区域更新；菜单期间没有重建宿主。文件 `Rename probe.txt` 未更名、未修改内容。

此结果是**一组可交互的充分条件**，不是私有菜单的最小必要条件证明。它用于对照定位完整浏览器承担的命令转发职责；补齐独立宿主的转发后，产品不沿用完整 Explorer 裁剪路线。

未完成：正式 Pane 嵌入、混合目录分组、窗口移动/缩放/DPI 联动、隐藏宿主、自绘内容、Explorer 重启恢复、桌面过滤联测、冷启动/菜单加载耗时和其他 Windows 版本测试。裁剪原型中第三方菜单项仍有“正在加载”阶段；不能声称刷新或转圈问题已经解决。

参考：[微软的 Explorer 自动化模型说明](https://learn.microsoft.com/en-us/windows/win32/shell/developing-with-windows-explorer)。本机 CoCreateInstance 失败及裁剪后的交互结果来自实际测试，而非文档保证。

消息预处理规则已核对 [ContentPreTranslateMessage 官方说明](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/win32/microsoft.ui.dispatching.interop/nf-microsoft-ui-dispatching-interop-contentpretranslatemessage)：返回 TRUE 才表示消息已处理。自建循环按该规则派发；官方支持自定义/嵌套循环不代表本原型的 Shell 宿主服务已经完整。

独立进程反例：

```powershell
cargo run -p desktop-shell --example shell_pane_probe -- --folder --compact
```

## 下一步验收顺序

1. 独立原型的精简菜单重命名、属性、Esc 已通过；正式 Pane 继续按真实交互验收，不用弹出成功代替命令成功。
2. 在同一宿主中验证重复打开、Esc、点外部关闭、显示更多选项及至少一个可核对的文件命令。
3. 只有命令链通过后，逐项减少可见性、改变窗口关系和替换自绘内容；每次只改变一个条件。
4. 最后接回正式 Pane 和桌面过滤，检查全程成员及坐标稳定，再测冷启动和重复打开耗时。
