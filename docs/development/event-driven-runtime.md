# 事件驱动的后台调度

普通分组窗口不再注册全局轮询计时器。Runtime 统一接收 Shell 变化、桌面输入、目录扫描、图标加载和审计结果的通知。`Wake` 合并尚未消费的通知，并在窗口销毁时解除绑定，避免后台线程向已释放的窗口投递新消息。密集桌面事件保留 20 ms 合并窗口，延后工作由单次计时器接续，避免通知洪峰变成重复扫描。

## 调度规则

| 工作 | 触发与兜底 |
| --- | --- |
| 文件夹刷新 | 等待目录变化或手动刷新事件；变化突发合并 100 ms；通知不可用时每 2 秒重试 |
| 文件夹线程退出 | 独立退出标记唤醒等待；STA 在空闲期间继续处理 Windows 消息 |
| 搜索 | 输入后单次 250 ms 防抖，后台结果主动通知；空闲时没有 50 ms 轮询 |
| 桌面输入 | 输入通知主动唤醒；鼠标按下期间按需检查释放状态，间隔 25 ms |
| 图标更新 | 结果主动通知；变化防抖和失败重试按截止时间安排计时器 |
| 桌面审计 | 连续稳定后间隔从 1 秒退避为 2、4、8、15 秒；读取新快照时重置 |
| 布局发布 | 变化与交互触发；稳定审计不再强制重发；距离上次同步 30 秒时保留一次修复 |
| Runtime 维护 | 全局每秒检查连接、显示器布局、备份与窗口恢复；没有搜索面板时跳过不必要的 Everything 路径检查 |
| 自动隐藏 | 仅启用该功能的面板保留 60 ms 悬停检查 |

计时器仍用于有明确截止时间的任务和恢复兜底。Explorer 内部重置不一定产生可观察通知，因此此轮没有完全取消周期布局修复。通知缺失时，桌面清单审计的恢复延迟可能增加至约 15 秒，纯显示状态修复可能等待约 30 秒。

## 批量坐标核对

`BaselineCheck` 使用独立的有界 `WM_COPYDATA` 消息，将期望的 Shell 版本、项目索引和原始坐标一起发送给 Hook。Hook 验证版本与数量，在目标 UI 线程内逐项只读比较，再返回一致或不一致。

原来一次完整坐标核对需要约 `2 × N` 次同步跨进程请求，现在只需一次；目标进程内部仍需最多 N 次坐标读取。负坐标不再与协议错误返回值混淆。新消息需要同次构建的主程序与 Hook DLL 配套使用。

桌面项目标识审计仍通过 Shell COM 读取，完整快照与初次基线采集也仍有开销。本次没有修改数据库格式，也没有关闭主程序退出时恢复桌面的 Hook 看门狗。

## 验证

使用独立构建目录，避免与其他构建共享中间产物：

```powershell
cargo check --workspace --all-targets --offline --target-dir target/event-driven-check
cargo test -p lucidpane --bin lucidpane --offline --target-dir target/event-driven-check -- --test-threads=1
cargo test -p desktop-core -p desktop-storage -p desktop-hook -p desktop-shell -p desktop-window -p desktop-graphics --lib --offline --target-dir target/event-driven-check -- --test-threads=1
cargo test -p lucidpane --test canvas_compat --offline --target-dir target/event-driven-check
cargo run --locked --offline --manifest-path tools/windows-bindings/Cargo.toml -- --check
```

新增回归覆盖通知突发合并、禁止计时器轮询时的目录结果投递、线程退出中断等待、批量核对边界/过期版本/负坐标，以及空闲同步截止时间。

2026-09-13 全量自动验证：主程序 126 项通过、9 项手动测试跳过；工作区库测试 45 项通过、2 项手动测试跳过；Canvas 集成测试 4 项通过。合计 175 项通过、0 项失败、11 项跳过。全目标编译和 Windows 绑定生成一致性检查通过，示例仍有未使用代码警告。

验证期间修复了 `native_backdrop_probe` 的 `effects` 和 `animation` 模块引用，以及设置窗口测试写死旧高度的断言。首次完整 UI 测试的 `STATUS_ACCESS_VIOLATION` 可由材质测试接续淡入测试复现；测试夹具现保留进程生命周期的 COM 使用引用，避免短生命周期 STA 退出后继续使用缓存的 WinRT 工厂。修复后两项复现测试和完整主程序测试均通过。

测试清理保留事件唤醒、监听退出、存储写入、协议边界和原生窗口生命周期的回归覆盖；删除旧启动模式的重复输入枚举和重复 OLE 初始化。设置页渲染测试默认只做像素断言；需要导出视觉检查用 BMP 时，设置 `LUCIDPANE_TEST_EXPORT_SNAPSHOTS=1`，再运行 `settings_layout_and_rendering_at_multiple_scales`，图片仍输出至 `target/settings-*.bmp`。常规测试不再自动写出 14 张图片。

跳过项依赖实际 Shell、Everything、QuickLook、通知区域或交互菜单，也包含手动 GPU 性能基准；自动测试通过不等于这些手动场景已经验收。

上述请求数变化来自实现与回归测试，不代表实测 CPU 或磁盘 I/O 降幅。运行验证应在相同图标数量和面板配置下，同时比较 LucidPane 与 Explorer 的 CPU、I/O、空闲唤醒和交互延迟，并检查拖入/拖出、桌面重排、Explorer 重启、多显示器切换及菜单期间的恢复。
