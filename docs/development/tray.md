# 运行时托盘

托盘为 LucidDesk 提供显示面板、搜索、刷新和设置等常用入口。它与面板共用 UI 线程，通过 Win32 `Shell_NotifyIconW` 注册通知区域图标，使用版本 4 回调。系统可以把图标放入通知区域的折叠菜单，程序不修改用户的任务栏设置。

## 实现入口

| 文件 | 职责 |
| --- | --- |
| `app/src/tray.rs` | 图标注册、隐藏窗口、回调分派、菜单条目与释放 |
| `app/src/pane/hybrid.rs` | 创建 Tray，读取当前外观与搜索设置，执行 Action |
| `app/src/pane/menu.rs` | 共用 Composition 菜单、键盘交互与工作区定位 |
| `app/src/app_icon.rs` | 从程序资源加载图标并管理 HICON 所有权 |

托盘自身不保存工作区或实现刷新逻辑。新增菜单项时，需要同时维护条目 ID、ID 到 Action 的映射、应用侧处理和菜单测试。

## 操作与可见条件

左键或键盘激活执行 `Action::Show`，调用现有的显示面板入口，沿用面板层级设置，不修改置顶偏好。右键菜单按以下顺序组织：

| 操作 | 行为 |
| --- | --- |
| 显示面板 | 调用 `quick_reveal::show_all` |
| 搜索 | 仅 Everything 搜索已启用时显示，复用搜索激活入口 |
| 刷新全部 | 请求桌面图标、各文件夹来源及现有搜索窗口刷新，并唤醒协调器 |
| 新建分组 | 交给面板事件处理创建桌面分组 |
| 新建文件夹面板 | 交给现有文件夹面板流程 |
| 设置 | 打开设置窗口 |
| 打开配置目录 | 打开当前数据目录，不等同于重新加载配置 |
| 退出 | 请求 UI 消息循环退出，随后执行正常清理 |

显示与刷新、新建、设置与配置目录、退出分别用分隔线分区。刷新操作发起已有刷新流程，不承诺菜单关闭前所有后台读取都已完成。搜索项的显示条件取自当前启用设置，不表示 Everything 服务一定可用。

## 回调与延迟执行

Shell 可在同步 Hook IPC 内重入托盘窗口，此时应用状态可能仍被借用。`CALLBACK` 分支只校验并记录输入，不访问应用状态，也不直接打开菜单。

1. 从版本 4 回调的 `lParam` 高位检查图标 ID，忽略其他图标的消息。
2. 接收 `NIN_SELECT`、`NIN_KEYSELECT` 或 `WM_CONTEXTMENU`，投递内部 `DISPATCH` 消息。
3. 只有投递成功且没有待处理事件时才记录事件与锚点；已有事件等待执行期间，后续受支持的回调被合并。
4. `DISPATCH` 取出待处理事件，执行显示操作，或读取当前设置并打开菜单。

这是一个待处理事件槽，不是完整输入队列，也不是只合并同类点击：待处理事件存在时，后来的另一类托盘输入同样不会追加。修改该逻辑时应保持“同步 Shell 回调不进入应用操作或嵌套菜单循环”的边界。

`WM_CLOSE` 不走上述输入合并路径，而是直接请求 `Action::Exit`，保留 Tray 至正常退出流程释放。线程、重入与资源约束见 [Rust API 与资源生命周期约定](rust-api-review.md)。

## 菜单外观、定位与焦点

菜单复用面板的 Composition 浮层、圆角、Fluent 图标及淡入和悬停效果。每次打开时读取当前全局外观；未设置时回退到第一个面板，再回退到系统主题和 Mica。菜单使用材质的默认属性，不照搬面板的自定义强度；纯色与半透明背景也由菜单的默认材质规则处理。

版本 4 回调在 `wParam` 中携带有符号屏幕坐标，负坐标不能按无符号位置使用。键盘菜单锚点为 `(-1, -1)` 时，先查询通知图标矩形，失败后回退到鼠标位置。

打开前将隐藏所属窗口移动到锚点所在屏幕，使菜单按该屏幕 DPI 布局；共用菜单再把位置限制在最近显示器的工作区。方向键导航、Enter/Space 选择、Escape 取消和失焦关闭沿用共用菜单实现。

菜单关闭后投递 `WM_NULL`，并通过 `NIM_SETFOCUS` 请求将焦点交还通知区域。取消或未知命令不产生 Action。首帧与动画边界见[绘图与绑定](rendering.md)。

## 注册、重建与释放

注册依次执行 `NIM_ADD` 和 `NIM_SETVERSION`。添加失败返回错误；版本设置失败时先删除刚添加的图标再返回错误。隐藏窗口使用工具窗口和不激活样式，不作为普通应用窗口显示。

`Tray` 持有隐藏窗口和共享拥有的图标，窗口回调也持有图标引用。正常释放时先发送 `NIM_DELETE`，再随对象释放窗口与图标资源。退出操作本身不立即销毁托盘，应用退出顺序由主运行作用域负责。

收到 `TaskbarCreated` 时尝试重新添加图标；这只恢复通知区域入口，不替代 Explorer/Hook 会话恢复。当前重建分支忽略重新注册错误，没有专门的托盘重试定时器，因此不能承诺所有任务栏重建故障都自动恢复。

界面语言变化时通过 `NIM_MODIFY` 更新提示文本，菜单标签在打开时重新读取。退出和 Explorer 恢复的整体边界见[架构说明](architecture.md#退出与异常终止)。

## 验证

菜单条目测试不需要操作真实通知区域：

```powershell
cargo test -p luciddesk --bin luciddesk common_actions_follow_search_setting_and_exit_is_separate --locked --offline
```

交互式 Windows 会话中的托盘测试默认忽略，应单独串行执行：

```powershell
cargo test -p luciddesk --bin luciddesk tray::tests --locked --offline -- --ignored --test-threads=1
```

| 测试 | 覆盖内容 |
| --- | --- |
| `common_actions_follow_search_setting_and_exit_is_separate` | 搜索条件、各操作唯一性及退出分区 |
| `tray_registers_handles_keyboard_selection_readds_and_removes` | 注册、键盘回调、错误图标 ID、模拟任务栏重建及释放 |
| `sent_clicks_defer_until_state_is_released_and_coalesce` | 状态被借用期间同步连续点击，延迟执行与合并 |
| `native_close_requests_normal_exit_without_destroying_tray_early` | 原生关闭请求触发正常退出，托盘不提前销毁 |

这些测试使用独立托盘图标，以消息模拟任务栏重建，不会重启 Explorer，也不覆盖真实菜单所有操作的最终效果。

实机检查左键和键盘激活、右键菜单取消、搜索开关变化、刷新、打开配置目录、语言切换及退出。多显示器场景检查不同 DPI、负坐标与任务栏折叠菜单中的定位和焦点；验证重建行为时区分模拟消息与真正的 Explorer 重启。构建环境与完整验收要求见[构建与验证](build.md)和[验证与兼容边界](validation.md)。
