# 开源桌面分组实现调查

调查日期：2026-09-08。方法：检出以下仓库源码，检查图标显示、桌面显隐、布局和文件操作路径；未安装、运行这些项目，未改变用户桌面。结论限于下面的提交，不代表运行兼容性已经验证。

用户约束：保留 Windows 自动排列，尽量保持原生桌面图标外观和交互，增加分组与材质；不能把隐藏整个桌面并重绘所有图标直接当作已接受的方案。

## 结论

这次检查的五个项目没有提供已确认同时满足“原生图标层、自动排列保持开启、已分组项单独隐藏”的现成实现。

源码揭示的路线主要是：移动原生图标坐标；自建图标控件并整体隐藏桌面；自建图标控件并显示真实目录。获取 Shell 图标或使用 Shell 菜单，不代表使用 Explorer 的原生图标视图。

## 1. win11-desktop-fences：移动原生图标，关闭自动排列

- 仓库：https://github.com/yuan201644-collab/win11-desktop-fences
- 提交：`b8169e37932cbbc61099db9c6908c034158eaab3`
- 文件仍在原路径，通过桌面 `SysListView32` 读写图标坐标；分组框由 WPF 提供。
- `DisableAutoArrange()` 使用 `SetWindowLong` 清除 `LVS_AUTOARRANGE`，控制器实际调用该方法，并非未使用的辅助代码。
- 收起分组时将图标停放到屏幕外，`ParkBase = -32000`，展开时恢复坐标。代码还处理坐标截断和图标滞留的恢复。
- 判断：原生图标路线的具体参考，但直接违反用户不关闭自动排列的要求；不能作为 LucidPane 默认架构。

源码：[自动排列与定位](https://github.com/yuan201644-collab/win11-desktop-fences/blob/b8169e37932cbbc61099db9c6908c034158eaab3/src/DesktopOrganizer/Win32/SysListView32Provider.cs#L212)、[分组控制器](https://github.com/yuan201644-collab/win11-desktop-fences/blob/b8169e37932cbbc61099db9c6908c034158eaab3/src/DesktopOrganizer/Services/FenceOverlayController.cs#L426)、[屏外停放](https://github.com/yuan201644-collab/win11-desktop-fences/blob/b8169e37932cbbc61099db9c6908c034158eaab3/src/DesktopOrganizer.Core/Layout/FenceClusterBuilder.cs#L35)。

## 2. desktop_box：保存引用，自建图标控件，整体隐藏桌面

- 仓库：https://github.com/kof2000git/desktop_box
- 提交：`93abe1d8bf67fd69cc2ff465651217a114cb4027`
- 普通收纳按项目说明保存引用而不移动文件；盒子由 WPF `ItemsControl` / `WrapPanel` 显示项目。
- `DesktopIconsService` 向 `SHELLDLL_DefView` 发送 `WM_COMMAND / 0x7073`，并写入 `HideIcons`。这是整体显隐，不是按分组过滤桌面项。
- 原生右键菜单在独立 C++ 辅助 EXE 中实现：`SHParseDisplayName`、`GetUIObjectOf(IContextMenu)`、`IContextMenu2/3` 与菜单消息处理。
- 判断：菜单辅助进程值得借鉴；其整体显隐方式不满足保留原生桌面的要求。源码中的私有命令值不能当作跨 Windows 版本的接口保证。

源码：[桌面显隐](https://github.com/kof2000git/desktop_box/blob/93abe1d8bf67fd69cc2ff465651217a114cb4027/src/DesktopBox/Services/DesktopIconsService.cs)、[盒子控件](https://github.com/kof2000git/desktop_box/blob/93abe1d8bf67fd69cc2ff465651217a114cb4027/src/DesktopBox/Controls/BoxControl.xaml#L150)、[Shell 菜单](https://github.com/kof2000git/desktop_box/blob/93abe1d8bf67fd69cc2ff465651217a114cb4027/src/DesktopBox.ShellMenu/DesktopBox.ShellMenu.cpp#L173)。

## 3. DeskBox：WinUI 3 控件，真实目录与映射

- 仓库：https://github.com/Tianyu199509/DeskBox
- 提交：`a02bdca3585741418d7840e650b307b8eb06f1c3`
- 文件区使用自定义 WinUI `GridView` / 项目模板，不是嵌入 Explorer 原生桌面视图。
- 映射现有目录时不搬目录；托管文件组件有实际后端目录。
- “整理桌面”是另一条操作路径：`DesktopOrganizationTransaction` 创建源/目标计划并调用 `_transfer.MoveAsync`，确实移动文件。不能将“映射不移动”推广为“所有整理均不移动”。
- 实现中有预览、操作记录、部分完成结果处理和回滚逻辑；这些降低整理失败的影响，但不会让物理路径变化变成不存在。
- 材质、原生拖放桥接、系统菜单和文件操作分别有独立模块，适合研究功能边界。
- 判断：与 LucidPane 的材质和交互目标最相关的组件参考；不是保留原生图标层的答案。若以后实际复用代码，需遵循其 GPL-3.0 许可证；本次未复制代码。

源码：[文件区 GridView](https://github.com/Tianyu199509/DeskBox/blob/a02bdca3585741418d7840e650b307b8eb06f1c3/src/DeskBox/Controls/WidgetContents/FileSurfaceContent.xaml#L579)、[整理事务](https://github.com/Tianyu199509/DeskBox/blob/a02bdca3585741418d7840e650b307b8eb06f1c3/src/DeskBox/Services/DesktopOrganizationTransaction.cs#L145)、[材质](https://github.com/Tianyu199509/DeskBox/blob/a02bdca3585741418d7840e650b307b8eb06f1c3/src/DeskBox/Views/WidgetWindowBase.Backdrop.cs)、[拖放桥接](https://github.com/Tianyu199509/DeskBox/blob/a02bdca3585741418d7840e650b307b8eb06f1c3/src/DeskBox/Controls/NativeShellFileDragProvider.cs)。

## 4. Palisades：自建桌面覆盖层，隐藏原生 ListView

- 仓库：https://github.com/Walkoud/Palisades
- 提交：`b798e394e1bf806e09ff9f0f16f81fdbd2924af3`
- 启动代码显示自己的 overlay，随后调用 `HideDesktopIcons()`。
- 该方法查找桌面 `SysListView32` 并 `ShowWindow(SW_HIDE)`。
- `DesktopOverlayWindow.AddIconElement` 创建 WPF `Image`、`TextBlock` 和 `StackPanel`，自行显示图标和文字。
- 判断：即使项目描述为桌面覆盖层，也不能据此认为其继续使用原生图标；这里的启动调用链明确属于整体隐藏和自建显示路线。

源码：[启动调用](https://github.com/Walkoud/Palisades/blob/b798e394e1bf806e09ff9f0f16f81fdbd2924af3/Palisades.Application/App.xaml.cs#L209)、[隐藏桌面](https://github.com/Walkoud/Palisades/blob/b798e394e1bf806e09ff9f0f16f81fdbd2924af3/Palisades.Application/Services/DesktopService.cs#L100)、[自建图标](https://github.com/Walkoud/Palisades/blob/b798e394e1bf806e09ff9f0f16f81fdbd2924af3/Palisades.Application/Views/DesktopOverlayWindow.xaml.cs#L664)。

## 5. DesktopFramesPlus：独立分组、Portal 与可选整体显隐

- 仓库：https://github.com/limbo666/DesktopFramesPlus
- 提交：`ef0edc14ecd7f32323a7a9708bb38596963c16bb`
- Data Frame 管理快捷入口，Portal Frame 显示目录内容，不能将两者视为同一种存储操作。
- `DesktopIconManager.SetDesktopIconsVisible` 对整个桌面 ListView 调用 `ShowWindow`，不是隐藏指定图标。
- `AutoOrganizeManager` 中有真实 `File.Move(sourcePath, destPath)`；自动整理与单纯显示入口的语义不同。
- 判断：可参考分组产品能力和 Portal 组织方式；未发现能直接解决原桌面按项隐藏的机制。

源码：[整体显隐](https://github.com/limbo666/DesktopFramesPlus/blob/ef0edc14ecd7f32323a7a9708bb38596963c16bb/Code/Desktop%20Frames/DesktopIconManager.cs#L91)、[自动整理移动文件](https://github.com/limbo666/DesktopFramesPlus/blob/ef0edc14ecd7f32323a7a9708bb38596963c16bb/Code/Desktop%20Frames/AutoOrganizeManager.cs#L225)。

## LucidPane 的下一步判断

1. 不回到“关闭自动排列并移动原生图标”的已否定方案。
2. 分别借鉴 DeskBox 的材质/拖放边界、desktop_box 的菜单辅助进程、真实文件操作项目的恢复记录，不能据此承诺原生桌面外观一致。
3. 在所检查源码中未找到 `IExplorerBrowser` / `IResultsFolder` / `IShellView` 宿主实现，也未找到通过 `RemoveObject` 持久过滤原桌面的实现；这是本次样本范围内的检索结果，不是对所有开源项目的结论。
4. “系统 Shell 视图承载 pane”和“原桌面按项隐藏”仍应分别做原型验证。前者解决 pane 的系统交互复用，后者才决定不搬文件、无重复显示、保留原生桌面能否同时成立。
5. 若用户最终接受物理收纳，可把“仅移动快捷方式”作为更窄的备选，并显式区分真实文件移动和目录映射；本次调查不改变当前产品行为。
