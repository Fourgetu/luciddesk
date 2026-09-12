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
- `try_begin_frame` 在呈现队列繁忙时跳过光栅化，保留一次定时刷新，以最新状态重绘。提前到达的定时器会重新安排剩余等待，避免最后一次更新丢失；没有额外固定 60 FPS 上限。无变化时不主动连续绘制，系统材质合成的开销另计。
- 硬件 D3D 设备创建失败时自动尝试 WARP。启动前设置 `LUCIDPANE_RENDERER=warp` 可强制验证软件路径；移除此环境变量并重启恢复默认模式。该入口不写入用户配置，不提供运行中无缝热切换保证。
- WARP 是 CPU 软件光栅化；DWM 和亚克力仍由 Windows 合成，不等于整个窗口系统无 GPU。普通帧不做应用级整帧 CPU 往返拷贝；图标缩放、选中背景生成后需要 CPU→GPU 上传，Shell 拖拽文字需要 GPU→CPU 读回。

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

背景使用 Windows.UI.Composition 与 HWND 桌面互操作。云母和云母 Alt 采用系统壁纸画刷叠加不同色调，不是完整原生 Mica 控制器；亚克力使用 HostBackdrop 画刷。材质失败时保留普通背景。

菜单淡入由 `windows-animation` 提供统一透明度，同时作用于内容与材质。首帧准备完成后开始计时，延迟帧仍提交终值，失败或系统禁用动画时直接显示。折叠保留现有曲线与真实 HWND 高度更新。

这些操作仍受 UI 消息循环调度，不代表已经实现独立合成线程动画。

## 验证

覆盖多 DPI、透明文字、裁剪失败清理、缓存位图替换、合成读回及淡入首尾状态。构建与测试通过不代替可见桌面的逐帧观察，详细结果见[验证记录](validation.md)。
