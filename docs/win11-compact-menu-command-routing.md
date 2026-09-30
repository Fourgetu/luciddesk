# Win11 独立 Shell 菜单命令路由

本文说明独立 Shell 菜单的命令路由与生命周期；该路径依赖系统内部接口，不能保证所有 Windows 构建兼容。

## 命令分发

独立 Shell 视图需将注册消息 `FILE_EXPLORER_CONTEXTMENU_INVOKEMENUITEM` 转发到菜单 Presenter 的 `Invoke`。消息编号及命令编号由当前运行环境确定，不能硬编码数值。窗口命中只说明输入到达，不代表 Shell 命令已执行；验证时同时检查命令路由与最终操作结果。

### 产品结构

Pane 保留现有图标绘制、布局、选择和编辑器。真实桌面持续过滤 Pane 中的项目。菜单使用另一份包含选中项目的原生 Shell 视图，两份视图不共享选择状态；既不临时恢复桌面成员，也不复制真实桌面 ListView。

```mermaid
sequenceDiagram
    participant P as Pane UI STA
    participant D as Explorer 桌面 STA
    participant M as Explorer 菜单 STA
    participant X as 原生 Win11 Presenter
    P->>D: MENU_PREPARE(身份集合, owner, 锚点)
    D->>M: 准备或复用独立 Shell 视图
    M-->>D: 经过身份校验的视图 HWND
    D-->>P: ACK + HWND
    P->>M: 安装观察器后请求打开菜单
    M->>X: 原生 Shell 菜单请求
    X->>M: 注册消息 + 动态命令 ID
    M->>X: Invoke(command)
    X->>M: 原生 Shell 命令回调
    P->>D: 菜单关闭后 MENU_FINISH
    D->>M: 标记可复用，保留宿主
```

### 接入点

| 位置 | 职责 |
| --- | --- |
| `app/src/pane/window.rs` | 捕获完整选中集合；释放模型借用后打开菜单；接收重命名结果；键盘菜单锚定选中图标 |
| `crates/desktop-shell/src/native_menu.rs` | 前台授权、菜单观察器、关闭后有条件返回 Pane 焦点 |
| `crates/desktop-hook/src/filter/engine.rs` | 验证 owner 所属进程，管理准备/结束协议，菜单错误不撤销桌面过滤 |
| `crates/desktop-hook/src/filter/menu/worker.rs` | 常驻独立 STA，序列化准备、结束与消息派发；超时取消过期准备请求 |
| `crates/desktop-hook/src/filter/menu.rs` | 原生结果视图、选中身份校验、视图命令消息转发及统一菜单请求 |
| `crates/desktop-hook/src/filter/menu/presenter.rs` | 封装私有 COM 接口，处理注册消息对应的 Invoke，保留回调生命周期 |
| `crates/desktop-hook/src/filter/menu/commands.rs` | 包装本次 IContextMenu，记录动态编号起点，查询标准动词并转发普通命令 |
| `crates/desktop-hook/src/filter/menu/callback.rs` | 保留原生 Shell 回调；将 rename 转回 Pane，其他命令继续交给原生回调 |

### 宿主复用与生命周期

- 每个过滤会话持有一个菜单 STA。`MENU_FINISH` 释放本次菜单使用权，不立即销毁 STA 和 Shell 视图。
- 同一 owner、同一有序目标集合可复用，但每次都重新解析目标并比较视图中完整的 Shell 身份集合，再恢复选中项和弹出锚点。删除、改名或集合不匹配不能复用旧目标。
- 目标改变时创建新的已验证视图。旧宿主若仍拥有可见命令窗口（例如属性），先保留，待窗口关闭后在菜单 STA 上释放。
- 属性窗口仍打开时，不复用它的宿主打开下一份菜单，避免禁用 owner 或模态状态影响输入。
- 控制进程退出、过滤会话断开时，worker 请求通道断开，宿主和 COM 对象在其创建线程清理；不停止或重启 Explorer。
- 生产宿主使用空窗口区域提供焦点与 Shell 上下文，本身不绘制文件内容。项目已有 `LUCIDPANE_INSPECT` 测试入口只改变窗口的可检查样式，菜单宿主仍保持空区域。

### 输入与错误边界

- 菜单 HWND 的正常命中只是输入到达窗口的证据，不能替代命令消息和最终功能结果。
- 菜单先关闭、再执行命令是异步链路。观察器等待关闭稳定后才结束请求，不把 XAML 的零尺寸初始化窗口当作已显示菜单。
- 属性或其他应用获得前台时不抢回 Pane 焦点。只有前台仍是菜单宿主时才返回 Pane。
- Pane 图标继续自绘，改名输入仍由既有 `RenameItem` 处理。菜单通过 `IContextMenuSite::DoContextMenuPopup` 打开；包装的 `IContextMenu` 记录本次 `QueryContextMenu` 的编号起点，回调以 `command - first` 查询 `GetCommandString(GCS_VERBW)`。仅标准 `rename` 动词转换为 Pane 重命名结果，其他命令继续交给原生 Shell 回调。
- 重命名主路径在开始隐藏编辑前拦截标准动词；`LVN_BEGINLABELEDIT` 只作为兼容补充，不作为编辑器启动的必要条件。
- 识别重命名后先结束原生视图的菜单会话，再通知 Pane。初始化失败或编号范围无效时不猜测命令编号；每次打开重新记录范围，准备失败清除旧范围。
- 不把菜单错误视为桌面过滤失败；准备失败和命令结束各自清理状态，保留真实桌面的过滤成员。
- 冷启动和第三方扩展加载仍有成本；复用减少重复初始化，不等于已经量化消除了所有忙碌指针或扩展加载时间。

- 生产 `menu_messages` 在独立视图上识别注册消息，转给该视图自己的 NativePresenter。
- 使用 `RegisterWindowMessageW` 获取运行时消息 ID；不写死 `0xc162`、重命名 ID 或其他命令 ID。
- 沿用原生 Presenter 的关闭和命令选择；普通命令交给 Shell，重命名通知 Pane，不使用固定命令编号或模拟键盘操作。
- 消息处理期间保留 COM/Rc 引用，避免命令重入关闭宿主时释放正在使用的对象；已关闭 Presenter 不再执行命令。

### 提交改名后的身份交接

`rename_shell_item` 使用公开的 `IFileOperationProgressSink::PostRenameItem` 获取 Shell 返回的新项目。托管桌面项目由 `hybrid/rename_transaction.rs` 提交：

1. `UPDATE_BEGIN` 暂停桌面绘制并保持成员过滤。
2. Shell 完成改名后，更新 Pane、图像键和持久化身份，保留分组位置。
3. `REPLACE_IDENTITY` 交接新 PIDL、过滤名字和恢复坐标，发布完整成员集合。
4. `UPDATE_END` 释放事务；失败时仍尝试补偿发布和释放，待保存的工作区由后续同步重试。

更新期间暂停后台清单接收。审计版本包含稳定 ID 和解析名，拒绝改名前发出的结果；文件 ID 不变也不能接收旧路径。晚到的插入通知在桌面下一次绘制前重新过滤。失败、取消及控制进程退出均需要解除绘制暂停。

### 输入来源与 COM 接口

Pane 将鼠标或键盘来源传给菜单。Presenter 适配层在 `DoContextMenu` 及可选 `IContextMenuPresenterTipTest` 显示入口设置鼠标来源位 `0x8`；键盘来源清除此位，其他标记与显式访问键命令透传。

适配层通过 `QueryInterface(IContextMenuPresenter)` 获取准确接口指针，不能把规范 IUnknown 指针按 Presenter 函数表调用。它只暴露自身实现的 IUnknown、Presenter 和可选 TipTest，并保持统一 COM 身份；宿主持有原生 IClosable 用于关闭。Shell 视图回调继续提供其原有服务查询。

`IContextMenu` 包装层同时转发 `IObjectWithSite::SetSite` 与 `GetSite`，保留 Shell 宿主上下文，包括空值清理和错误传播；打开方式等子命令依赖这一上下文。

### 光标与等待

激活独立视图、获取选择菜单、转发 `QueryContextMenu` 及弹出调用返回时检查光标。只有当前菜单 STA 拥有前台 `LucidPane.IsolatedShellHost.v1`，且光标为 `IDC_WAIT` 或 `IDC_APPSTARTING` 时，才恢复 `IDC_ARROW`。不修改其他前台窗口或全局系统光标。

菜单等待使用有超时的消息唤醒；持有桌面状态借用期间只处理同步发送消息，不派发已投递的过滤修改请求。宿主复用不能消除原生菜单和第三方扩展的加载成本。

### 异常路径与清理

- 客户端先发布带 BEGIN 序号的 `UpdateRelease` 标记，再请求 END；Explorer 定时器可以独立确认 `UpdateReleased`。未确认释放前不开始新事务，旧序号不能释放后续事务。
- `MENU_CANCEL` 设置共享取消标记，菜单线程在显示和命令调用前检查，并关闭 Presenter；取消过的宿主不能复用。扩展阻塞时不强杀 Explorer 线程，返回后继续收尾。
- 初始快照未修改视图时，读取失败允许三次退避重试；持续失败或修改后的失败进入恢复。重试期间阻止新插入项提前绘出。
- 整体过滤连接故障时可能短暂恢复桌面，不能承诺所有异常路径都无闪现。

## 普通菜单回退外观

私有 Presenter 创建、初始化或服务安装失败时，同一验证后的 Shell 选择集通过公开 `IContextMenu` 和 `TrackPopupMenuEx` 打开菜单，保留动态消息和 Pane 重命名转发。

原生 HMENU 复用文件夹面板的系统明暗主题与 DPI 留白。每次弹出重新读取主题，高对比度沿用系统绘制；关闭后释放窗口钩子并恢复 Explorer 的主题偏好。Shell 扩展负责自己的项目、分隔线和子菜单。

## 验证入口

原生 Presenter 接口检查在独立测试进程执行：

```powershell
cargo test -p desktop-hook --lib --offline native_presenter_uses_queried -- --ignored --nocapture
```

命令路由测试应覆盖鼠标与键盘来源、重命名提交与取消、打开方式子菜单、属性窗口、丢失事务释放请求和取消后的迟到命令。模拟 COM 对象不能替代 Explorer 内的原生接口验证；适用平台与未覆盖范围见[验证与兼容边界](development/validation.md)。
