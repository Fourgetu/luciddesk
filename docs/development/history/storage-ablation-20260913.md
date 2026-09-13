# 存储层代码消融实验（2026-09-13）

本轮检查三处看似重复的保存保护是否可以删除。结论：三处均有保留依据；本轮没有删除生产代码。范围限于存储层，不代表已经完成 Hook、绘图和整个应用的消融。

## 实验方法

- 基线：`6619e57e5ce60001c5255d8d547fcfd3795091d7` 的工作区源码快照；开始时已有文档修改和空的未跟踪 `AGENTS.md`，实验未改动这些内容。
- 环境：Windows x64，`rustc 1.98.1 (48a229cea 2026-09-01)`，MSVC，Cargo debug profile，离线依赖。
- 在 `target/storage-ablation/<时间戳>/` 复制 core 和 storage，固定一次源码快照，再创建五个独立变体。只在副本删除指定条件，生产源码不变。
- 每个变体执行全部 30 项存储测试，并注入 1 项测量测试。测量测试使用真实 `WorkspaceStore` 和独立临时目录；五种场景各重复三轮，每轮 50 次操作，记录变更计数及单次耗时 p50/p95。
- 五种场景分别为相同配置值、字符串不同但数值等价的配置、相同 metadata、未变工作区、每次改变配置。最后一种是正向对照，所有变体每轮均记录 50 次配置写入。

## 结果

下表变更计数为每轮 50 次调用的增量，三轮结果一致。通过数包含注入的测量测试。

| 变体 | 测试通过 / 失败 | 相同配置 | 等价配置 | 相同 metadata | 未变工作区 |
| --- | --- | --- | --- | --- | --- |
| baseline | 31 / 0 | 0 | 0 | 0 | 0 |
| without_value_guard | 31 / 0 | 0 | 0 | 0 | 0 |
| without_source_guard | 30 / 1 | 0 | 50 | 0 | 50 |
| without_both_config_guards | 30 / 1 | 50 | 50 | 0 | 50 |
| without_metadata_guard | 30 / 1 | 0 | 0 | 50 | 0 |

`change_count()` 为 SQLite `total_changes()` 与配置原子写入成功次数之和。metadata 的 50 次表示 SQL 行变更，不等同于 50 次物理磁盘写入；配置场景中的 50 次对应 `atomic_write` 成功执行 50 次。

三轮 p50 的中位数，单位为微秒：

| 变体 | 相同配置 | 等价配置 | 未变工作区 |
| --- | --- | --- | --- |
| baseline | 0.2 | 150.2 | 551.9 |
| without_value_guard | 146.3 | 152.4 | 529.1 |
| without_source_guard | 0.2 | 4066.2 | 4087.8 |
| without_both_config_guards | 3989.2 | 3898.5 | 4081.8 |
| without_metadata_guard | 0.2 | 153.8 | 517.5 |

## 各处代码的贡献

1. `ConfigFile::save` 的 `updates.iter().all(...)`：提前识别完全相同的配置值。移除后现有测试仍通过，后面的源文本比较仍能避免写盘，但每次调用需要克隆、修改、解码及序列化 TOML。相同配置的测量耗时明显增加，因此测试通过不足以支持删除。
2. `ConfigFile::save` 的 `source == self.source`：识别不同字符串经规范化后得到相同 TOML 的情况。本轮使用小数的不同拼写触发该路径；移除后每轮产生 50 次无效配置写入。已有 `unchanged_workspace_does_not_rewrite_database_or_config` 也失败，说明正常工作区保存同样依赖它。
3. `save_preference` 的 `WHERE metadata.value != excluded.value`：避免相同 metadata 被再次计为行更新。移除后 `unchanged_preferences_do_not_count_as_database_changes` 失败。该变体只移除这一个条件，没有移除工作区保存或文件夹排序的其他 SQL 条件。

同时移除两层配置检查用于确认相互补偿关系：第一层保护相同输入，第二层兜住规范化后的相同输出；同时移除后，相同输入也发生写盘。

## 复现与证据

从仓库根目录运行：

```powershell
python tools/storage-ablation.py
```

脚本需要 Python 3、Rust 和已有 Cargo 依赖缓存。它记录基线版本、工作区状态、工具链、快照 SHA-256、具体测试命令、失败测试名和 75 组测量结果。编译失败、基线失败或缺少测量数据会使脚本失败；消融变体的测试失败作为实验结果记录，不被误判为编译失败。原始日志和所有副本保留在 `target`，可逐项检查。

本次记录使用 `target/storage-ablation/20260913-202122-059862/results.json` 及同目录五份 `.log`。原始产物不提交 Git，此文保留测量摘要；后续运行产生独立时间戳目录。

主线库级基线另有 54 项通过、2 项按默认设置跳过。实验五个变体均完整执行；源文本保护及 metadata 保护移除后的失败是观测结果。

## 解释范围

这是 debug 构建下的局部诊断。变体按固定顺序运行，没有随机化跨进程顺序，没有测量整机 CPU、物理 I/O 或 release 性能。亚微秒计时有时钟开销，不据此宣称应用整体加速倍数。没有验证真实桌面交互、Explorer Hook 或 GPU，也没有将测试通过解释为所有输入下的行为等价。
