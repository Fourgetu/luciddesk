# 面板拖动被同类窗口遮挡修复

2026-09-28。

旧逻辑仅在 WM_MOUSEACTIVATE 抬升面板，已激活但被另一个面板盖住时可能没有该消息。
更关键的是，实际面板共享 GetShellWindow 返回的 owner。缺少 SWP_NOOWNERZORDER 时，
显式调整一个面板可能连带触发 owner/其他 owned 窗口重排，覆盖目标排序。
此前回归测试创建的是无 owner 的窗口，因此漏掉这一条件。

修复：
- 在低层 subclass 中处理点击、非客户区点击、进入原生移动循环，避免依赖模型借用或激活消息。
- 显式 SetWindowPos 和 set_layer 调用加入 SWP_NOOWNERZORDER；继续通过 desktop_insert_after
  限制在桌面面板层内，不将普通桌面面板置为全局 topmost。
- 回归窗口增加共享 owner，验证新建、被覆盖后点击/拖动、普通应用上层约束和置顶切换。

验证：增加 owner 后旧路径测试失败；修复后 5 项窗口测试和 4 项标签测试通过。
真实 Debug 中将分组 10 提到分组 8 前面，再直接发送 WM_ENTERSIZEMOVE，分组 8 回到前方；
随后发送 WM_EXITSIZEMOVE 清理移动状态，没有改变窗口位置和尺寸。该检查验证 native 消息及
Z-order 路径，不等同于人工完整鼠标拖动验收。

记录：target/pane-window-tests.log、target/pane-tabs-tests.log、target/pane-drag-live-check.json。
当前运行 Debug PID 58024。
