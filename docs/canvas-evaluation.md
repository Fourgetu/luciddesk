# Canvas 接入验证（2026-09-11）

约束：Rust + Win32，不依赖 Windows App SDK，不引入 Reactor。

## 图标清晰度与跨 pane 拖动修复

- 混合模式请求至少 128 像素源图。快捷方式优先通过 IExtractIconW 请求指定尺寸；图像列表回退优先选择足够大的尺寸，避免旧排序选到较小位图。
- 图标按目标物理像素尺寸预缩放：缩小使用 WIC Fant，放大使用高质量三次插值；保持预乘 BGRA。Canvas 中以物理像素对齐的坐标和尺寸绘制，避免居中时落在半像素产生二次滤波。位图缓存包含显示尺寸，DPI 变化会重建。
- pane 的 OLE 目标接入 IDropTargetHelper 的 DragEnter/DragOver/DragLeave/Drop，把源 IDataObject、真实屏幕坐标和接受效果传给系统，以保持 Explorer 的拖动图标和名称。文件操作仍只接受链接式收集，不要求移动源文件。
- 新增高频图标在 100%/125%/150%/200% DPI 下的逐像素验证，以及拖动助手完整生命周期和负屏幕坐标验证。
- `app/examples/icon_quality_probe.rs` 可从真实快捷方式输出 72 像素 BMP 供视觉检查。LocalSend、LM Studio、像素蛋糕、委托交易均成功取得 128×128 源图，输出图已检查。此检查不替代真实桌面的跨 pane 拖动验证。

修复构建输出为 `target/debug/lucidpane.exe`，同目录包含 `desktop_hook.dll`。运行中的 `target/canvas-next/debug/lucidpane.exe` 不会因编译自动更新。

## 第二阶段：具体绘制迁移

设置页、菜单和分组的画刷、填充、边框、圆角、线条、图标位图和普通文字已改为 Canvas 绘制。图标、原生标签栅格图及选中态图片使用 Canvas Bitmap 缓存；菜单与离屏测试改用 Canvas RenderTarget，原来的 WIC 绘制目标已移除。

新增 `app/src/preview/canvas.rs` 作为小型兼容层：

- 保留现有 DIP 布局类型，集中转换 Canvas 几何类型。
- 每帧只创建一个借用 DrawingSession，并显式检查 EndDraw。
- Canvas 普通文字绘制没有自动裁剪，因此在文字范围内压入/弹出裁剪区域。
- 用 RAII 在中途错误退出时清理裁剪栈并结束绘制，避免污染下一帧。
- 字体由 Canvas 创建；省略号、对齐和换行配置继续通过 DirectWrite 原生接口处理。

仍保留 Shell 图标提取、桌面标签栅格化与阴影、原生编辑框，以及亚克力/Mica 背景和 DirectComposition 挂载。这些不属于本阶段替换范围。

验证：`cargo test -p lucidpane --offline --target-dir target/canvas-next -- --test-threads=1`，共 54 passed、3 ignored。新增文字裁剪、绘制失败后的恢复、菜单透明度/悬停/缩放测试；原有设置、分组 GPU 像素、图标缓存与窗口生命周期测试通过。检查了测试生成的 settings-dark.bmp 和 settings-light.bmp，未见明显布局和文字错位。检查不等于真实桌面材质和多显示器交互验证。

新程序构建于 `target/canvas-next/debug/lucidpane.exe`，因为当前运行实例占用默认的 `target/debug/lucidpane.exe`。此次未终止、重启或另开混合模式实例。Clippy 可执行通过，但原有项目警告仍存在。

## 第一阶段接入结果（历史记录）

Canvas 已从开发依赖提升为生产依赖。共享 `Surface` 使用 `GpuDevice::new_or_warp`、合成 `SwapChain` 和 Canvas 持有的 Direct2D 上下文，应用于分组、设置和菜单。Canvas 管理设备、缓冲区绑定、缩放及呈现；原生 DirectComposition 挂载和亚克力/壁纸材质保持原实现。

现有 Renderer 仍直接调用 D2D/DWrite/WIC，明确检查 BeginDraw/EndDraw。COM 跨版本适配通过 QueryInterface 获取独立引用，不转移 Canvas 的所有权。Canvas 的 Present 返回 `Ok(false)` 时转换为设备丢失错误，继续走应用现有错误路径；本阶段未新增设备自动恢复。

验证结果：

- `cargo test -p lucidpane preview:: --offline -- --test-threads=1`：36 passed、1 个手动性能基准 ignored。包括实际隐藏 HWND、GPU 像素读回、窗口缩放、设置打开关闭、材质初始化；新增 CPU 菜单上传与 GPU 绘制交替、150% DPI 及错误缓冲区长度检查。
- `cargo test -p lucidpane --test canvas_compat --offline`：4 passed。
- `cargo build -p lucidpane --offline`：通过。
- Clippy 执行通过，但项目原有代码存在警告，不能视为零警告检查。
- 并行运行预览测试时，原有输入框位置断言曾出现 99/100 的差异；串行运行通过，尚未定位该测试的并行干扰原因。
- 混合模式启动的 PID 40512 已核验为本项目调试程序、参数 `--hybrid-desktop`，但桌面工具无法枚举该实例的窗口。自动审批拒绝了强制终止并重启的操作；未继续终止或启动第二个实例。交互视觉验证尚未完成。

下面保留第一轮独立兼容性探针的覆盖范围和迁移注意事项。

## 已验证

基于本机缓存的已发布 `windows-canvas = 0.100.0`，使用 WARP 实际绘图并读回像素：

| 项目 | 结果 |
| --- | --- |
| 预乘 BGRA 图标 | 透明背景为全零；半透明颜色读回一致；缓存位图跨三次绘制可复用 |
| DPI | 96、120、144、192 DPI 下，16 DIP 方块分别覆盖 16、20、24、32 像素 |
| 中文文字 | 中文布局可换行；白色文字有中间 Alpha 值，RGB 灰度一致且不超过 Alpha |
| 合成交换链 | 三次尺寸变更后仍保持预乘 Alpha；绘图上下文保持 144 DPI |
| 原生接口互操作 | Canvas 的 COM 接口可通过受控借用/AddRef 与项目 windows 0.62.2 类型衔接 |
| 依赖 | 未启用 Reactor/Composition 集成，未引入 Windows App SDK |

运行：`cargo test -p lucidpane --test canvas_compat --offline`。结果：4 passed。测试使用合成图标像素，不代表已验证真实 Shell 图标提取和所有字体回退。

## 接入时必须处理

1. **发布版本与 master 不同。** 本机发布包没有 `system` feature；默认已依赖 `windows-window`。使用 `default-features = false` 即可运行此次验证，不能照搬最新 master 的 feature 配置。
2. **透明窗口使用合成交换链。** 发布包 `create_swap_chain` 配置预乘 Alpha；`create_swap_chain_for_window` 走普通 HWND 交换链，不能直接替换现有透明 Surface。初期保留 DirectComposition 挂载及原生背景材质。
3. **坐标语义不同。** `Rect::new` 接收四条边；项目现有 rect 辅助函数接收位置和尺寸，迁移时使用 `Rect::from_xywh`。
4. **保留明确的绘制错误处理。** 发布包 DrawingSession 析构中的 EndDraw 只记录设备丢失，未传播其他绘制错误。可用借用 session 保留调用方的 BeginDraw/EndDraw 检查，不应仅靠 session 析构判断绘制成功。
5. **类型版本隔离。** Canvas 使用 windows-core/numerics 0.100，项目仍有 core 0.62.2 和 numerics 0.3.1。接口转换集中在适配层，禁止转移借用 COM 指针的所有权；缓存必须随设备重建失效。
6. **灰度文字和省略号。** 透明表面明确设置灰度抗锯齿；现有 DirectWrite 的省略号裁剪保留原生调用，直到替代实现验证通过。

## 尚未验证

- 可见桌面的 Present 显示、与亚克力/壁纸材质的最终合成外观（隐藏 HWND 挂载及 Present 已由接入测试覆盖）。
- 实际跨显示器 WM_DPICHANGED、缩放期间命中区域与输入框位置。
- 硬件 GPU、设备丢失后的重建、长时间运行性能。
- 真实 Shell 图标、拖动图像以及现有设置页的视觉一致性。

`Surface` 的设备/交换链管理，以及第二阶段列出的具体绘制均已迁移；并非所有底层 D2D 调用都应移除，兼容层仍负责互操作、裁剪及明确的绘制错误检查。

参考：[Canvas 源码](https://github.com/microsoft/windows-rs/tree/master/crates/libs/canvas)。上述 API 和行为结论以实际测试的发布包为准。
