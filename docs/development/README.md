# 开发文档

当前主线采用视图成员过滤架构：Explorer 管理未收纳的原生桌面图标，LucidDesk 自绘分组图标，通过过滤 Hook 隔离已收纳项目，并由独立 Shell 宿主提供原生菜单。配置与数据兼容规则见[存储](storage.md)。

## 开始开发

1. 阅读[构建与验证](build.md)，准备 Rust 与 Windows 构建环境。
2. 阅读[架构说明](architecture.md)，确认修改所在模块的职责。
3. 涉及持久化时阅读[存储与版本约定](storage.md)；涉及绘图时阅读[绘图与绑定](rendering.md)。
4. 根据修改范围执行测试，并在[验证记录](validation.md)中区分自动检查与实际桌面结果。

## 当前实现

| 文档 | 主题 |
| --- | --- |
| [架构说明](architecture.md) | 模块职责、启动流程、Hook 与兼容边界 |
| [目录结构](structure.md) | 源码目录、功能归属与新增文件约定 |
| [原生桌面与成员过滤](hybrid-desktop.md) | 身份、收纳、过滤名单发布与恢复 |
| [绘图与绑定](rendering.md) | Canvas、DComp、WinRT、动画与生成绑定 |
| [背景材质与主题](mica-materials.md) | 四种材质、局部配色、系统策略与回退 |
| [多语言](localization.md) | Fluent 资源、语言选择、字体与验证 |
| [设置组件](settings-components.md) | 设置页布局与共用交互组件 |
| [图标内存管理](memory-optimization.md) | 像素去重、缓存预算与验证边界 |
| [Rust API 与资源约定](rust-api-review.md) | 接口边界、COM 线程与资源生命周期 |
| [面板标签](pane-tabs.md) | 标签状态、切换与持久化 |
| [存储与版本约定](storage.md) | TOML 配置、工作区数据库、备份和开发期版本策略 |
| [图标菜单与重命名](pane-item-rename.md) | 独立 Shell 宿主、原生编辑窗口与改名事务 |
| [面板层级](pane-drag-order.md) | 普通、文件夹和搜索面板的层级与菜单交互 |
| [后台调度](event-driven-runtime.md) | 事件唤醒、在途请求与维护期限 |
| [选择与刷新时序](selection-latency.md) | 选择互斥、后台清单读取和过滤发布 |
| [运行时托盘](tray.md) | 托盘消息、资源释放和验证入口 |
| [验证记录](validation.md) | 最近一次检查结果与未覆盖范围 |

## 文档维护约定

仅维护与当前实现一致的说明；已替代方案、旧测量和临时排查过程通过 Git 历史查询。

- 介绍与使用说明放在 `docs` 根目录，避免混入 ABI、日志开关和测试夹具细节。
- 开发指南放在 `docs/development`，不再追加已失效的实验记录。
- 正文使用中文；代码、API 名称、命令和必要的原始日志保持原样。
- 变更行为时同步更新指南，删除失效描述；验证记录注明环境和局限，不把旧结果作为当前验收结论。
- 相对源码路径以仓库根目录为基准；Markdown 链接以所在文档目录为基准。
