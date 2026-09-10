# 运行时托盘

`--hybrid-desktop` 模式启动后在 Windows 通知区域注册 LucidPane 图标。左键或键盘激活显示现有分组；右键菜单提供显示分组、新建分组和退出。显示操作沿用 pane 的现有层级设置，不修改置顶偏好。

托盘使用 Win32 `Shell_NotifyIconW` 和版本 4 回调，不增加依赖。图标、隐藏消息窗口及菜单资源随运行会话释放；退出发送现有 UI 消息循环的退出请求，然后执行 Hook 分离与桌面恢复。`TaskbarCreated` 消息重新注册托盘图标，但不更改已有的 Explorer/Hook 恢复策略。

系统可以将图标放进通知区域的折叠菜单；程序不修改用户的任务栏设置。

交互式 Windows 会话中执行托盘测试：

```powershell
cargo test -p lucidpane tray::tests --bin lucidpane -- --ignored
```

测试使用独立托盘图标，验证注册、版本 4 键盘回调、错误图标 ID 过滤、模拟任务栏重建后的重新注册，以及释放时删除图标。它不重启 Explorer。
