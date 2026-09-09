# 整体隐藏与自绘桌面

2026-09-08。用户重新确认采用自绘方案后，实现独立桌面图标层，并复用现有 Acrylic/Mica 分组。

## 使用

```powershell
cargo run -p lucidpane --release -- --desktop
# 不接管桌面的独立预览
cargo run -p lucidpane --release -- --preview
# 异常后恢复原生桌面
.\target\release\lucidpane.exe --restore-shell
```

无参数等同 `--desktop`。新模式使用 `redrawn-desktop.db`，保留原有 `preview.db` 和历史模式的数据。
拖入分组只修改数据库归属，不搬迁、删除或创建快捷方式，也不修改 Explorer 的自动排列或对齐网格选项。

未收纳图标由每个显示器的透明窗口绘制，初次采用 Explorer 的坐标；窗口区域只包含图标单元格，空白处继续交给桌面。
图标进入 pane 后从自由桌面层移除，其余图标原地保留。拖回桌面寻找可见的空闲网格，不挤出屏幕；放不下时取消此次拖动。
新发现的项目优先采用原生位置，冲突时寻找空闲位置，桌面全满时进入第一个可滚动分组。

## 视觉实现

- 启动最早设置 Per Monitor V2 DPI 感知，再读取显示器和 Explorer。实机对照发现晚初始化会使自由图标的坐标和尺寸采用不同缩放；已修正。
- `IFolderView2` 提供图标大小、间距、项目坐标和实际显示名称。当前桌面样本为 150% 缩放，图标视图大小 48，间距 112×147 屏幕像素。
- Shell 提供图标和缩略图；快捷方式从系统 ImageList 提取基础图标，不叠加左下角箭头，保留快捷方式自定义图标。这只影响 LucidPane 自绘层，不修改 Explorer 的全局角标设置。系统标题字体来自 `SPI_GETICONTITLELOGFONT`。
- 选中、悬停及失焦选中改为参考原生截图校准的中性半透明浅色矩形，使用小圆角和预乘 alpha。原先 `Explorer::ListView` 的文件夹蓝色选中样式不适合桌面，已移除。
- 选中框高度按图标与实际文字行高计算，不再占用整行网格及下方间隙。一行文字约 69 DIP，两行约 85 DIP（具体随系统字体变化）。
- 图标文字使用 `SystemParametersInfoForDpi(SPI_GETICONTITLELOGFONT)` 的完整 LOGFONT 和 GDI 换行度量，生成白色字形蒙版与柔和黑色阴影；以物理像素对齐，避免旧版单次偏移文字造成的生硬重影。系统主题或设置变更会重建渲染缓存。
- 自由桌面与 pane 使用同一套图标布局、命中测试及渲染。图标保持不透明，只有材质层透明。
- 图标右键通过 `IContextMenu`，转发 `IContextMenu2/3` 的菜单消息，打开/复制/属性等由 Shell 执行。

这里不能据此承诺所有机器、主题和图标的像素级 1:1。透明字形使用灰度抗锯齿，文字阴影与选中框为自绘校准；标签截断、部分角标和缩略图仍可能区别于 Explorer。尚未完成高对比度和混合 DPI 的完整视觉验收。没有把原生图标截图当成可交互替身。

## Windows 11 精简菜单调查

用户要求的是 Explorer 原生精简菜单，包括顶部操作图标。当前发布入口仍是系统经典 `IContextMenu`，精简菜单尚未实现。

试验曾通过 `IFolderView2` 找到并选择目标项目，再向隐藏的桌面 ListView 发送键盘形式的 `WM_CONTEXTMENU`。实机弹出的是桌面背景菜单，目标上下文不正确，因此撤下该入口。向父视图转发的变体也未通过验收，不保留在发布路径。后续 UI 验证工具报 `foreground window did not report a process id`，未据此宣称精简菜单可用。测试进程已退出，守护进程完成恢复。

微软的 [现代菜单扩展文档](https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/integrate-packaged-app-with-file-explorer) 描述 `IExplorerCommand` 加应用身份向 Explorer 菜单增加命令，并未提供把整个精简菜单托管进任意自绘窗口的入口。不能把注册菜单扩展等同于宿主整个菜单。[IContextMenuSite::DoContextMenuPopup](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-icontextmenusite-docontextmenupopup) 已被官方标记为不可用，也不作为新入口。

这不是所有技术路线均不可能的结论；后续仍需验证 Explorer 菜单宿主或更深的 Shell 集成。自绘一个带顶部按钮的菜单可以模仿外观，但不等于已经接通用户要求的原生精简菜单。

此次自动验证增加系统字体在 96/144/192 DPI 下的一行/两行高度、透明背景、白色字形及预乘 alpha 检查，以及中性选中框的高度和交互状态检查。实机进行了 150% 桌面显示观察，没有完成所有缩放和所有交互的视觉验收。

## 生命周期与边界

### 绘制与缓存修复（2026-09-08）

主窗口从 CPU WIC 全窗栅格化、`CopyPixels`、整幅上传，改为 `ID2D1DeviceContext` 直接绘入 DXGI 后备缓冲区。4K 主窗口不再每帧分配和复制约 32 MB 的 CPU 位图。折叠/缩放期间沿用同一个设备上下文和图标/文字缓存，窗口尺寸变化只重建后备缓冲区；小型自绘菜单仍可使用 WIC 路径。

图标右键关闭不再发送清空缓存的 Refresh。F5 保留旧图片，待后台读取成功后替换；正在扫描时请求的强制重载会排入下一轮。主题变化只重建绘制资源，不请求全量 Shell 图片重载。模型同步会比较身份、名称、位置和图片引用，没有变化的窗口不重绘。GPU 图片按稳定身份更新，避免每次换图片都追加一份旧缓存。

新增实际 GPU 后备缓冲区读回测试，校验多帧、尺寸变化后的不透明图标颜色、透明背景及主窗口无 CPU 帧位图；新增扫描期间刷新保留旧图片的回归测试。

Release 手动基准：3840×2160、200 个合成图标、150% 缩放、隐藏测试窗口、预热后 24 帧，包含 Present 的调用耗时中位数，旧 CPU 位图加上传路径为 8.35 ms，GPU 直接绘制为 6.95 ms；P95 分别为 8.68 ms / 7.11 ms。这里受 Present 节拍影响，不能作为实际桌面端到端延迟或帧率结论。重现：`cargo test -p lucidpane --release benchmark_4k_desktop_submission -- --ignored --nocapture`。

独立配置启动成功并取得首帧就绪后的桌面租约，测试后退出恢复。computer-use 两次观察均报 `foreground window did not report a process id`，本轮没有完成实际鼠标拖动和菜单开关的视觉验收。Windows 11 精简菜单仍未实现。

全量枚举、图标加载和所有窗口首帧成功之后才取得桌面隐藏租约。初始化失败保留原生桌面。
正常退出先恢复 Explorer，再销毁替代层。独立恢复进程监视主进程退出；接管标记也支持下一次启动恢复。
Explorer 重启或显示配置变化时退出接管并恢复原生桌面，重新启动后读取新配置。

每三秒重新读取 Shell 清单，保留已保存归属；新建、删除和重命名会同步。F5 请求重载图像。
拖动时记录稳定身份，避免列表刷新后把相同索引的另一个文件移入分组。未知目标及满屏放置不改变归属。

当前交互边界：单项选择、双击/Enter 打开、内部拖放、经典 Shell 菜单；框选、多选、F2 内联重命名、应用外 OLE 拖放和完整 UI Automation 尚未实现。

## 验证

实机检查：150% 缩放下原生与自绘对照；修正图标尺寸与间距；桌面图标拖入 pane 后其他图标保持原位；Acrylic 分组显示；结束测试后恢复原生桌面。
追加检查：经典 Shell 右键菜单及第三方扩展项成功展开；通过 pane 菜单正常退出后接管标记消失、进程退出；强制结束主进程后守护进程也完成还原并退出。窗口库会暂时卸载正在执行的消息回调，因此右键菜单改由异步消息打开，并显式处理系统关闭命令。
当前 UI 测试工具限制拖动终点必须在源窗口内，因此跨出 pane 的人工拖动尚未由该工具完成；布局与容量处理另有自动测试。

```powershell
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build -p lucidpane --release
```

集成测试读取 Explorer，需在普通交互式 Windows 桌面运行，受限沙箱可能拒绝 COM 访问。
测试覆盖自由图标拖入 pane 不改变兄弟项目、归属持久化、发现新项目的冲突处理、屏幕满时的可访问性、移动自身的位置和首帧未完成时禁止接管。
上述完整测试及 Clippy 严格检查通过，Release 构建成功；测试结束时已退出接管并恢复原生桌面。

开发者可设置 `LUCIDPANE_DATA_DIR` 使用独立测试数据库。`LUCIDPANE_INSPECT=1` 暂时将窗口暴露为普通应用窗口便于观察，`native` 则不隐藏原生图标并清空替代层内容用于对照；不要作为日常启动配置。
