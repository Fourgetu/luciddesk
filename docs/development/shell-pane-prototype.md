# 可见 Shell Pane 原型测试（2026-09-15）

结论：独立 Shell 原型可承载系统视图。在 Explorer 内启用精简菜单并补齐注册命令消息转发后，重命名进入编辑框、属性和 Esc 关闭均已验证。根因与 Pane 接入见 [命令路由技术文档](../win11-compact-menu-command-routing.md)。`full_shell_pane_probe` 只保留作历史对照，产品不需要完整 Explorer 文件窗口。下表保留初次独立进程测试结果。

## 运行

```powershell
# 混合目录集合：两个不同目录的临时文件
cargo run -p desktop-shell --example shell_pane_probe

# 真实文件夹：Shell 自行维护内容变更
cargo run -p desktop-shell --example shell_pane_probe -- --folder

# 在可见窗口内尝试接入 Win11 私有 presenter
cargo run -p desktop-shell --example shell_pane_probe -- --folder --compact

# 对照已有研究中的另一宿主参数（未公开的枚举，不能当作兼容性契约）
cargo run -p desktop-shell --example shell_pane_probe -- --compact --desktop-presenter
```

文件只在 `target/shell-pane-probe/<进程号>/` 下创建。原型不注入 Explorer、不注册桌面过滤、不改 Pane 配置、不移动真实桌面文件。窗口关闭时释放视图与 presenter；临时文件保留以供核对。

普通右键完全由 Shell 处理。选中项目后按 F6，会向**同一个可见视图**显式调用 `IContextMenuSite::DoContextMenuPopup`，用于与鼠标路径对照。该接口及私有 presenter 仅用于诊断，不构成精简菜单的公开兼容性保证。退出使用窗口关闭按钮或 Alt+F4。

## 架构

- 常驻 `IExplorerBrowser` 和 `IShellView`，每个进程只创建一个内容宿主，不在右键时重建。
- `--folder` 使用 `BrowseToObject` 承载真实目录。
- 默认模式通过 `IResultsFolder::AddItem` 聚合不同父目录的 `IShellItem`。
- 图标绘制、选择、右键和原生重命名编辑框均由 Shell 处理。
- OLE STA 消息循环支持 Shell 的 `TranslateAccelerator`；只把 S_OK 视为已处理，S_FALSE 仍须派发。另接入已加载 WinUI 的 `ContentPreTranslateMessage`。
- `--compact` 单独测试私有 COM presenter 的初始化与服务查询，不修改系统注册表、不使用模块地址偏移。标题中的 `compact experiment` 表示实验开关，**并不表示实际出现了精简菜单**。

## 本机实测

通过 Computer Use 在实际可见窗口中点击菜单；重命名结果另由文件系统读取核对。

| 测试 | 结果 |
| --- | --- |
| 不同父目录的两个文件显示在一个内容区 | 通过 |
| 普通鼠标右键、菜单点击、Esc 取消 | 通过，显示经典菜单 |
| 从混合目录集合菜单进入原生重命名并提交 | 磁盘重命名成功 |
| 混合目录集合重命名后的名称同步 | 未通过，视图仍显示旧名称，需要集合维护逻辑 |
| 真实文件夹菜单重命名并提交 | 通过，磁盘和视图同时更新 |
| 私有 presenter 初始化及服务 QueryInterface | 本机返回成功，但不足以让菜单变成精简样式 |
| 两种宿主参数、普通右键与 F6 显式请求 | 观察到的菜单均为经典菜单 |
| 生命周期 | 同一视图内完成菜单与重命名；已结束的测试实例正常释放宿主 |

关键日志在 `target/shell-pane-compact.log`、`target/shell-pane-compact-comparison.log` 和 `target/shell-pane-folder.log`。目录 PID 69544 的样例成功变为 `Renamed by Shell.txt`；PID 78580 的真实文件夹样例成功变为 `Renamed in native folder.txt`。

## 尚未验证

- 独立原型精简菜单已通过重命名、属性和取消；正式 Pane 的隐藏宿主与桌面过滤联测以最新技术文档为准。
- 混合目录分组需要增量维护成员身份和文件变更。真实文件夹视图的自动更新能力不能直接等同于结果集合能力。
- 未测删除、拖放、第三方扩展全部命令、多 Pane、主题适配及 Explorer 重启。
- 未进行鼠标忙碌指针或弹出延迟的量化测试，不能宣称性能问题已解决。
- 本次未验证 Win10 或其他 Win11 版本。正式 Pane 与 Hook 的既有实现未因本原型而替换。
