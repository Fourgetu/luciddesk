# 项目目录结构

更新：2026-09-13。路径均相对仓库根目录。

## 顶层分工

| 目录 | 放置内容 |
| --- | --- |
| `app/` | 可执行程序、窗口交互、应用资源和 UI 集成测试 |
| `crates/` | 领域模型、存储及 Windows 平台能力，按依赖边界划分 |
| `tools/` | 发布打包、资源和绑定生成、独立验证工具 |
| `docs/` | 使用说明、开发指南、设计资料和历史记录 |
| `.github/` | CI 与发布工作流 |
| `target/` | Cargo 产物、测试输出和本地实验数据，不作为源码目录 |

## 应用功能目录

```text
app/src/
├── main.rs                 # 启动与参数
├── tray.rs                 # 托盘生命周期
├── hook_runtime.rs         # Hook DLL 运行副本
├── diagnostics.rs
├── app_icon.rs
└── pane/
    ├── mod.rs              # 应用状态与模块入口
    ├── model.rs
    ├── events.rs
    ├── runtime.rs
    ├── search/
    │   ├── mod.rs          # 搜索窗口与结果交互
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
    ├── hybrid.rs
    ├── hybrid/icon_changes.rs
    ├── acrylic.rs
    ├── acrylic/effects.rs
    └── …                   # 各功能共享的绘图、布局、窗口和输入模块
```

搜索配置和快捷键仅在 `pane` 范围内可见；调用方显式从 `search` 导入，
不在父模块重新暴露旧的平铺模块路径。拖放描述是 `drag_drop` 内部实现。
窗口、模型、绘图等多个功能共用的模块继续留在 `pane`，避免仅按名称搬动后增加循环依赖。

## 库与生成文件

- `desktop-core/src/`：`identity`、`geometry`、`appearance`、`item`、`panel`、`workspace`，由 `lib.rs` 重导出公共类型。
- `desktop-storage/src/`：`lib.rs` 导出 API，`error.rs` 定义错误，`store/` 包含事务、配置、恢复、编解码和测试。
- `desktop-shell/src/`：`lib.rs` 导出 API，`namespace`、`desktop`、`notification`、`activation`、`apartment`、`error` 及原有文件操作、原生菜单、布局和重命名模块分别维护。
- `desktop-hook/src/geometry/profile.rs`：精确映像配置，生成器为 `tools/generate_geometry_profile.py`。
- `desktop-hook/src/geometry/hidden/native_identity.rs`：隐藏成员模块使用的原生身份缓存。
- `desktop-hook/src/client/discovery.rs`：控制端发现与冲突检测，`client.rs` 保留 Hook 会话管理。
- `desktop-graphics/src/bindings/`：DWM/DComp 生成绑定，`layer.rs` 管理合成层；`desktop-window/src/`：显示器枚举和错误提示。

各 crate 的入口、内部目录和依赖约定见 [crates 导航](../../crates/README.md)。

本次整理只移动 Hook 配置和身份文件，并同步模块声明与生成目标；没有重新生成映像签名。

## 新增文件约定

1. 功能专用的实现放入功能目录，多处共用的实现才提升到共同父模块。
2. 使用常规 `mod` 与 Rust 目录规则；`name.rs` 配合 `name/` 和 `name/mod.rs` 都是有效组织方式，不为统一形式而搬动文件。
3. 较大的单元测试模块放在所属目录的 `tests.rs`；跨模块集成测试保留在 crate 的 `tests/`。
4. 生成文件移动时同步生成器，验证工具的相对引用也需一起检查。
5. 示例中用于复用实现的 `#[path]`、生成绑定的 `include!` 按实际用途保留。
6. 移动源码后运行全目标编译和受影响测试；历史文档保留当时的路径，当前开发指南更新为新路径。
