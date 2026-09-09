# Windows 11 原生精简菜单：宿主接口调查

更新：2026-09-08。目标是实际系统精简菜单及顶部操作栏，不把自绘 WinUI 菜单或系统经典菜单算作完成。

## 当前判断

普通文件工具栏实测：自有文本文件通过正式共享接口显示剪切、复制、共享、删除四个顶部按钮，没有重命名。在独立探针启用 `--can-rename`（仅增加 CMF_CANRENAME）后，已观察到剪切、复制、重命名、共享、删除五个按钮，确认该标志影响工具栏。已请求用户点击重命名，仅观察输入框并 Esc 取消；提交改名及隐藏原生视图下的编辑行为尚未验证，生产接口暂不启用该标志。当前探针命令为 `--window --hold --activate-desktop --can-rename --target C:\Users\Yuchen\Desktop\LucidPane-menu-validation-20260908.txt`，日志在 `target/menu-rename-stdout.log` 和 `target/menu-rename-stderr.log`。

普通文件身份匹配修正：创建自有测试文件 `C:\Users\Yuchen\Desktop\LucidPane-menu-validation-20260908.txt`（66 字节，无用户数据）后，原 CANONICAL 比较未匹配，但解析名枚举确认它实际位于桌面索引 24；Refresh 返回成功也未改变比较结果。加入 SICHINT_TEST_FILESYSPATH_IF_NOT_EQUAL 后正确匹配索引 24，选择往返验证通过；回收站仍匹配索引 2，仓库 README 仍被拒绝。原因是合并桌面命名空间与文件系统解析路径可有不同 PIDL，需要 Shell 文件路径比较。生产路径未加入刷新。Clippy 全目标通过，新 debug 编译通过，并重新启动独立验收 pane；测试文件保留供复制/重命名验收，后续需清理。

用户交互验收（2026-09-08）：用户提供三张截图，分别显示回收站精简菜单、“显示更多选项”后的经典菜单（含 WizTree 扩展）及“回收站 属性”系统窗口，确认回收站的实际菜单切换和属性命令可用。对应 `target/menu-user-stderr.log` 记录现代弹窗隐藏后出现 #32768 SHOW / MENUPOPUPSTART，再收到 MENUPOPUPEND / MENUEND，随后才输出 `native_popup_closed_after_ms=1794` 和 `desktop_selection_and_focus_verified=true`。这次切换没有提前结束等待；后续会话也记录选择恢复核对成功，测试 PID 3908 仍响应。此前等待这两项手动验证的阻碍已解除，但不能由回收站结果推断普通文件的复制、重命名或所有 Shell 扩展均已通过。

重命名边界：当前入口传入 CMF_ITEMMENU，没有 CMF_CANRENAME。微软文档说明后者代表调用方支持重命名，适用时应加入重命名项。因此不能把当前菜单显示成功等同于完整工具栏与重命名可用；隐藏原生列表的编辑框行为仍需验证。生产日志已改为可关闭且忽略写入错误，避免输出不可用时从 Windows 回调抛出 panic；此加固不能证明此前无日志退出的原因。

手动交互验收窗口：`NativeMenuValidation` 使用 `target/menu-user-validation` 独立数据，stdout/stderr 在 `target/menu-user-*.log`。已向用户请求协助验证“显示更多选项”和“属性”，因为当前输入工具会先激活 pane，导致 Explorer 弹窗取消。等待用户反馈时不将命令验收标记通过。

正式图标右键入口现已改用 `desktop_shell::show_desktop_item_menu`，共享代码位于 `crates/desktop-shell/src/native_menu.rs` 及同名目录。旧 TrackPopupMenuEx 入口已移除；失败显示错误，不将经典菜单冒充精简菜单。原生菜单的完整验收仍未完成，以下历史研究记录描述各阶段证据。

实际 pane 集成验证：新版 debug 程序以 `--preview --title MenuIntegrationCheck` 启动，`LUCIDPANE_DATA_DIR` 指向工作区 `target/menu-pane-validation`，与运行中的旧 release 桌面实例隔离。回收站右键实际显示精简菜单，两次点击空白取消均记录 `desktop_selection_and_focus_verified=true`。第二次开启自动收起，测试数据库 `panel_behavior` 确认该分组 `auto_hide=1`；菜单打开约 41 秒期间 pane 保持展开，取消后进程正常响应。测试实例通过分组菜单退出。首次直接 shell 启动的预览在右击附近消失，没有捕获退出原因；改为 Start-Process 并重定向日志后未复现，stderr 为空，不能将首次消失定性为已修复的崩溃。尚缺“显示更多选项”、实际文件命令、正式桌面模式等验证。

使用 `--window --bridge --target '::{645FF040-5081-101B-9F08-00AA002F954E}'` 调用与正式程序完全相同的共享接口，已观察到回收站精简菜单、点击空白关闭，日志返回 `desktop_selection_and_focus_verified=true` 和 `foreground_popup_result=Ok(())`。测试进程正常退出。工作区 59 项测试通过，1 项手动性能基准跳过，Clippy 全目标通过；受限环境无法访问 Explorer 的测试在具备桌面访问权限的环境复验通过。实际 pane、普通文件命令及“显示更多选项”的交互仍待验收；当前运行中的旧 release 程序未被替换。

目前更值得推进的是 **Explorer 桌面 IContextMenuSite 桥接**。2026-09-08 的独立前台测试窗口已实际触发 Windows 11 精简菜单，截图观察到顶部操作栏和“显示更多选项”。生产程序隐藏原生图标时也已验证可显示，焦点交接后点击空白正常关闭。这是原生弹窗的原型验证，不是 pane 集成完成；目标一致性、完整菜单生命周期和真实命令仍需验证。

此前的独立 STA + ContextMenuPresenter 路线虽然可取得接口并初始化，但本机初始化检查 `IsProcessAnExplorer`，普通探针的 XAML 启用标志为 0。因此不能继续把“CoCreate / Initialize 成功”等同于独立进程可以显示新版菜单。未使用注入、进程伪装或系统 DLL 补丁。

保持现有约束：不移动文件路径、不关闭桌面自动排列、不重启或注入 Explorer、不因菜单开关清空图标缓存。新宿主验证完成前，不接入生产菜单入口。

## 方案比较

| 路线 | 已有证据 | 尚缺什么 / 是否符合目标 |
| --- | --- | --- |
| IContextMenu + TrackPopupMenuEx | 当前生产代码调用此路径 | 系统经典菜单，不符合精简菜单要求 |
| WinUI MenuFlyout + Shell 命令 | Files 项目枚举 Shell 菜单，再转换为自己的 flyout model | 可以优化外观，但不是系统精简菜单宿主 |
| 向隐藏桌面转发 WM_CONTEXTMENU | 先前试验实际弹出了桌面背景菜单 | 目标上下文错误，已从生产代码撤下 |
| IExplorerBrowser + IResultsFolder | 本机独立隐藏宿主能添加跨路径 / 命名空间项目、选择项目、取得 IContextMenu | 证明 Shell 选择上下文可建立，不证明其默认弹窗是新版 |
| 系统 ContextMenuPresenter | 本机 CoCreateInstance 成功，私有 IContextMenuPresenter 的 QueryInterface 返回 S_OK | 需验证初始化、宿主回调、XAML 运行时、弹窗和命令；接口没有公开兼容性承诺 |
| Explorer IContextMenuSite 桥接 | 桌面接口可跨进程取得；原生图标隐藏时也能显示精简菜单，焦点交接后可关闭 | 接口文档已标为不可用；本机行为不能代替跨版本验证；真实目标、命令及完整生命周期待验收 |
| Explorer 内部挂钩 | ExplorerPatcher 源码存在 Shell / 菜单相关挂钩和版本相关逻辑 | 不等于可复用的菜单宿主；当前不采用注入路线 |

## 本机独立探针

代码：`crates/desktop-shell/examples/menu_host_probe.rs`。创建自己的隐藏窗口和 Shell view，不改变 Explorer 桌面选择，不打开菜单，不调用文件命令。运行需要可访问交互式桌面 COM 的环境；受限环境可能在 Shell 初始化时返回 E_FAIL。

```powershell
cargo run -p desktop-shell --example menu_host_probe -- E:\Project\LucidPane\README.md E:\Project\LucidPane\app\Cargo.toml
cargo run -p desktop-shell --example menu_host_probe -- E:\Project\LucidPane\README.md '::{645FF040-5081-101B-9F08-00AA002F954E}'
```

Shell view 使用 `EBO_NAVIGATEONCE | EBO_NOTRAVELLOG`、`FillFromObject(NULL, EBF_NODROPTARGET)`，然后通过 `IResultsFolder::AddItem` 添加真实 IShellItem。结果通知是异步的，必须处理本线程窗口消息后再选择；未处理消息时曾出现计数正确但 SelectItem 返回 E_INVALIDARG。

已经分别验证两个文件和文件 + 回收站的集合：计数 2、选中 1、选择标志 0x1，取得选中项 IContextMenu 成功。最终新增接口查询以单个 README.md 再次运行成功。

本机系统：Windows 11 25H2，build 26200，UBR 9168。只读注册表显示：

```text
CLSID {86ca1aa0-34aa-4e8b-a509-50c905bae2a2}
Name: File Explorer Context Menu
InprocServer32: C:\Windows\System32\Windows.UI.FileExplorer.dll
ThreadingModel: Apartment
```

该类可通过 `CoCreateInstance` 取得 IUnknown；以下公开接口查询失败：IContextMenu、IShellExtInit、IInitializeWithItem、IMenuPopup、IObjectWithSelection、IExecuteCommand、IObjectWithSite。

IInspectable 查询成功，但 RuntimeClassName 为空；`GetIids` 成功且只返回 `30D5A829-7FA4-4026-83BB-D75BAE4EA99E`，即 Windows.Foundation.IClosable。**GetIids 没有列出后面查询成功的私有 COM 接口，因此不能用它的结果断言其他接口不存在。**

## 公开符号带来的新证据

从本机 DLL 的 RSDS 记录得到 PDB 标识 `3277EA25232962C91AD0A412ABBE98C5`，age 1。从微软 symbol server 下载匹配的 `windows.ui.fileexplorer.pdb`，未修改或加载替换系统 DLL。

匹配符号中的 `ContextMenuPresenter_Old` 实现包含 IContextMenuPresenter。读取对应 QueryInterface 实现中的比较常量和 vtable，定位到：

```text
IContextMenuPresenter IID: 37a472f7-63cf-4ccf-a88b-5231a3c7d8b6
实际 QueryInterface: HRESULT(0x00000000)
```

默认探针只执行查询。新增 `--initialize` 选项会先匹配本机实际对象的 11 个 vtable 项，再使用已从调用方核对的签名初始化并关闭。以下是匹配符号给出的签名，**不是公开 SDK 合约**：

```text
Initialize(int, IInvokeContextMenuCommand*, HWND, CONTEXT_MENU_PRESENTER_HOST) -> HRESULT
PrepareForContextMenu(LOCATION_CONTEXT_FLAGS, IUnknown*, POINT, IContextMenu*,
                      uint, uint, int, const ICIVERBTOIDMAP*, GUID, IShellItem*) -> HRESULT
DoContextMenu(HMENU, CONTEXT_MENU_PRESENTER_FLAGS, GUID) -> void
DismissContextMenu(const wchar_t*) -> void
IsContextMenuOpen() -> int
Invoke(uint) -> void
IsReadyForNewContextMenu(GUID, POINT) -> int
DisplayAccessKeys() -> void
```

Initialize 的匹配实现包含 InitializeXamlIsland、DispatcherQueue 创建和菜单扩展获取。它保存 IInvokeContextMenuCommand 指针、HWND 和 host enum。`_Old` 是本机符号名称，不能仅凭该名称判断用户看到的菜单版本；实际输出必须观察验证。

本机实际对象的 11 个 vtable 项与匹配符号完全一致。进一步下载了匹配的 shell32.pdb（RSDS `4907816C76ABD6288BBE01D3E8033EE9`，age 1），CDesktopBrowser::EnsureContextMenuPresenter 的调用点确认参数为 `Initialize(1, callback, HWND, 1)`。CDefView 的 QueryInterface 表和 vtable 给出回调 IID `9a19ddcf-9ed4-4a18-89de-c3b4dd1d7ae3`，继承 IUnknown，另有：

```text
Invoke(IContextMenu*, IShellItemArray*, HMENU, uint, POINT) -> void
OnContextMenuDismiss() -> void
DoExpandedContextMenu(POINT, int, GUID*, POINT*, int) -> void
SetFocus() -> void
```

独立 Shell 视图的回调 QueryInterface 成功；初始化返回 S_OK，IClosable::Close 成功。但匹配实现中用于控制 XAML 路径的字段为 0。反汇编及导入表证明判断条件包括 Windows.Storage.dll!IsProcessAnExplorer；没有通过修改字段或挂钩绕过检查。

```powershell
cargo run -p desktop-shell --example menu_host_probe -- --initialize E:\Project\LucidPane\README.md
```

## Explorer 桥接的实际弹窗验证

新探针 `crates/desktop-shell/examples/desktop_menu_service_probe.rs` 默认只读查询：

```text
desktop_view_IContextMenuSite=Ok(())
desktop_browser_IContextMenuSite=E_NOINTERFACE
desktop_view_IServiceProvider=Ok(())
desktop_browser_presenter_service=0x80040155
desktop_view_presenter_service=0x80040155
```

直接取得私有 presenter 服务失败于接口未注册（跨进程封送），不能把私有指针直接当本地对象调用。而旧的 IContextMenuSite 可跨进程取得。匹配 shell32 实现显示：DoContextMenuPopup 检查原生视图 IsWindowVisible，再走带默认 Mode 的菜单路径；该路径包含 DoCuratedContextMenu。官方文档的不可用标记与本机仍有实现并不矛盾：它仍然不是有公开兼容性承诺的新接口。

```powershell
cargo run -p desktop-shell --example desktop_menu_service_probe -- --window --hold --activate-desktop
```

测试窗口右键事件中调用 AllowSetForegroundWindow（Explorer PID），返回 1；临时选中首个桌面项目（实测显示名 yuchen liu），从同一桌面 IShellView 取得选中项 IContextMenu，再调用 `IContextMenuSite::DoContextMenuPopup(menu, CMF_ITEMMENU, point)`。

观察结果：后台命令模式返回 S_OK，但没有观察到菜单显示；前台测试窗口模式实际显示了系统精简菜单，顶部有适用于该项目的“复制 / 删除”，底部有“显示更多选项”。没有点击文件命令。通过 Esc / 焦点取消后关闭测试窗口，探针进程已正常退出；恢复选择的 API 返回成功。另在 Explorer 的 lucidpane.exe 上右键，观察到完整剪切、复制、重命名、共享、删除操作栏，作为系统原生对照。

**焦点和关闭事件：** 仅调用 AllowSetForegroundWindow 可以显示菜单，但实测 Esc 和点击空白未能正常关闭。增加 Explorer 根窗口的 SetForegroundWindow 和 IShellView::UIActivate(SVUIA_ACTIVATE_FOCUS) 后，点击空白正常关闭。DoContextMenuPopup 在关闭前返回 S_OK，因此 `--hold` 已改为监听限定 Explorer 进程和桌面线程的 WinEvent，并检查实际弹窗可见性；弹窗隐藏稳定 200 毫秒后才释放对象和恢复选择，不再固定等待 25 秒。尚未覆盖“显示更多选项”的经典菜单切换、全部子菜单和 Esc 行为。

**原生图标隐藏已验证兼容：** 生产程序隐藏的是子窗口 SysListView32，父级 SHELLDLL_DefView 仍可见。2026-09-08 在 LucidPane 实际运行时记录 `menu_host_visible=1, native_icon_list_visible=0`，仍成功显示系统精简菜单，并通过点击空白正常关闭，恢复选择 API 返回成功。此前将父视图可见性检查误认为图标列表必须可见的推断不成立。裁剪实验在修改前拒绝运行，未改变任何窗口区域，现已移除，无需更换现有隐藏和恢复守护方案。

**目标匹配：** 原型支持 `--target` 传入路径或命名空间解析名，通过 IShellItem::Compare(SICHINT_CANONICAL) 匹配桌面项目，找不到就拒绝。`--resolve-only` 在选择和弹窗前退出。只读实测回收站解析名匹配索引 2，仓库 README（不属于桌面）返回 E_INVALIDARG，未替换为其他图标。选择快照已改为保留 IShellItem 身份并单独读取 GetFocusedItem，恢复前重新定位，跳过已不存在的项目。

**选择恢复实测：** 新增 `--selection-roundtrip --target '::{645FF040-5081-101B-9F08-00AA002F954E}'`，临时选择回收站后立即恢复，并逐项重新读取选择标志和 GetFocusedItem，与快照比较。本机返回 `desktop_selection_and_focus_verified=true`。正常路径恢复失败会传递错误；提前退出仍由 RAII 尝试恢复。这证明了当前桌面状态下的选择往返，不代表菜单命令或用户并发更改时也已验证。

**关闭观察器补充：** 同时跟踪现代 PopupWindowSiteBridge 和经典 #32768 的 SHOW / MENUPOPUPSTART，并在检查存活时重新核对窗口类、进程和线程，防止旧 HWND 复用导致错误等待。实际“显示更多选项”切换与子菜单时序仍待交互验证，200 毫秒关闭稳定窗口也仍需验证。

**前台交互补充：** 测试窗口启用 Per-Monitor V2 DPI，并使用右击时的屏幕鼠标坐标。本机回收站精简菜单已在点击位置显示。关闭测试窗口的中止路径也返回 `desktop_selection_and_focus_verified=true`。尝试自动点击“显示更多选项”时，输入工具将前台切回测试窗口，导致菜单取消，没有观察到经典菜单；不得将此次操作计作展开成功。

**pane 自动收起：** 正式窗口新增菜单活动作用域，菜单消息循环期间暂停自动收起和重复菜单请求，退出后恢复计时。此改动同时覆盖现有项目菜单与分组菜单；尚未在正式原生桥接中交互验收。

**原型限制：** 命令行旧 `--show-first` 入口仍用固定坐标；窗口入口已用实际鼠标坐标。新增目标匹配和选择往返已验证，尚未覆盖完整菜单交互。接入生产前还需真实命令、多选、多屏 DPI、pane 自动收起、用户同时改变选择等验证。恢复时不能触发用户图标批量刷新。

原始符号、诊断脚本和反汇编输出位于忽略目录 `target/research/`，不属于生产依赖。生产程序不会按这些 RVA 调用系统函数。

## 下一步实验与验收

1. 继续 IContextMenuSite 路线：补充“显示更多选项”、子菜单和 Esc 的关闭事件验证，避免过早释放目标上下文。
2. 保留已验证兼容的原生图标隐藏与恢复守护；不把私有 vtable 或内存字段访问接进生产入口。
3. 将 IShellItem / PIDL 作为目标上下文，保持请求 ID、窗口位置、DPI、选择项和命令 ID 一致。不要通过显示名称或全局桌面选择猜目标。
4. 先验证显示和取消，不执行删除等有副作用命令。必须观察到系统精简菜单及顶部操作栏，并确认“属性”等项目对应正确文件。
5. 接入真实命令回调后，以专用测试文件验证复制、重命名等操作；普通文件、快捷方式、目录、回收站和混合选择分别验证。
6. 验证 Esc、点击空白、焦点切换、pane 收起、helper 崩溃、主程序退出、多屏 DPI；菜单不能出现在 pane 后面，不能触发图标批量重新加载。
7. 满足上述行为后才接入现有右键入口；仍需跨支持系统版本验证。当前原生图标可见及隐藏两种状态下的单项目前台原型已显示原生菜单，焦点交接后点击空白可正常关闭；pane 集成与命令验收未完成。

## 主要来源

- [微软 IExplorerBrowser 文档](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-iexplorerbrowser)：公开 Shell view 宿主。
- [微软 FillFromObject 文档](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-iexplorerbrowser-fillfromobject)：results folder 与宿主选项。
- [微软 ExplorerBrowserCustomContents 示例](https://github.com/microsoft/Windows-classic-samples/blob/main/Samples/Win7Samples/winui/shell/appplatform/ExplorerBrowserCustomContents/ExplorerBrowserCustomContents.cpp)：建立 results folder、取得 IResultsFolder 并添加项目。
- [微软现代菜单扩展文档](https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/integrate-packaged-app-with-file-explorer)：IExplorerCommand 和应用身份用于添加命令，不能当成显示整套菜单的接口。
- [Files Shell 菜单代码](https://github.com/files-community/Files/blob/51bf6328241ce94463a686956cba5db6c43967b9/src/Files.App/Utils/Shell/ContextMenu.cs) 与 [自有 flyout model 转换](https://github.com/files-community/Files/blob/51bf6328241ce94463a686956cba5db6c43967b9/src/Files.App/Data/Factories/ShellContextFlyoutHelper.cs)：原生命令与自有显示层的区别。
- [ExplorerPatcher 源码](https://github.com/valinet/ExplorerPatcher/blob/0a88a6e0ef6b1752fea36e581cffff1097e862b0/ExplorerPatcher/dllmain.c)：版本相关挂钩的实例；不能据此声称其提供可直接嵌入的 Win11 菜单。
- [本机匹配的微软 PDB](https://msdl.microsoft.com/download/symbols/windows.ui.fileexplorer.pdb/3277EA25232962C91AD0A412ABBE98C51/windows.ui.fileexplorer.pdb)：内部接口和方法签名的本机证据，不是官方公开 API 文档。
