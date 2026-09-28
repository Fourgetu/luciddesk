# Rust 依赖更新（2026-09-29）

## 更新范围

| 依赖 | 原版本 | 更新后 |
| --- | --- | --- |
| rusqlite | 0.37.0 | 0.40.2 |
| libsqlite3-sys | 0.35.0 | 0.38.2 |
| toml_edit | 0.23.10+spec-1.0.0 | 0.25.15+spec-1.1.0 |

同步更新 Cargo 允许范围内的间接依赖，包括 bitflags、cc、cfg-if、find-msvc-tools、hashlink、rustix、smallvec、syn 和 unicode-ident。两个独立开发工具的锁文件同步更新 unicode-ident。

版本依据：[rusqlite 发布记录](https://github.com/rusqlite/rusqlite/releases/tag/v0.40.2)、[toml_edit 版本文档](https://docs.rs/toml_edit/0.25.15+spec-1.1.0/toml_edit/)，以及 Cargo 在线解析结果。

## 兼容处理

- rusqlite 新版将 u64/usize 的 SQL 转换放在 `fallible_uint` feature 下。面板 ID 使用 u64，显式启用该功能以保留原有范围检查，未改变数据库格式。
- 保留 bundled SQLite、备份 API 和 `.cargo/config.toml` 中已有的扩展裁剪。存储测试继续验证编译选项、配置读写、布局与备份恢复。
- 系统 API 的 windows 0.62.2、windows-core 0.62.2、windows-numerics 0.3.1 与 Canvas 的 0.100.0 系列属于两组配套依赖，保留其兼容边界；不能只替换单个版本来消除重复项。
- rusqlite 新增的 wasm 目标依赖会出现在跨平台锁文件中，但不属于 Windows 目标的正常依赖树。
- 工作区最低 Rust 声明保持 1.95；本次验证使用本机 stable 1.98.1，未单独运行 1.95 工具链验证。

## 验证

以下检查通过：

```powershell
cargo check --workspace --all-targets --locked --offline
cargo test --workspace --lib --bins --tests --locked --offline -- --test-threads=1
cargo test --workspace --doc --locked --offline
cargo build -p luciddesk -p desktop-hook --release --locked --offline
cargo run --manifest-path tools/windows-bindings/Cargo.toml --locked --offline -- --check
cargo check --manifest-path tools/dependency-evaluation/Cargo.toml --locked --offline
```

应用测试 234 项通过、14 项按原有条件跳过，Canvas 集成测试 4 项通过；工作区其他测试亦通过。需要特定外部程序或人工桌面交互的忽略项未启用。保留示例未使用函数与 MSVC 链接器提示等现有警告。

应用版本保持 0.10.3，新增变更列入“未发布”，没有重打或覆盖先前的便携包。用户版 README 与 CHANGELOG 聚焦操作和体验，原技术变更记录保存在 [历史归档](history/changelog-through-0.10.3.md)。
