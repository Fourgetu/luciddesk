# 绘图与绑定

本文说明窗口内容如何绘制、提交和释放，以及两套 Windows Rust 绑定之间的接口边界。背景效果的参数与回退见[背景材质与主题](mica-materials.md)，缓存预算见[内存与资源优化](memory-optimization.md)。

## 实现入口

| 入口 | 职责 |
| --- | --- |
| `app/src/pane/render/mod.rs` | 面板内容、图标、文字与局部绘制资源 |
| `app/src/pane/canvas.rs` | 绘制作用域、裁剪、文字回退与离屏读回 |
| `app/src/pane/composition.rs` | 窗口 Surface、交换链、尺寸调整与呈现重试 |
| `app/src/pane/native_graphics.rs` | 设备缓存、绑定转换与图形资源退出顺序 |
| `crates/luciddesk-graphics/src/layer.rs` | DirectComposition 设备缓存与内容层 |
| `app/src/pane/acrylic/mod.rs` | WinRT 材质视觉树及合成提交 |
| `app/src/pane/settings/mod.rs` | 设置窗口首帧显示与材质编辑 |
| `app/src/pane/scaled_icons.rs` | 线程内 CPU 图标缩放缓存 |

## 绘制边界

应用直接使用 `windows_canvas` 的几何、颜色、画刷、文本格式、布局、位图和绘制上下文。绘制代码直接使用这些类型，绑定转换集中在互操作模块。

`canvas::DrawPass` 管理绘制生命周期与错误，并解引用到 Canvas 的 `DrawingSession`。它保留以下当前实现所需的原生操作：

- 轴对齐裁剪及失败退出时的裁剪栈清理。
- 透明合成目标上的灰度文字抗锯齿。
- 显式 BeginDraw/EndDraw 与错误传播。
- 文本省略号裁剪。
- 离屏目标绑定及像素读回。

`composition::Surface` 管理交换链与内容呈现，按窗口类型选择独立内容层或共享 WinRT 视觉树。面板、搜索、菜单与设置共用 Surface 接口；交换链按客户区尺寸创建，避免首帧先创建 1×1 缓冲再调整大小。

## 一帧的绘制与提交

常规窗口绘制依次经过以下边界：

1. 状态变化使窗口失效，窗口消息处理读取当前模型并请求绘制。
2. `Surface::try_begin_frame` 检查呈现重试期限；允许绘制时调整交换链尺寸并返回绘制上下文，等待期间返回 `None`。
3. `canvas::draw` 按 DPI 设置上下文，调用 `BeginDraw`，绘制内容并通过 `DrawPass::finish` 结束绘制。
4. `Surface::end_frame` 调用 `Present` 提交交换链，由 Windows 合成显示。

`finish` 会清空裁剪栈并传播 `EndDraw` 错误。绘制中途返回错误时，`Drop` 同样清理裁剪和结束绘制，但析构中的 `EndDraw` 错误不会再次返回。因此，正常路径应显式调用 `finish`；结束绘制与成功呈现是两个不同步骤。

呈现使用 `Present(0, DXGI_PRESENT_DO_NOT_WAIT)`。仅当返回 `DXGI_ERROR_WAS_STILL_DRAWING` 时安排单个 16 ms HWND 重试定时器，当前调用返回成功以保留后续刷新机会；这不表示该帧已经显示。重试读取最新状态，不保留过时帧队列。其他呈现结果会清除重试状态，错误正常传播；Surface 销毁也会取消定时器。

等待重试期间，`try_begin_frame` 跳过光栅化。定时器提前到达时会按剩余时间重新安排，避免丢失最后一次刷新。16 ms 是队列繁忙后的等待间隔，不是固定 60 FPS 限制，也不是实际显示延迟保证。

菜单首帧准备后仅在状态变化时失效；透明度不变时不提交合成更新。设置窗口的运行状态轮询仅在状态改变时请求重绘。无变化时不主动连续绘制，系统材质合成的开销另计。

## 设备与资源生命周期

UI 线程复用 D3D/D2D 设备，同一 DXGI 设备还复用 DirectComposition 设备，各窗口保留独立交换链与视觉树。合成设备缓存保留 COM 身份引用；图形设备变化时替换缓存，已有窗口仍持有自己的引用。

`gpu_device()` 复用缓存前检查设备移除状态，失效时重新创建设备。这个检查不等于所有现存 Surface 都会自动恢复；修改设备恢复逻辑时，需要同时验证窗口资源的重建和错误路径。

图形资源按“窗口与渲染器 → 图形线程缓存 → OLE/COM apartment”的顺序释放。`GraphicsLifetime` 显式清理缓存，不依赖进程退出时的 TLS 析构；见[启动与退出](architecture.md#退出与异常终止)。

图标文字复用字体格式、行高和有界布局缓存，字体大小属于布局缓存键。普通窗口不经过 CPU 像素读回；读回只用于 Shell 拖拽图像与测试。

## 资源、刷新与软件回退

- 图标 GPU 纹理仅保留视口及相邻一行，滚动过的大目录不会一直累积已上传图标；原始 Shell 图标像素仍由数据层管理。选中状态纹理缓存仍限制为 64 项。
- `scaled_icons.rs` 在每个绘图线程内共享纯 CPU 缩放缓存，最多 128 项、2 MiB 像素缓冲容量，按最近使用淘汰。缓存键包含源图像对象和目标物理尺寸，只弱引用源图像；源被替换后不会命中旧结果。清理失效源发生在后续插入时，缓存总量始终受限。同尺寸直接共享原 Arc，其他尺寸保持 WIC 原有插值和预乘透明度处理；不跨绘图上下文共享 COM 位图，也不扩大 GPU 纹理驻留范围。
- 硬件 D3D 设备创建失败时自动尝试 WARP。启动前设置 `LUCIDDESK_RENDERER=warp` 可强制验证软件路径；移除此环境变量并重启恢复默认模式。该入口不写入用户配置，不提供运行中无缝热切换保证。
- WARP 是 CPU 软件光栅化；DWM 和亚克力仍由 Windows 合成，不等于整个窗口系统无 GPU。普通帧不做应用级整帧 CPU 往返拷贝；图标缩放、选中背景生成后需要 CPU→GPU 上传，Shell 拖拽文字需要 GPU→CPU 读回。

桌面初次图标加载每批最多 32 项，同一会话仅允许一个在途批次，每批最多四个 Shell STA；刷新线程与初次加载批次不同时启动。批次完成后接续下一批，不等待下一次一秒维护。文件夹加载保留自身的三工作线程与有界结果通道。窗口刷新在复制文件夹项目、名称、元数据和图像 Arc 前先比较借用的快照，无变化时跳过复制。

## 绑定互操作

应用的 `windows`/`windows-core` 使用 0.62.2；Canvas、动画与私有生成绑定使用 0.100.0。相同 COM ABI 不代表相同 Rust 类型。

转换集中在 `native_graphics.rs`：

| 入口 | 用途 |
| --- | --- |
| `native_interface()` | 通过 QueryInterface 获取带独立引用计数的应用侧接口 |
| `canvas_result()` | 将新绑定错误转换为应用侧 HRESULT 错误 |
| `create_layer()` | 借用已知 DXGI 接口构造内容层 |
| `set_attribute()`、`extend_frame()` | 设置 DWM 属性与客户区扩展 |

不通过裸指针转移源接口所有权，不在普通绘图调用中分散类型转换。

## 生成绑定

`tools/windows-bindings` 是独立工具。`dwm.txt` 筛选 C API 和常量，`dcomp.txt` 筛选内容层需要的 COM 方法，结果保存到 `crates/luciddesk-graphics/src/bindings`。

不要手工修改生成结果。变更 API 清单后重新生成并运行 `--check`；命令见[构建与验证](build.md)。生成器不参与每次应用构建，API 清单、工具锁文件和生成源码应保持一致。

## 材质与动画

背景使用 Windows.UI.Composition 与 HWND 桌面互操作，壁纸或 HostBackdrop 经过亮度和染色混合；实现与回退规则见 [背景材质与主题](mica-materials.md)。这不是完整原生 Mica 控制器。系统高级效果关闭或进入节电模式时，亚克力、Mica 和 Mica Alt 回退为随主题变化的不透明底色，保留材质设置。策略恢复会使材质缓存失效并重新应用效果。壁纸画刷单独不可用时可先回退亚克力，其他失败再回退纯色。

菜单淡入由 `windows-animation` 提供统一透明度，同时作用于内容与材质。首帧准备完成后开始计时，延迟帧仍提交终值，失败或系统禁用动画时直接显示。折叠保留现有曲线与真实 HWND 高度更新。

这些操作仍受 UI 消息循环调度，不代表已经实现独立合成线程动画。

## 设置窗口首帧

设置窗口在同一 WinRT 视觉树中组合材质与交换链。显示前使用 `DWMWA_CLOAK` 隐藏尚未完成的画面，再以不激活方式显示窗口并等待 `RequestCommitAsync`。提交完成后按系统动画设置准备淡入，通过 `DwmFlush` 同步后解除隐藏并激活。

短期定时器只服务首帧准备，显示完成后移除。等待达到一秒后跳过继续等待；创建定时器失败时直接解除隐藏并尝试激活，避免窗口永久不可见。该顺序保证应用提交完成，不代表系统背景采样在所有驱动上都具有相同完成时刻。修改此流程应检查实际首帧显示。

## 纯色背景

`Backdrop::Solid` 保存 RGB 和背景不透明度，通过 `CompositionColorBrush` 绘制并复用内容与圆角。拖动滑块时更新视觉，松开后保存配置；全局默认值写入 `config.toml`，面板独立覆盖写入工作区数据库。

## 验证

在已准备好构建环境的 Windows 终端中运行以下针对性测试，环境配置见[构建与验证](build.md)。涉及 HWND、COM、D3D 或 WIC 的测试应在可用的 Windows 桌面环境中执行。

```powershell
# 绘制作用域、文字边界和裁剪失败恢复
cargo test -p luciddesk --bin luciddesk pane::canvas::tests --locked --offline -- --test-threads=1

# 材质、尺寸变化、呈现重试和淡入
cargo test -p luciddesk --bin luciddesk pane::composition::tests --locked --offline -- --test-threads=1

# 缩放缓存复用、淘汰和像素一致性
cargo test -p luciddesk --bin luciddesk pane::scaled_icons::tests --locked --offline

# 图形缓存先于 COM apartment 和进程退出释放
cargo test -p luciddesk --bin luciddesk graphics_caches_release_before_apartment_and_process_exit --locked --offline -- --test-threads=1

# 单独执行：会显示四个 GPU 窗口，默认测试跳过此项
cargo test -p luciddesk --bin luciddesk multi_window_render_latency --locked --offline -- --ignored --test-threads=1 --nocapture
```

手动检查以下场景：

- 在不同 DPI 的显示器之间移动窗口，检查文字、图标、裁剪和圆角是否正确。
- 连续滚动大目录、调整窗口尺寸并停止操作，确认最终状态完整显示，没有持续空转刷新。
- 打开设置及菜单，观察首帧、淡入末帧和材质切换；关闭系统动画后再次检查。
- 启动前设置 `LUCIDDESK_RENDERER=warp`，验证绘制、缩放与关闭，再清除变量并重启验证默认路径。
- 关闭全部窗口并退出，检查资源释放错误；性能测试比较同一机器、DPI、窗口数量和渲染路径下的结果。

测试通过不能代替真实桌面的首帧和动画观察，也不能证明所有驱动上的设备恢复行为。完整检查边界见[验证与兼容边界](validation.md)。
