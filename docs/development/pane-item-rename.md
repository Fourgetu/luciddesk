# 图标菜单与重命名

图标菜单继续由 Explorer 托管。请求菜单时加入 `CMF_CANRENAME`，让支持改名的 Shell 项显示原生命令。

菜单入口保留鼠标/键盘来源。鼠标右键先向 `SHELLDLL_DefView` 转发 `WM_RBUTTONDOWN`，再投递带屏幕坐标的 `WM_CONTEXTMENU`；不向 ListView 转发鼠标消息，不发送右键抬起，避免重新命中其他图标或打开两次菜单。目标仍先通过 Shell 身份核验并单独选中，菜单生命周期继续由 Explorer 窗口事件观察器跟踪。键盘入口保留接口调用和 pane 菜单锚点。

本机 Shell 符号及对应实现显示：`CDefView::_OnContextMenu` 对非 `-1` 坐标设置鼠标来源状态；`DoCuratedContextMenu` 将其转换成 presenter 标志 `8`。菜单 presenter 在来源掩码 `0xE` 为零时自动显示访问键。直接调用 `DoContextMenuPopup` 会绕过该初始化；仅设置 `WM_CHANGEUISTATE / UISF_HIDEACCEL` 不能解决 XAML 浮标问题，因此移除了该尝试。生产代码只发送窗口消息，不读写私有字段、不调用私有 ABI。此入口依赖 Explorer 宿主行为，Windows 更新后需复测；灰色浮标消失及命令目标正确性仍需实际菜单验证。

Pane 与桌面采用互斥选中：点击、键盘操作或开始重命名 pane 图标时，通过 Hook 清除原生桌面项的 `LVIS_SELECTED | LVIS_FOCUSED`。菜单期间仍允许 Explorer 临时选中隐藏目标；菜单结束后清除该临时状态，不恢复旧桌面选择。仅在明确的 pane 输入及菜单边界执行，不改变桌面拖动和命中逻辑，菜单内部焦点切换也不会提前清除命令目标。

普通 pane 输入使用不带指针的异步注册消息通知 Hook，选中效果不再等待 `SendMessageTimeout` 返回。Hook 只在前台仍属于控制端进程时执行，防止延迟消息清掉用户随后在桌面上的新选择；系统菜单边界继续同步清理，以确保隐藏目标选择时序。

焦点日志确认 `SetForegroundWindow` 曾成功返回 pane，但后续 `IFolderView2::SelectItem(... SVSI_FOCUSED)` 又激活了桌面。因此仅交换调用顺序不足以解决闪烁；pane 菜单必须停止恢复旧选择。无 pane 所属窗口的独立诊断菜单保留原来的选择恢复行为。

菜单目标查找优先使用现有桌面清单发布的索引提示，再用 Shell 身份比较核验该索引。提示失效则重新遍历；不直接信任排序前的索引。缓存仅含名称与索引，不跨线程共享 COM 对象，也不持有锁进行 COM 调用。回退遍历复用一次取得的父文件夹对象。

只读基准 `desktop_menu_service_probe --benchmark-resolve` 覆盖首、中、末尾图标及错误索引回退。本机 81 项示例中，末尾项遍历约 141 ms，索引核验约 3 ms；这是目标查找耗时，不等于系统菜单整体弹出时间。`LUCIDPANE_MENU_PERF=1` 可记录连接、目标核验、选择、菜单对象取得及首次观察到菜单的分段时间，关闭后才一次写入 `menu-performance.log`，不把用户浏览菜单的时间当作打开耗时。

被 pane 收纳的图标仍是隐藏的桌面 Shell 项。如果直接让 Explorer 开始内联编辑，编辑框会落在隐藏的桌面位置。因此 Hook 仅在本程序的菜单会话中拦截隐藏项的 `LVN_BEGINLABELEDIT`，取消该编辑框并保留一次性的重命名请求。菜单关闭和临时桌面选择清理完成后，控制端读取该请求，为当初右键的准确身份创建原生 EDIT 弹出窗口，覆盖 pane 中该图标的文字位置。F2 使用同一入口。

编辑框使用系统桌面字体、居中文本、白底及原生选中文字效果，原绘制文字在编辑期间隐藏。尺寸随文本换行调整，位置使用绘制相同的图标网格、滚动位置和 DPI，随所属 pane 移动；编辑期间不自动收起 pane。

编辑框采用 pane 拥有的 `WS_POPUP | WS_EX_TOOLWINDOW` 原生 EDIT 窗口，而非子窗口：pane 使用 `WS_EX_NOREDIRECTIONBITMAP` 和 DirectComposition，普通 GDI 子窗口不能依赖它提供可见的重定向表面。编辑窗口独立合成、不显示任务栏按钮，以屏幕坐标跟随标签，随 pane 销毁。它不是置顶窗口。

隐藏扩展名的快捷方式只编辑显示名称，提交时保留原扩展名；显示扩展名的文件预选扩展名前的部分。Enter 确认、Esc 取消，失去焦点时确认；输入法组词期间不把 Enter 当成完成重命名。确认后使用 `IFileOperation::RenameItem`，让 Shell 处理实际改名、冲突和通知。现有清单同步通过文件身份保留分组归属并更新路径和标题。未隐藏的桌面图标继续使用 Explorer 原本的内联改名。

验证：

- 原生 EDIT 的所属窗口、独立合成样式、标签坐标、pane 移动跟随、中文预选、Esc 取消清理、隐藏扩展名保留。
- 测试临时文件的 Shell 菜单包含 `rename` verb；中文改名后内容和文件身份不变。
- 核心模型改名后保留分组归属。
- Hook 隐藏项编辑被拦截、请求仅消费一次、可见项编辑不被拦截。
- 原有首次拖动、隐藏排序和插入分隔符几何回归检查通过。
