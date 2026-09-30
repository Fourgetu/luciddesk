# 项目目录结构

路径均相对仓库根目录。

## 顶层分工

| 目录 | 放置内容 |
| --- | --- |
| `app/` | 可执行程序、窗口交互、应用资源和 UI 集成测试 |
| `crates/` | 领域模型、存储及 Windows 平台能力，按依赖边界划分 |
| `tools/` | 发布打包、资源和绑定生成、独立验证工具 |
| `docs/` | 使用说明、开发指南和当前图标设计稿 |
| `.github/` | CI 与发布工作流 |
| `target/` | Cargo 产物、测试输出和本地实验数据，不作为源码目录 |

## 应用功能目录

```text
app/src/
├── main.rs                 # 启动与参数
├── desktop_component.rs    # 包标记、桌面 DLL 部署与缓存
├── tray.rs                 # 托盘生命周期
├── diagnostics.rs
├── app_icon.rs
└── pane/
    ├── mod.rs              # 应用状态与模块入口
    ├── model.rs
    ├── events.rs
    ├── folder.rs           # 目录监听、清单更新与导航
    ├── folder/
    │   ├── entry_mode.rs   # 文件夹激活策略与全局模式
    │   ├── preferences.rs  # 视图默认值、列配置与持久化
    │   └── images.rs       # 目录图像加载与缓存
    ├── runtime.rs
    ├── search/
    │   ├── mod.rs          # 搜索窗口与结果交互
    │   ├── drawing.rs      # 搜索输入栏、结果与状态绘制
    │   ├── tooltip.rs      # 完整路径及错误信息悬停提示
    │   ├── everything.rs   # Everything IPC 查询
    │   ├── everything_settings.rs
    │   ├── hotkey.rs       # 全局搜索快捷键
    │   └── tests.rs
    ├── drag_drop/
    │   ├── mod.rs
    │   ├── target.rs       # OLE 拖放注册
    │   ├── image.rs        # 拖动预览
    │   └── description.rs  # 临时拖放描述
    ├── settings.rs
    ├── settings/
    │   ├── layout.rs
    │   └── tests.rs
    ├── hybrid.rs           # 会话、桌面输入与同步顺序
    ├── hybrid/
    │   ├── audit.rs        # 后台 STA、请求通道与在途状态
    │   ├── audit_schedule.rs # 空闲退避与通知失效
    │   ├── inventory.rs    # 清单合并与身份修订
    │   ├── icons.rs        # 图标批次、刷新与像素接收
    │   ├── icons/tests.rs  # 图标加载与回收回归
    │   ├── icon_changes.rs # Shell 图标通知解码与合并
    │   ├── image_retention.rs # 闲置图像预算与期限
    │   └── rename_transaction.rs # 改名身份交接
    ├── scaled_icons.rs     # 绘图线程共享的 CPU 缩放缓存
    ├── acrylic.rs
    ├── acrylic/effects.rs
    └── …                   # 各功能共享的绘图、布局、窗口和输入模块
```

搜索配置和快捷键仅在 `pane` 范围内可见；调用方显式从 `search` 导入，
父模块只暴露功能所需的入口。拖放描述是 `drag_drop` 内部实现。
窗口、模型、绘图等多个功能共用的模块继续留在 `pane`，避免仅按名称搬动后增加循环依赖。

## 库与生成文件

- `desktop-core/src/`：`identity`、`geometry`、`appearance`、`item`、`panel`、`workspace`，由 `lib.rs` 重导出公共类型。
- `desktop-storage/src/`：`lib.rs` 导出 API，`error.rs` 定义错误，`store/` 包含事务、配置、恢复、编解码和测试。
- `desktop-shell/src/`：`lib.rs` 导出 API，`namespace`、`desktop`、`notification`、`activation`、`apartment`、`error` 及文件操作、原生菜单、布局和重命名模块分别维护。
- `desktop-hook/src/filter.rs` 和 `filter/`：当前视图成员过滤后端、客户端、IPC、Shell 项目恢复。
- `app/src/pane/hybrid/inventory.rs`：合并原生视图与独立桌面来源，保留已过滤的分组身份。
- `app/src/pane/hybrid/audit.rs`：封装后台审计的通道、在途请求和调度操作；`hybrid.rs` 核验返回的成员修订，再更新模型和发布过滤名单。
- `app/src/pane/hybrid/icons.rs`：集中初次加载、通知刷新、迟到结果校验和缓存维护；期限与预算策略由 `image_retention.rs` 管理，相关集成回归位于 `icons/tests.rs`。
- `app/src/pane/folder/preferences.rs`：集中默认视图、显示列和列宽配置；列位掩码的兼容校验与显示列保存各保留一处实现。文件夹激活从事件入口直接调用目录导航，系统打开仍使用延迟执行路径。
- `app/src/pane/scaled_icons.rs`：复用纯像素缩放结果，不持有或跨线程共享 COM 绘图资源；GPU 纹理仍由 `render.rs` 管理。
- `desktop-hook/src/discovery.rs`：控制端发现与冲突检测；`notifications.rs` 定义桌面输入通知。
- `desktop-graphics/src/bindings/`：DWM/DComp 生成绑定，`layer.rs` 管理合成层；`desktop-window/src/`：显示器枚举和错误提示。

各 crate 的入口、内部目录和依赖约定见 [crates 导航](../../crates/README.md)。

应用通过 `FilterSession` 连接成员过滤后端；会话、IPC 与 Explorer 端引擎分别归属对应 crate 的 `filter/` 模块。

## 新增文件约定

1. 功能专用的实现放入功能目录，多处共用的实现才提升到共同父模块。
2. 使用常规 `mod` 与 Rust 目录规则；`name.rs` 配合 `name/` 和 `name/mod.rs` 都是有效组织方式，不为统一形式而搬动文件。
3. 较大的单元测试模块放在所属目录的 `tests.rs`；跨模块集成测试保留在 crate 的 `tests/`。
4. 生成文件移动时同步生成器，验证工具的相对引用也需一起检查。
5. 示例中用于复用实现的 `#[path]`、生成绑定的 `include!` 按实际用途保留。
6. 移动源码后运行全目标编译和受影响测试；同步更新开发指南和文档链接。
