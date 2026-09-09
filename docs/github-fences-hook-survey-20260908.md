# GitHub 类 Fences 项目与 Explorer Hook 路线核查

核查日期：2026-09-08。对象是 LucidPane 的架构选择，重点是原生图标、文件路径、自动排列、未收纳项目和 Windows 11 右键菜单。

## 结论

**Explorer 进程内 Hook 值得做独立验证，但这轮没有找到可直接复用、已经证明同时满足全部要求的开源 Fences 实现。**

38 个候选仓库已取固定提交的源码或文本文件，检查入口、相关文档及 Hook/布局/绘制调用；对关键候选追查了实现。核查深度并不相同，后面的清单明确区分代码证据和初筛。没有编译运行这些项目，因此不把源码意图当作 Windows 11 上的运行验证。

发现了两个重要方向：

1. `weiweigogo/openFrence` 确实有 Explorer 进程内桌面窗口 Hook 实验代码，但当前主程序仍自绘图标与 Fluent 菜单，该 DLL 不在主程序 CMake 构建链中。
2. Windhawk 的桌面模块提供了更直接的进程内子类化、系统函数 Hook、私有符号 Hook 参考。它们能修改原生桌面的行为，却没有提供完整的分组引擎。

“使用原生 Windows API”“调用 Shell 菜单”“把窗口挂到 WorkerW”“代码里有 Hook”都不能单独证明分组中的图标由 Explorer 原生桌面视图绘制。

## 检索范围和限制

- 使用 GitHub 网页检索及仓库搜索 API，关键词包括 `fences desktop`、`fences alternative`、`desktop organizer windows`、`桌面整理 围栏`、`SysListView32`、`IShellView`、`inject`、`hook`。
- API 按 C#、C++、Rust 和描述检索，每个请求最多取第一页 50 条；存在大量无关结果。一条中文 API 查询遇到编码问题，中文覆盖主要来自网页检索，不能把这条失败查询算作有效覆盖。
- 纳入已知项目、相关 fork、后继项目和新发现的候选；没有把重复 fork 当成独立技术突破。
- 排除仅下载、推广、评测的 GitHub 仓库；商业软件或免费软件在没有对应源代码证据时，不称为开源实现。
- **这是有边界的广泛检索，不能保证覆盖 GitHub 所有仓库。** 下表也不是逐行审计或完整兼容性测试。
- 工作区 `target/fences-survey/` 保存检索结果、提交清单和源码子集。提取时跳过二进制、依赖目录和过大文件；补充提取了 VB6 `.cls`、Avalonia `.axaml` 和 Flutter `.dart`。没有运行仓库脚本、安装软件或给 Explorer 加载 DLL。

## Hook 分层：必须区分的三种实现

| 层次 | 实际能力 | 对 LucidPane 的意义 |
|---|---|---|
| 全局鼠标/键盘 Hook | 接收双击、框选、拖动开始等输入 | 不能单独解决图标绘制、单项隐藏和自动排列 |
| 跨进程桌面访问 | 用 `IFolderView` 或 `LVM_*` 读写位置；为消息参数分配远程内存 | 可以操作真实图标，但不是函数 Hook，也没有自动解决全局排列冲突 |
| Explorer 进程内 Hook | 在 Explorer 地址空间中子类化桌面控件，或拦截系统/内部函数 | 有机会保留原生视图并改变布局、绘制与命中测试；需要新的分组实现和版本验证 |

`WriteProcessMemory` 在多个项目中只是为了传递 `LVITEM`、`POINT`、`RECT` 参数，不能据此认定存在 DLL 注入。`SetWindowSubclass` 如果作用于项目自己的窗口，也不属于接管 Explorer 桌面。

### openFrence：有真实 Hook 代码，但不是完成品

固定提交：`95d3327d685439d58736a6f76b8bc0c490f0810f`。

[cpp/hook/dllmain.cpp](https://github.com/weiweigogo/openFrence/blob/95d3327d685439d58736a6f76b8bc0c490f0810f/cpp/hook/dllmain.cpp#L48) 的实际逻辑是：

- 声明进程内 COM DLL，注册逻辑涉及 `InprocServer32`、`ShellServiceObjectDelayLoad` 等位置，意图由 Explorer 加载；本轮没有验证其在当前 Windows 11 上实际加载成功。
- 找到 `SHELLDLL_DefView` 下的 `SysListView32`。
- 用 `SetWindowLongPtrW(GWLP_WNDPROC)` 替换图标列表窗口过程。
- 收到 `WM_COPYDATA` 时更新围栏矩形。
- 拦截 `LVM_SETITEMPOSITION`，把坐标限制在附近围栏矩形内，再转交原窗口过程。

它没有完成的关键环节：

- 没有按稳定项目身份维护完整分组关系，只按坐标附近的矩形限制位置。
- 这段 Hook 没有展示独立的分组自动排列、折叠后不可命中、分组滚动等完整链路。
- 只拦截一个定位消息，不能推导出 Explorer 内部所有布局路径都被覆盖。
- 没有看到与已安装子类过程对应的完整撤销逻辑；`DllCanUnloadNow` 仅看锁计数，没有充分绑定子类回调和对象生命周期。不能直接搬进生产版本。
- [主 CMakeLists.txt](https://github.com/weiweigogo/openFrence/blob/95d3327d685439d58736a6f76b8bc0c490f0810f/cpp/CMakeLists.txt) 构建的是 `src/render.cpp` 等主程序文件，没有接入 `hook` 子目录。
- [render.cpp](https://github.com/weiweigogo/openFrence/blob/95d3327d685439d58736a6f76b8bc0c490f0810f/cpp/src/render.cpp#L904) 实际绘制选中底板、图标位图和标签；[README](https://github.com/weiweigogo/openFrence/blob/95d3327d685439d58736a6f76b8bc0c490f0810f/README.md) 也明确称菜单为自绘。

判定：**可参考的小型实验，不能作为 Fences 级原生实现已被开源复现的证据。**

### Windhawk：更有价值的原生桌面 Hook 参考

额外核查 `ramensoftware/windhawk-mods` 的三个模块，固定提交 `8dcad63f573216788738142dc36cb982536d970a`。它不计入 38 个类 Fences 应用。

| 模块 | 代码证据 | 能证明什么 |
|---|---|---|
| [classic-desktop-icons](https://github.com/ramensoftware/windhawk-mods/blob/8dcad63f573216788738142dc36cb982536d970a/mods/classic-desktop-icons.wh.cpp#L442) | 目标 `explorer.exe`；Hook `CreateWindowExW`、`CLVDrawManager::_PaintWorkArea`、`CLVSelectionManager::DragSelect`、`CListView::IsDoubleBuffer`、`CDesktopBrowser::SetDesktopWorkAreas` | 可以在保留原桌面控件的前提下改变选框、工作区等行为；不是分组实现 |
| [zen-desktop-toggle-icons](https://github.com/ramensoftware/windhawk-mods/blob/8dcad63f573216788738142dc36cb982536d970a/mods/zen-desktop-toggle-icons.wh.cpp#L279) | 对真实 `SHELLDLL_DefView` 和 `SysListView32` 子类化；Hook 窗口创建，提供卸载处理 | 桌面对象发现、创建后挂接、输入处理和卸载可作参考；功能主要是图标整体切换 |
| [hide-desktop-icon-text](https://github.com/ramensoftware/windhawk-mods/blob/8dcad63f573216788738142dc36cb982536d970a/mods/hide-desktop-icon-text.wh.cpp#L174) | 在桌面绘制上下文中拦截 `DrawTextW`、`DrawThemeTextEx` | 可有条件改变原生标签绘制；不能证明项目级分组、原生菜单和布局问题同时解决 |

这些是具体代码证据，仍需在用户的 Windows 构建上验证。借鉴 Hook 生命周期与符号适配方式，比直接沿用只改坐标的原型更合适。

### Pickets：典型的“有 Hook，但分组仍自绘”

[DesktopHook.cs](https://github.com/Creeptones/Pickets/blob/5fb9aa4c1c84fa088e3f1b6a6f53c454aa41212d/DesktopHook.cs#L39) 用的是 `WH_MOUSE_LL`。

[DesktopIconHider.cs](https://github.com/Creeptones/Pickets/blob/5fb9aa4c1c84fa088e3f1b6a6f53c454aa41212d/DesktopIconHider.cs#L9) 用 `IFolderView.SelectAndPositionItems` 把原图标放到 `(-2000,-2000)`；分组是 WPF 图标界面。代码明确要求关闭自动排列和对齐网格。文件可以不搬家，但不满足用户的自动排列要求。

### win11-desktop-fences：原生图标路线，但依赖关闭自动排列

它使用透明分组框和真实桌面图标坐标管理，是重要参考；然而 [SysListView32Provider.DisableAutoArrange](https://github.com/yuan201644-collab/win11-desktop-fences/blob/24a804cbf898399690fe0d470750aa26b909743e/src/DesktopOrganizer/Win32/SysListView32Provider.cs#L216) 会清除 `LVS_AUTOARRANGE`，折叠仍通过屏外停放图标实现。

这轮读到的新提交修正了屏外坐标超出 16 位范围等问题，不能拿旧提交的 bug 一概描述当前版本；但是关闭自动排列这一架构前提仍然存在。

### desktop-fences：README 的 native 描述需要拆开看

`antonio-abrantes/desktop-fences` 的 [FenceWindow.xaml](https://github.com/antonio-abrantes/desktop-fences/blob/329a47b1e9deececd9d24a976ece3ecf0e63b0e3/src/DesktopFences.App/FenceWindow.xaml#L151) 是 WPF `ItemsControl`。

[DesktopHide.cs](https://github.com/antonio-abrantes/desktop-fences/blob/329a47b1e9deececd9d24a976ece3ecf0e63b0e3/src/DesktopFences.Core/Occupancy/DesktopHide.cs#L14) 对普通文件/快捷方式选择移入存储目录，对回收站、此电脑等命名空间图标选择注册表可见性路线。它的鼠标 Hook 和远程坐标读取都不是原生图标绘制的证明。

## 对 LucidPane 的架构建议

建议把 **Explorer 原生视图 + 最小进程内扩展 + 进程外分组界面** 作为下一轮验证方向。以下是建议设计，并非已在这些项目中找到的完整方案：

- Explorer 继续拥有图标、文字、选中状态、编辑框和常规 Shell 交互；LucidPane 维护分组元数据并绘制背景、标题和动画。
- 分组记录项目身份和所属区域，不依赖移动文件来消除桌面重复项目。
- 未收纳项目保留在原生桌面。不要先全隐藏，再假定能无损恢复原生体验。
- Hook 层尽量只处理必须介入的布局、区域裁剪和输入行为；应用配置、动画计算等放在进程外，避免阻塞 Explorer 的界面线程。
- 真正需要验证的不是“DLL 是否加载”，而是原生视图能否把分组项目从全局自动布局中区分出来，并正确处理刷新、重命名、拖放和可访问性。

**自动排列是核心难点。** 微软文档说明，带 `LVS_AUTOARRANGE` 的列表视图在设置项目坐标后还会排列。因此，仅拦截 `LVM_SETITEMPOSITION`、周期性把图标搬回去，不能作为满足要求的方案。[ListView_SetItemPosition](https://learn.microsoft.com/en-us/windows/win32/api/commctrl/nf-commctrl-listview_setitemposition)

“保留 Windows 原生自动排列机制”与“由 LucidPane 接管排列但表现自动”是两个不同承诺。本轮不把后者偷换成前者。需要先验证原生布局中的项目分流/工作区约束是否足够；若不足，再定位必要的内部布局 Hook。不能预先承诺公开 API 或某一个 Hook 点就能解决。

Windows 11 精简菜单也必须通过真实交互验收：在分组内对真实 Shell 项目打开菜单，执行重命名、属性、第三方命令后仍正确。使用 `IContextMenu` 或画出顶部命令栏，本身都不等于 Explorer 的现代菜单。

最小验证的通过条件：

1. 一个真实桌面图标进入一个分组，路径不变，外观和选中框继续由原生视图提供。
2. 未收纳图标仍可见，用户原先开启的自动排列持续有效；新增项目和刷新不破坏布局。
3. 分组内菜单、重命名和拖放使用正确的项目和位置，没有重复点击层。
4. 收起后图标既不可见也不可被框选/键盘误选；展开恢复原状态。
5. Explorer 重启或扩展撤销后桌面能够恢复，回调不会指向已经卸载的代码。

如果前两项无法验证，就应明确报告该路线的实际限制，而不是继续用自绘或关闭自动排列掩盖问题。进程内错误会影响 Explorer，版本变化也可能改变内部函数；这决定了它应先是独立实验，而不是直接替换当前运行版本。

## 38 个候选的可复核清单

下面按固定提交列出。标为“初筛”的行只说明已读文档或入口，不能据此断言没有其他隐藏路线。代码证据行也仅覆盖所列功能，不代表逐行审计。具体许可证应在复用代码前按文件和依赖核对，公开仓库本身不等于任意可用。

| 仓库 / 固定提交 | 深度 | 实现与限制 | 证据入口 |
|---|---|---|---|
| [Twometer/NoFences](https://github.com/Twometer/NoFences/tree/90374ca17d3d308f135325bc62bfa21d8dde240b) · `90374ca1` | 代码 | WinForms 自绘图标、标签；拖入记录项目引用。 | [源码/文档](https://github.com/Twometer/NoFences/blob/90374ca17d3d308f135325bc62bfa21d8dde240b/NoFences/FenceWindow.cs) |
| [lee-soft/ViPad](https://github.com/lee-soft/ViPad/tree/19273ff5f0ccdddfc68eee4be1fc6287ed82c6f3) · `19273ff5` | 代码 | VB6 启动面板，自行绘制和管理项目；鼠标 Hook 不等于原生桌面分组。 | [源码/文档](https://github.com/lee-soft/ViPad/blob/19273ff5f0ccdddfc68eee4be1fc6287ed82c6f3/LaunchPadItem.cls) |
| [dskiiii/minifence](https://github.com/dskiiii/minifence/tree/22128adb3ed29d04626ba903f7b077b61794baa5) · `22128adb` | 代码 | 整体隐藏原桌面图标，以 WPF 界面承载项目；需要自管未收纳项目。 | [源码/文档](https://github.com/dskiiii/minifence/blob/22128adb3ed29d04626ba903f7b077b61794baa5/MiniFences/MainWindow.xaml.cs) |
| [yuan201644-collab/win11-desktop-fences](https://github.com/yuan201644-collab/win11-desktop-fences/tree/24a804cbf898399690fe0d470750aa26b909743e) · `24a804cb` | 代码 | 原生桌面图标坐标 + 分组覆盖框；关闭自动排列，收起时屏外停放。 | [源码/文档](https://github.com/yuan201644-collab/win11-desktop-fences/blob/24a804cbf898399690fe0d470750aa26b909743e/src/DesktopOrganizer/Win32/SysListView32Provider.cs) |
| [kof2000git/desktop_box](https://github.com/kof2000git/desktop_box/tree/93abe1d8bf67fd69cc2ff465651217a114cb4027) · `93abe1d8` | 代码 | WPF ItemsControl 图标容器；不能用 Shell 接口调用证明原生绘制。 | [源码/文档](https://github.com/kof2000git/desktop_box/blob/93abe1d8bf67fd69cc2ff465651217a114cb4027/src/DesktopBox/Controls/BoxControl.xaml) |
| [Tianyu199509/DeskBox](https://github.com/Tianyu199509/DeskBox/tree/6ffd7aa9baf15e84e20f1bacbc769f06cc9d6f6f) · `6ffd7aa9` | 代码 | WinUI 界面；整理事务明确调用文件移动，文件夹映射是另一种模式。 | [源码/文档](https://github.com/Tianyu199509/DeskBox/blob/6ffd7aa9baf15e84e20f1bacbc769f06cc9d6f6f/src/DeskBox/Services/DesktopOrganizationTransaction.cs) |
| [Walkoud/Palisades](https://github.com/Walkoud/Palisades/tree/b798e394e1bf806e09ff9f0f16f81fdbd2924af3) · `b798e394` | 代码 | 隐藏原图标层，使用自身桌面覆盖界面；与 Xstoudi 同名仓库分别记录。 | [源码/文档](https://github.com/Walkoud/Palisades/blob/b798e394e1bf806e09ff9f0f16f81fdbd2924af3/Palisades.Application/App.xaml.cs) |
| [Xstoudi/Palisades](https://github.com/Xstoudi/Palisades/tree/d779e61c0a62c06518a1f0113d5d71876316469d) · `d779e61c` | 代码 | WPF Shortcuts ItemsControl，由应用承载快捷方式界面。 | [源码/文档](https://github.com/Xstoudi/Palisades/blob/d779e61c0a62c06518a1f0113d5d71876316469d/Palisades.Application/View/Palisade.xaml) |
| [limbo666/DesktopFramesPlus](https://github.com/limbo666/DesktopFramesPlus/tree/ef0edc14ecd7f32323a7a9708bb38596963c16bb) · `ef0edc14` | 代码 | 引用框/目录 Portal；自动整理路径包含 File.Move，不是原生桌面项目过滤。 | [源码/文档](https://github.com/limbo666/DesktopFramesPlus/blob/ef0edc14ecd7f32323a7a9708bb38596963c16bb/Code/Desktop%20Frames/AutoOrganizeManager.cs) |
| [AnTeCP100/SimpleDesktopFence](https://github.com/AnTeCP100/SimpleDesktopFence/tree/7815fba7945e736403d5914f96de8b8e5e074fde) · `7815fba7` | 代码 | 目录面板，WPF ListView 和自定义项目模板。 | [源码/文档](https://github.com/AnTeCP100/SimpleDesktopFence/blob/7815fba7945e736403d5914f96de8b8e5e074fde/SimpleDesktopFence/Views/FolderPanelWindow.xaml) |
| [PinchToDebug/DeskFrame](https://github.com/PinchToDebug/DeskFrame/tree/a9e409079d0de5365fa889622d58ce8ed819ab27) · `a9e40907` | 代码 | WPF 窗口；普通收纳路径会移动文件/目录，另有快捷方式等模式。 | [源码/文档](https://github.com/PinchToDebug/DeskFrame/blob/a9e409079d0de5365fa889622d58ce8ed819ab27/DeskFrame/DeskFrameWindow.xaml.cs) |
| [ZRainbow1275/bentodesk](https://github.com/ZRainbow1275/bentodesk/tree/76d74a4d138b5ddc041f8402b5295a3e5d882696) · `76d74a4d` | 代码 | Rust 自定义界面；stealth 隐藏树方案实际移动文件，有还原路径。 | [源码/文档](https://github.com/ZRainbow1275/bentodesk/blob/76d74a4d138b5ddc041f8402b5295a3e5d882696/crates/bentodesk-backend/src/stealth/hide.rs) |
| [HappyDane/Paddock](https://github.com/HappyDane/Paddock/tree/66f3ca30e78769c8b103177fca43f37c7514705f) · `66f3ca30` | 代码 | WPF 分组；桌面/存储区项目移入分组目录，外部项目可保持引用。 | [源码/文档](https://github.com/HappyDane/Paddock/blob/66f3ca30e78769c8b103177fca43f37c7514705f/src/Paddock.Core/Services/IconStore.cs) |
| [weiweigogo/openFrence](https://github.com/weiweigogo/openFrence/tree/95d3327d685439d58736a6f76b8bc0c490f0810f) · `95d3327d` | 代码 | 有进程内 Hook 实验；主程序仍 Direct2D/DirectWrite 自绘。 | [源码/文档](https://github.com/weiweigogo/openFrence/blob/95d3327d685439d58736a6f76b8bc0c490f0810f/cpp/hook/dllmain.cpp) |
| [Creeptones/Pickets](https://github.com/Creeptones/Pickets/tree/5fb9aa4c1c84fa088e3f1b6a6f53c454aa41212d) · `5fb9aa4c` | 代码 | WPF 图标 + 原图标屏外停放；要求关闭自动排列/网格；WH_MOUSE_LL 监听。 | [源码/文档](https://github.com/Creeptones/Pickets/blob/5fb9aa4c1c84fa088e3f1b6a6f53c454aa41212d/DesktopIconHider.cs) |
| [antonio-abrantes/desktop-fences](https://github.com/antonio-abrantes/desktop-fences/tree/329a47b1e9deececd9d24a976ece3ecf0e63b0e3) · `329a47b1` | 代码 | WPF 图标；文件移入 store，命名空间项目用注册表可见性。 | [源码/文档](https://github.com/antonio-abrantes/desktop-fences/blob/329a47b1e9deececd9d24a976ece3ecf0e63b0e3/src/DesktopFences.Core/Occupancy/DesktopHide.cs) |
| [MrIvoe/Spaces](https://github.com/MrIvoe/Spaces/tree/d19bbfef6808f5a234f898bd90c92e92ce69c88f) · `d19bbfef` | 代码 | Win32 自行绘制项目文字；真实目录收纳并记录原路径。 | [源码/文档](https://github.com/MrIvoe/Spaces/blob/d19bbfef6808f5a234f898bd90c92e92ce69c88f/src/SpaceWindow.cpp) |
| [DcZipPL/Tiels](https://github.com/DcZipPL/Tiels/tree/ca577396326e602fb2d180e0a710d0ba99600835) · `ca577396` | 初筛 | 旧 WPF 项目；README 指向 Avalonia 重写，未证明进程内原生分组。 | [源码/文档](https://github.com/DcZipPL/Tiels/blob/ca577396326e602fb2d180e0a710d0ba99600835/README.md) |
| [DcZipPL/TielsTwo](https://github.com/DcZipPL/TielsTwo/tree/571c29eea635dd501fdaa6ec5d3bb1144e72abd1) · `571c29ee` | 初筛 | 重写项目，Avalonia 界面与 Rust 目录并存；只核查入口，未验证完整收纳链路。 | [源码/文档](https://github.com/DcZipPL/TielsTwo/blob/571c29eea635dd501fdaa6ec5d3bb1144e72abd1/README.md) |
| [NobleMode/DesktopFolders](https://github.com/NobleMode/DesktopFolders/tree/a74c10b67dac2befa7017691d039659dc81a6fd2) · `a74c10b6` | 代码 | WinForms 虚拟文件夹入口自行绘制；具体隐藏链路未深审。 | [源码/文档](https://github.com/NobleMode/DesktopFolders/blob/a74c10b67dac2befa7017691d039659dc81a6fd2/UI/FolderIconForm.cs) |
| [chrisdfennell/OpenFences](https://github.com/chrisdfennell/OpenFences/tree/2ef46569f8baada8f0c5f2bfbe2f39bfeec4cf68) · `2ef46569` | 初筛 | WPF 分组；README 描述创建快捷方式、Auto-Import、整体桌面图标切换。 | [源码/文档](https://github.com/chrisdfennell/OpenFences/blob/2ef46569f8baada8f0c5f2bfbe2f39bfeec4cf68/README.md) |
| [Notbazz12/Universe](https://github.com/Notbazz12/Universe/tree/17737b0bf730243c239d4edc21e5ddfb65da21e5) · `17737b0b` | 初筛 | NoFences 风格 WinForms FenceWindow；未逐项验证新增事务和菜单功能。 | [源码/文档](https://github.com/Notbazz12/Universe/blob/17737b0bf730243c239d4edc21e5ddfb65da21e5/README.md) |
| [Damianttje/Fenceless](https://github.com/Damianttje/Fenceless/tree/4854d3db2815caf730cbff6b2c2fd303809c8a52) · `4854d3db` | 初筛 | WinForms FenceWindow/Rendering 分离的收纳工具；不是仅凭 native 宣传认定原生视图。 | [源码/文档](https://github.com/Damianttje/Fenceless/blob/4854d3db2815caf730cbff6b2c2fd303809c8a52/README.md) |
| [359193585/BestNoFences](https://github.com/359193585/BestNoFences/tree/16944e4df4beae2c492c84345b1746605373e4b0) · `16944e4d` | 初筛 | NoFences/BetterNoFences 衍生，主要扩展屏幕变化和布局恢复。 | [源码/文档](https://github.com/359193585/BestNoFences/blob/16944e4df4beae2c492c84345b1746605373e4b0/README.md) |
| [superlanboy/DesktopFramesPlusKai](https://github.com/superlanboy/DesktopFramesPlusKai/tree/5d492033f62b5995f87945dd26163d9d56f5948c) · `5d492033` | 初筛 | DesktopFramesPlus 衍生；文档描述 Data/Portal 模式，未另算全新架构。 | [源码/文档](https://github.com/superlanboy/DesktopFramesPlusKai/blob/5d492033f62b5995f87945dd26163d9d56f5948c/README.md) |
| [DevPossible/DesktopPossible](https://github.com/DevPossible/DesktopPossible/tree/d0a3b9af05bede6a60571e23fbd90de2ce21fd06) · `d0a3b9af` | 初筛 | Desktop Frames 风格的分组/目录界面；本轮未深审所有文件事务。 | [源码/文档](https://github.com/DevPossible/DesktopPossible/blob/d0a3b9af05bede6a60571e23fbd90de2ce21fd06/README.md) |
| [kuskebabi/BirdyFences](https://github.com/kuskebabi/BirdyFences/tree/29ce2c2aa3c0a956354b8fc3b123236a74894323) · `29ce2c2a` | 初筛 | 仅提取到少量应用入口/项目文件；不足以判定完整实现，不能拿其他 fork 代替该提交。 | [源码/文档](https://github.com/kuskebabi/BirdyFences/blob/29ce2c2aa3c0a956354b8fc3b123236a74894323/README.md) |
| [g2mt/fences](https://github.com/g2mt/fences/tree/ba9db212c13e42fcca7ba07defa4ae3d36879293) · `ba9db212` | 代码 | Rust Win32 图标组件调用 DrawIconEx 并自行绘制标签。 | [源码/文档](https://github.com/g2mt/fences/blob/ba9db212c13e42fcca7ba07defa4ae3d36879293/src/fence/icon.rs) |
| [weizlogy/Fencery](https://github.com/weizlogy/Fencery/tree/4e5949310ce98105fb34d55ef53bff80bcd49cce) · `4e594931` | 代码 | Rust 原生窗口 API + 自己的 graphics/drawing/painter；原生 API 不等于原生桌面视图。 | [源码/文档](https://github.com/weizlogy/Fencery/blob/4e5949310ce98105fb34d55ef53bff80bcd49cce/src/graphics/drawing/painter.rs) |
| [jaimitus/ZenDesktop](https://github.com/jaimitus/ZenDesktop/tree/8371595e24ae8ec2040a7655a8557b25a2291c13) · `8371595e` | 代码 | Rust 自定义 UI，整理规则有 fs::rename；未深审全部模式的桌面可见性。 | [源码/文档](https://github.com/jaimitus/ZenDesktop/blob/8371595e24ae8ec2040a7655a8557b25a2291c13/src/rules.rs) |
| [Fenceify/Fenceify](https://github.com/Fenceify/Fenceify/tree/85f7f4e8d5f302f120859f1ca2a645161570c59c) · `85f7f4e8` | 初筛 | 固定提交只提取到 README/license；README 功能勾选不能作为实现证据。 | [源码/文档](https://github.com/Fenceify/Fenceify/blob/85f7f4e8d5f302f120859f1ca2a645161570c59c/README.md) |
| [Emilien-Etadam/Naultinus](https://github.com/Emilien-Etadam/Naultinus/tree/040d209776c04ea79640bafb61fae6b0a6ec1c1e) · `040d2097` | 初筛 | .NET WPF/WinForms，包含文件夹门户与多种部件；不是专门的原生桌面 Hook 库。 | [源码/文档](https://github.com/Emilien-Etadam/Naultinus/blob/040d209776c04ea79640bafb61fae6b0a6ec1c1e/README.md) |
| [Igrekop/Desktop-Organize-Maxxing](https://github.com/Igrekop/Desktop-Organize-Maxxing/tree/282668cbc89f9899956516942259e5891d14e66f) · `282668cb` | 代码 | 原生坐标整理与隐藏原图标的分组模式并存；定位服务会关闭自动排列。 | [源码/文档](https://github.com/Igrekop/Desktop-Organize-Maxxing/blob/282668cbc89f9899956516942259e5891d14e66f/src/DesktopOrganizeMaxxing/Interop/DesktopListView.cs) |
| [baidu0104/DeskGo](https://github.com/baidu0104/DeskGo/tree/453428cbab7eee2a4a3413446190a6c407d6d31d) · `453428cb` | 代码 | Qt 自定义 FenceWindow；文件转移与原桌面坐标访问并存，不能由后者推导原生绘制。 | [源码/文档](https://github.com/baidu0104/DeskGo/blob/453428cbab7eee2a4a3413446190a6c407d6d31d/src/ui/fencewindow.cpp) |
| [ch998244353/TuckPane](https://github.com/ch998244353/TuckPane/tree/df994df2a2f9db7111f44f138059a2ea17d01a02) · `df994df2` | 初筛 | WPF 真实文件/文件夹 pane，含桌面定位和全局输入辅助；未深审所有模式。 | [源码/文档](https://github.com/ch998244353/TuckPane/blob/df994df2a2f9db7111f44f138059a2ea17d01a02/README.md) |
| [sqmw/desk_tidy](https://github.com/sqmw/desk_tidy/tree/bad4640fba6ed63b1d07d2289957685f25349164) · `bad4640f` | 初筛 | Flutter Windows 项目；已补取 Dart 源码，本轮未完整审阅其文件操作链路。 | [源码/文档](https://github.com/sqmw/desk_tidy/blob/bad4640fba6ed63b1d07d2289957685f25349164/README.md) |
| [duartelcunha/Racks](https://github.com/duartelcunha/Racks/tree/d425eb3aafddaf825154d01a945f042f56162de5) · `d425eb3a` | 初筛 | WPF；README 明确普通拖入会移动到私有存储，另有链接模式。 | [源码/文档](https://github.com/duartelcunha/Racks/blob/d425eb3aafddaf825154d01a945f042f56162de5/README.md) |
| [Nishikinonakai/MacDesk](https://github.com/Nishikinonakai/MacDesk/tree/dbc7d63c1dc822993eba3ef568a6d6d0acb6ac87) · `dbc7d63c` | 初筛 | README 明确自行绘制桌面网格；原生坐标读取用于布局衔接。 | [源码/文档](https://github.com/Nishikinonakai/MacDesk/blob/dbc7d63c1dc822993eba3ef568a6d6d0acb6ac87/README.md) |
