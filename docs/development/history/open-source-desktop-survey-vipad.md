# ViPad 与其他仿 Fences 项目补充调查

> 历史资料：本文记录当时的方案、实验与测量，不代表当前主线能力。旧路径、启动参数和已删除探针仅用于追溯；当前实现见[架构说明](../architecture.md)，可执行命令见[构建与验证](../build.md)。

日期：2026-09-08。只检查源码，没有安装或运行项目。补充上一轮调查，不将任何项目的宣传描述视为已验证的运行兼容性。

## 结论

新增检查 ViPad、NoFences、MiniFences、SimpleDesktopFence、DeskFrame、BentoDesk 六个仓库。ViPad 属于自绘启动面板与整体桌面显隐；BentoDesk 的可选隐藏子目录收纳路线值得单独研究，但会移动真实文件。没有发现可以直接拿来满足全部现有约束的原生桌面按项过滤实现。

## ViPad

- 提交：`19273ff5f0ccdddfc68eee4be1fc6287ed82c6f3`。
- 官方页面链接至 `lee-soft/ViPad`，源码为 VB6。不能把它当作 Shell 原生视图宿主。
- `AddShortcutsOnDesktop` 枚举用户与公共桌面的 `.lnk`，交给 `Form_DragDropFile`。
- 后者解析快捷方式，建立 `LaunchPadItem`，保存目标与参数，读取 HICON 并生成自己的图像资源；部分快捷方式复制到内部 bank。普通文件记录传入路径。这不是将 Explorer 图标窗口搬入 pane。
- 窗口通过自己的图像网格进行绘制。
- `HideDesktopIcons` 切换整个桌面显隐；Windows 8 分支直接对桌面 ListView 调用 `ShowWindow(SW_HIDE)`，其他分支发送桌面命令。退出路径会调用显示桌面图标。
- 可借鉴：分页、标签、图标资源缓存、启动面板组织方式。不能解决保留原生图标层和按项隐藏的组合要求。

证据：[官网](https://lee-soft.com/vipad/)、[导入与图标处理](https://github.com/lee-soft/ViPad/blob/19273ff5f0ccdddfc68eee4be1fc6287ed82c6f3/ViPickWindow.frm#L594)、[批量导入](https://github.com/lee-soft/ViPad/blob/19273ff5f0ccdddfc68eee4be1fc6287ed82c6f3/ViPickWindow.frm#L2427)、[整体显隐](https://github.com/lee-soft/ViPad/blob/19273ff5f0ccdddfc68eee4be1fc6287ed82c6f3/ProgramSupport.bas#L539)。

## NoFences

- 提交：`90374ca17d3d308f135325bc62bfa21d8dde240b`。
- `FenceWindow_DragDrop` 将路径加入 `fenceInfo.Files` 并保存，没有在这个收纳处理程序中移动文件或隐藏原图标。
- 图标与文字通过 `Graphics.DrawIcon` / `DrawString` 自绘，文字阴影与选择框也由项目绘制。
- 因此文件仍在桌面时，单纯收纳不会消除原桌面的重复显示。拖放反馈设置为 Move 不能当作物理移动已经发生的证据。

证据：[收纳路径](https://github.com/Twometer/NoFences/blob/90374ca17d3d308f135325bc62bfa21d8dde240b/NoFences/FenceWindow.cs#L183)、[自绘](https://github.com/Twometer/NoFences/blob/90374ca17d3d308f135325bc62bfa21d8dde240b/NoFences/FenceWindow.cs#L384)。

## MiniFences

- 仓库：`dskiiii/minifence`，提交 `22128adb3ed29d04626ba903f7b077b61794baa5`。
- DesktopGroup 保存分组归属；Folder Portal 的真实文件操作是另外的路径。
- `DesktopIconLayoutService.SetVisible` 对桌面 ListView 整体调用 `ShowWindow`；`MainWindow` 实际调用 `SetVisible(false)`，由 WPF 层显示桌面项目。
- Shell 菜单、拖放协议、文件操作记录与恢复机制可以作为组件参考。它没有保留 Explorer 原图标层来完成分组。

证据：[显隐服务](https://github.com/dskiiii/minifence/blob/22128adb3ed29d04626ba903f7b077b61794baa5/MiniFences/Services/DesktopIconLayoutService.cs#L14)、[隐藏调用](https://github.com/dskiiii/minifence/blob/22128adb3ed29d04626ba903f7b077b61794baa5/MiniFences/MainWindow.xaml.cs#L2184)。

## SimpleDesktopFence

- 提交：`7815fba7945e736403d5914f96de8b8e5e074fde`。
- 目录面板：ViewModel 枚举配置目录里的文件与子目录；XAML 绑定列表和图标，使用自己的 WPF UI。
- 源码范围内未发现原桌面按项隐藏或宿主 Explorer 原生视图。适合作为 Folder Portal 参考。

证据：[目录枚举](https://github.com/AnTeCP100/SimpleDesktopFence/blob/7815fba7945e736403d5914f96de8b8e5e074fde/SimpleDesktopFence/ViewModels/FolderPanelViewModel.cs#L291)、[文件列表](https://github.com/AnTeCP100/SimpleDesktopFence/blob/7815fba7945e736403d5914f96de8b8e5e074fde/SimpleDesktopFence/Views/FolderPanelWindow.xaml#L188)。

## DeskFrame

- 提交：`a9e409079d0de5365fa889622d58ce8ed819ab27`。
- WPF `ItemsControl` 显示后端目录的项目。
- 拖入处理区分普通模式与 `IsShortcutsOnly`：普通分支调用 `File.Move` / `Directory.Move`；快捷方式分支调用 `CreateShortcut`。空面板还存在绑定目录的处理，不应把所有拖入统一描述为移动。
- 快捷方式模式不会自动消除原始文件的桌面入口；移动模式改变文件路径。

证据：[拖入操作](https://github.com/PinchToDebug/DeskFrame/blob/a9e409079d0de5365fa889622d58ce8ed819ab27/DeskFrame/DeskFrameWindow.xaml.cs#L2645)、[自建项目控件](https://github.com/PinchToDebug/DeskFrame/blob/a9e409079d0de5365fa889622d58ce8ed819ab27/DeskFrame/DeskFrameWindow.xaml#L240)。

## BentoDesk：本轮值得单独比较的路线

- 提交：`76d74a4d138b5ddc041f8402b5295a3e5d882696`。
- Rust 原生 Windows 应用，但原生程序并不等于 Explorer 的原生桌面图标层。
- `item_persistence::hide_item_file` 检查 `stealth_enabled`：关闭时保留原路径；开启且配置有效时调用后端 `hide_file`。
- 后端把文件移入桌面下 `.bentodesk/{zone_id}/`；目录使用 `HIDDEN | SYSTEM | NOT_CONTENT_INDEXED` 属性。这里是文件系统收纳，不是视图过滤。隐藏属性也不是安全访问控制，显示结果仍取决于系统/应用设置。
- 保存 original/hidden 路径，并有 manifest、镜像与恢复逻辑；manifest 写入失败时还尝试回滚移动。不能据此保证任何故障都无损恢复。
- 对 LucidPane 的价值：如果用户接受改路径，这条路线能解释如何保留其他原生桌面项，并让已移动项不再位于桌面根目录。代价是路径变化、自建 pane 图标区，以及恢复和外部文件引用处理。
- 自动排列边界：文件收纳本身无需摆放原生图标；但项目的独立图标布局恢复代码会清除自动排列/网格标志后写回坐标。因此不能声称该项目所有操作都保持 Windows 自动排列设置不变。

证据：[开关与调用链](https://github.com/ZRainbow1275/bentodesk/blob/76d74a4d138b5ddc041f8402b5295a3e5d882696/crates/bentodesk-shell/src/shell/item_persistence.rs#L58)、[真实移动](https://github.com/ZRainbow1275/bentodesk/blob/76d74a4d138b5ddc041f8402b5295a3e5d882696/crates/bentodesk-backend/src/stealth/hide.rs#L322)、[属性与恢复设计](https://github.com/ZRainbow1275/bentodesk/blob/76d74a4d138b5ddc041f8402b5295a3e5d882696/crates/bentodesk-backend/src/stealth/mod.rs)、[布局恢复改变自动排列](https://github.com/ZRainbow1275/bentodesk/blob/76d74a4d138b5ddc041f8402b5295a3e5d882696/crates/bentodesk-backend/src/icon_positions/writer.rs#L59)。

## 对现有调查的补充

本轮的实质新增是 BentoDesk 的可选隐藏子目录收纳链路。ViPad、NoFences、MiniFences 等扩展了样本，但没有证明存在无需文件移动的原生桌面按项过滤捷径。BentoDesk 使用 `IShellView` / `IFolderView` 的位置读取恢复链路也不能误认成 pane 内宿主系统视图。

如果后续讨论接受路径改变，应重点比较“隐藏目录 + 自建图标区”和“受管目录 + 系统 Shell 视图”两种候选。后一种仍需要原型验证材质、透明背景及与桌面外观的一致性，本次项目调查未证实它已经满足这些要求。
