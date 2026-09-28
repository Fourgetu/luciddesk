# 绘图与绑定

## 绘制边界

应用直接使用 `windows_canvas` 的几何、颜色、画刷、文本格式、布局、位图和绘制上下文。旧 Format/Brush 持有器及逐图元转发方法已移除；普通绘图不转换两套重复类型。

`canvas::DrawPass` 管理绘制生命周期与错误，并解引用到 Canvas 的 `DrawingSession`。它保留以下当前实现所需的原生操作：

- 轴对齐裁剪及失败退出时的裁剪栈清理。
- 透明合成目标上的灰度文字抗锯齿。
- 显式 BeginDraw/EndDraw 与错误传播。
- 文本省略号裁剪。
- 离屏目标绑定及像素读回。

`composition::Surface` 管理交换链与内容呈现，直接持有 `desktop_graphics::Layer`。面板、搜索、菜单与设置共用这条路径；交换链按客户区尺寸创建，避免首帧先创建 1×1 缓冲再调整大小。

UI 线程复用 D3D/D2D 设备，同一 DXGI 设备还复用 DirectComposition 设备，各窗口保留独立交换链与视觉树。合成设备缓存保留 COM 身份引用；图形设备变化时替换缓存，已有窗口仍持有自己的引用。

呈现使用 `Present(0, DXGI_PRESENT_DO_NOT_WAIT)`。队列繁忙时设置单个 16 ms HWND 定时器，下一次绘制提交最新状态；成功提交或销毁 Surface 时清理定时器，其他 HRESULT 错误正常传播。该策略减少界面线程等待，允许合并过时帧，并非逐帧送达或帧率保证。参见 [DXGI Present 标志](https://learn.microsoft.com/en-us/windows/win32/direct3ddxgi/dxgi-present)。

菜单首帧准备后不再主动重复失效；透明度不变时不提交合成更新。设置窗口的运行状态轮询仅在状态改变时请求重绘。图标文字复用字体格式、行高和有界布局缓存，字体大小属于布局缓存键。普通窗口不经过 CPU 像素读回；读回只用于 Shell 拖拽图像与测试。

## 资源、刷新与软件回退

- 图标 GPU 纹理仅保留视口及相邻一行，滚动过的大目录不会一直累积已上传图标；原始 Shell 图标像素仍由数据层管理。选中状态纹理缓存仍限制为 64 项。
- `scaled_icons.rs` 在每个绘图线程内共享纯 CPU 缩放缓存，最多 128 项、2 MiB 像素缓冲容量，按最近使用淘汰。缓存键包含源图像对象和目标物理尺寸，只弱引用源图像；源被替换后不会命中旧结果。清理失效源发生在后续插入时，缓存总量始终受限。同尺寸直接共享原 Arc，其他尺寸保持 WIC 原有插值和预乘透明度处理；不跨绘图上下文共享 COM 位图，也不扩大 GPU 纹理驻留范围。
- `try_begin_frame` 在呈现队列繁忙时跳过光栅化，保留一次定时刷新，以最新状态重绘。提前到达的定时器会重新安排剩余等待，避免最后一次更新丢失；没有额外固定 60 FPS 上限。无变化时不主动连续绘制，系统材质合成的开销另计。
- 硬件 D3D 设备创建失败时自动尝试 WARP。启动前设置 `LUCIDPANE_RENDERER=warp` 可强制验证软件路径；移除此环境变量并重启恢复默认模式。该入口不写入用户配置，不提供运行中无缝热切换保证。
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
| `set_attribute()`、`extend_frame()` | 设置 DWM 属性与客户区扩展，不再保持旧裸指针调用签名 |

不通过裸指针转移源接口所有权，不在普通绘图调用中分散类型转换。

## 生成绑定

`tools/windows-bindings` 是独立工具。`dwm.txt` 筛选 C API 和常量，`dcomp.txt` 筛选内容层需要的 COM 方法，结果保存到 `crates/desktop-graphics/src/bindings`。

不要手工修改生成结果。变更 API 清单后重新生成并运行 `--check`；命令见[构建与验证](build.md)。生成器不参与每次应用构建，API 清单、工具锁文件和生成源码应保持一致。

## 材质与动画


背景使用 Windows.UI.Composition 与 HWND 桌面互操作。云母使用系统模糊壁纸画刷，经亮度、染色两级 GPU 混合，参考 [WinUI 2.8 的公开实现](https://github.com/microsoft/microsoft-ui-xaml/blob/v2.8.0/dev/Materials/Backdrop/SystemBackdropBrushFactory.cpp)。主题底色为深色 #202020、浅色 #F3F3F3，染色强度分别为 80% 和 50%；Alt 暂为增强壁纸色彩的预设，强度为 65% 和 35%，不声称与官方 BaseAlt 完全一致。效果工厂在同一合成运行时内复用，不读取壁纸像素到 CPU，也不额外计算模糊。壁纸材质不可用时优先使用亚克力配方，保留所选效果强度；HostBackdrop 或效果也不可用时回退到不透明主题底色。Win10 的 HostBackdrop 初始化使用独立封装的动态兼容入口，详见 [Mica 材质](mica-materials.md)。当前仍不是完整原生 Mica 控制器，未复刻全部激活和系统策略行为；亚克力使用 HostBackdrop 已模糊背景，复用亮度、染色两级混合，按 WinUI AcrylicBrush 的中性色公式修正染色与亮度不透明度，避免对背景重复模糊。颜色固定跟随主题，不开放自定义颜色。当前尚未叠加官方配方中的噪点纹理。

菜单淡入由 `windows-animation` 提供统一透明度，同时作用于内容与材质。首帧准备完成后开始计时，延迟帧仍提交终值，失败或系统禁用动画时直接显示。折叠保留现有曲线与真实 HWND 高度更新。

这些操作仍受 UI 消息循环调度，不代表已经实现独立合成线程动画。

## 验证

覆盖多 DPI、透明文字、裁剪失败清理、缓存位图替换、合成读回及淡入首尾状态。构建与测试通过不代替可见桌面的逐帧观察，详细结果见[验证记录](validation.md)。

### Settings first presentation

Settings attaches its swap chain above its material in one WinRT composition
visual tree. Before showing the HWND it sets `DWMWA_CLOAK`; unlike `SW_HIDE`,
cloaking allows DWM to compose the window without displaying partial content.
After showing without activation, a short-lived timer polls `RequestCommitAsync`.
Once the material commit completes, a 160 ms opacity animation is attached to
the shared root if Windows client-area animations are enabled. Its initial
frame is committed while still cloaked to avoid a full-opacity flash. Then
`DwmFlush` synchronizes presentation and the window is uncloaked and activated.
The compositor animates content and material together without app repainting.
The timer is removed immediately on reveal; a one-second
failure deadline prevents an indefinitely inaccessible settings window.
This is a composition fence, not an API guarantee that every host backdrop
implementation has finished sampling. Opening-frame captures are required when
changing this sequence. No opaque cover, separate material fade, CPU readback or persistent
render loop is used. Plain-translucent mode hides only the material visuals so
content sharing the tree stays visible.


### Solid material

`Backdrop::Solid` carries RGB and a background-only opacity. It uses a
`CompositionColorBrush` with a transparent tint visual, without creating a host
backdrop or wallpaper brush for this material. Existing content visuals and
corner clips are reused; content opacity stays independent. Slider gestures
update visuals in memory and commit the workspace once on release. The storage
The workspace database stores per-panel color overrides in `panels`, and global solid
style defaults in `config.toml`. Development builds do not migrate older databases.
