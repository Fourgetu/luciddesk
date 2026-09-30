# 配置与工作区存储

应用使用 `config.toml`（格式版本 1）和 `workspace.db`。默认目录为 `%LOCALAPPDATA%\LucidDesk`，`LUCIDDESK_DATA_DIR` 可以指定独立目录；便携版使用程序旁的 `data`。数据目录与环境变量规则见[品牌规范](../brand.md)。

## 实现入口与数据路径

| 入口 | 职责 |
| --- | --- |
| `app/src/main.rs` | 数据目录选择 |
| `crates/desktop-storage/src/store/mod.rs` | 数据库打开、偏好路由和工作区保存 |
| `crates/desktop-storage/src/store/config.rs` | TOML 解析、外部修改检测、字段更新与原子替换 |
| `crates/desktop-storage/src/store/schema.rs` | 结构校验与限定升级 |
| `crates/desktop-storage/src/store/tabs.rs` | 标签组元数据读写 |
| `crates/desktop-storage/src/store/recovery.rs` | 一致快照、导出和恢复校验 |
| `app/src/pane/recovery.rs` | 备份策略、历史记录及后台任务协调 |

路径选择依次检查数据目录环境变量、程序旁的 `portable` 文件、默认用户目录。`config.toml` 与选中的 `workspace.db` 同目录，自动备份存于该数据目录的 `backups` 子目录。开发和测试使用独立目录，避免修改日常工作区。

MSIX 桌面 DLL 的 `LocalState\DesktopComponent` 缓存由独立部署逻辑管理，不应将其当作配置与工作区的数据目录；见 [MSIX 打包与运行](../msix.md)。

## 全局配置

根级字段 `language` 保存界面语言：`system`、`zh-CN`、`zh-TW`、`en-US`、`ja-JP`、`ko-KR`、`de-DE`、`ru-RU`。旧配置缺少该字段时默认 `system`，无效值报错；通过设置页修改后立即生效；手动编辑配置文件后使用“重新加载配置”或重启应用。

`config.toml` 是下表所列偏好的持久化来源，界面和外部编辑器共用同一文件。字体、备份策略等未映射到 TOML 的偏好仍保存在数据库 `metadata`，不能将所有全局设置都视为 TOML 字段。示例见 [config.example.toml](../config.example.toml)。

| 节 | 内容 |
| --- | --- |
| `appearance` | 默认主题和材质；每种材质分别保存强度、颜色或不透明度 |
| `panel_defaults` | 圆角、图标网格缩放、边框、吸附、文字模式、底色保护 |
| `show_panels` | 显示面板快捷键的启用状态与组合键 |
| `search` | Everything 开关、路径、可读快捷键 |
| `preview` | Peek / QuickLook 开关、路径、快捷键 |

颜色使用 `#RRGGBB`，不透明度使用 0–1，材质强度使用整数 0–100（50 对应界面的默认，界面显示值为该值减 50）。路径可以使用 TOML 单引号字符串，例如 `'C:\Apps\Everything.exe'`。快捷键支持 Ctrl、Shift、Alt 与空格、字母、数字、F1–F24，保留文件操作和系统组合不能使用。

首次创建使用默认配置，不从旧数据库导入。缺失字段使用默认值；错误类型、非法数值和未知配置版本会报告错误，不覆盖原文件。界面写入只改变对应字段，保留注释和未知字段；配置通过同目录临时文件、同步写入和原子替换保存，没有变化时不写入。

外部修改后，在“设置 → 备份与恢复”点击“重新加载配置”，或重启程序。没有文件监听和半成品编辑自动应用。运行时重新加载失败会保留当前有效设置。界面保存前检查文件是否被外部改动，有冲突时要求先重新加载。

与全局值相同的面板主题、材质保存为继承状态。重新加载配置时继承值跟随全局设置，单独覆盖的值保留在数据库。其他面板位置、标题与文件夹路径不属于全局配置。

应用内部 preference 接口仅将 `config::KEYS` 中的键通过适配层路由到 TOML，文件夹排序键路由到关联表，其余键访问 `metadata`；内部编码字符串不是对外配置格式。内存数据库仅用于测试和临时会话，其偏好保存在内存中。

## 数据库结构

| 表 | 内容 |
| --- | --- |
| `metadata` | 非 TOML 偏好、标签组、内部状态及备份/恢复配置快照标记 |
| `panels` | 标题、几何、类型、主题、材质、折叠、锁定、置顶、自动收起及外观继承状态 |
| `panel_folder_settings` | 文件夹路径、列表/图标视图、排序列与方向，通过外键关联面板 |
| `desktop_items` | Shell 身份、桌面/面板归属和位置，面板归属有延迟外键约束 |
| `monitor_layouts` | 显示器组合下的物理像素几何，通过外键关联面板 |

SQL 对布尔值、类型、几何、颜色、不透明度和归属字段实施约束。

保存工作区在事务内按主键 upsert，只更新发生变化的记录，并删除已经移除的记录。独立选项保存不会重写布局；重复保存同一工作区不会增加数据变更计数。多显示器快照没有变化时也跳过写入。

### 兼容性与限定升级

打开已有数据库时，`schema::validate_and_upgrade` 将必需表和索引的 SQL 定义与当前结构比较，忽略空白和大小写。它不是通用迁移框架，也不是任意旧结构的兼容承诺。

当前允许将 `panel_folder_settings.sort_column` 的已知 `0..2` 约束升级为 `0..3`，以支持大小排序。先校验所有必需结构，再在同一事务内重建该表并保留路径、视图和排序设置；其他不匹配结构报告错误，不通过重建空数据库恢复运行。

标签组另由 `metadata.pane_tabs_v1` 保存并执行成员校验，缺少该键表示独立面板。其读取约束见[普通面板标签页](pane-tabs.md)。配置的 `config_version` 与数据库结构校验是不同机制，不能以修改 TOML 版本号代替数据库升级。

### 工作区保存的提交边界

数据库事务覆盖数据库行，不覆盖整个应用状态。`save_workspace` 在数据库提交前保存关联 TOML；提交失败时尝试恢复之前的配置内容，恢复本身也可能报错。因此不能宣称数据库与 TOML 构成跨文件原子事务。

内存模型、窗口和 Shell 文件操作也不随数据库回滚自动恢复，由调用方协调。成功写入后的变化回调只应排队通知，不应同步重入存储对象。详细接口约束见 [Rust API 与资源生命周期约定](rust-api-review.md)。

## 备份与恢复

导出文件仍为单个 `.db`，方便复制和管理。SQLite Backup API 创建一致快照，`metadata.backup_config` 在备份副本中保存完整 TOML（含注释）。完成全部写入后才发布目标文件；日常工作数据库中不维护该配置副本。

恢复采用同样的结构校验及限定升级规则，不支持任意历史数据库格式。来源以只读方式打开，先在内存副本检查数据库完整性、配置格式、工作区和外键，再替换当前数据库与配置文件。配置文件写入失败时尝试从内存快照回滚数据库，回滚失败也会向调用方报告；跨文件恢复中断时，数据库内的 `pending_config` 记录让下次启动完成配置写入。恢复成功后清除该记录。来源备份始终不修改。

普通 `export_backup` 拒绝覆盖已有目标；显式替换导出通过独立入口先生成完整临时快照，再发布目标文件。恢复到带配置文件的工作区时，备份必须包含有效的 `backup_config`，不能只恢复布局却悄悄丢弃全局设置。

### 自动快照与任务调度

自动快照和手动导出复用底层快照逻辑。默认启用自动备份、间隔 5 分钟、保留 10 份；当前可选间隔为 5、15、30、60 分钟，保留数量为 10、20、50 份。策略保存在数据库中。

自动任务需要满足变化尚未检查、最后一次变化已过去 10 秒、距离上次尝试达到策略间隔，且没有其他备份任务在执行。内容比较使用带类型的逻辑行数据，而非数据库文件字节或单纯写入计数，以避免无实际变化的重复快照。

恢复前另存当前状态，备份任务串行执行，界面仍允许调整策略。备份不包含桌面真实文件、文件夹内容或已安装的外部程序，收纳与布局只保存引用。恢复不会找回已删除的真实文件。

## 修改与验证

新增持久化设置时，先确定它属于 TOML 偏好、面板数据还是内部元数据；同步更新默认值、解析校验、写入映射、备份恢复及示例。修改表结构时明确支持的输入结构，不能仅调整建表 SQL 后假定已有数据库可以打开。

在已准备好的构建环境中运行存储测试：

```powershell
cargo test -p desktop-storage --lib --locked --offline -- --test-threads=1
```

重点回归入口：

| 测试 | 检查内容 |
| --- | --- |
| `external_edits_require_reload_preserve_comments_and_reject_invalid_values` | 外部编辑冲突、注释保留和非法值拒绝 |
| `unchanged_workspace_does_not_rewrite_database_or_config` | 无变化保存不重写数据 |
| `size_sort_upgrades_existing_databases_and_survives_reopen` | 限定结构升级及重开 |
| `folder_sort_upgrade_does_not_modify_an_incompatible_database` | 其他结构不兼容时不进行局部升级 |
| `failed_workspace_commit_preserves_configuration` | 数据库提交失败后的配置恢复 |
| `corrupted_backup_config_is_rejected_before_changing_live_state` | 无效备份在修改当前状态前被拒绝 |
| `interrupted_restore_finishes_config_write_on_reopen` | 中断恢复后的配置补写 |

另外在隔离目录检查外部编辑后重新加载、继承与覆盖、导出再恢复、丢失来源文件及自动保留策略。测试结果应区分存储 API 验证与完整 UI 恢复，不将内存数据库测试当作跨文件中断恢复的证明。环境和实机要求见[构建与验证](build.md)及[验证与兼容边界](validation.md)。
