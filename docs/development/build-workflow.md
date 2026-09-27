# 日常构建与验收

日常开发验收使用 Debug；性能测量使用 Release。不要用开启诊断功能的 Release 代替 Debug。

```powershell
cargo build -p lucidpane -p desktop-hook --locked --offline
# 正常退出旧实例后，启动 target/debug/lucidpane.exe。
# 两个包一起构建，保证主程序和相邻 desktop_hook.dll 同步。
```

性能测量：增加 `--release`，正常退出旧实例后启动 `target/release/lucidpane.exe`。
常规开发复用根 target 目录，避免为每次验证创建新的 --target-dir。
保留默认完整调试信息和增量编译，便于排错并加速后续修改；不以删除调试信息压缩开发构建。

## 2026-09-28 空间检查

以下为文件逻辑大小合计，不代表安装包体积或精确物理磁盘占用（硬链接可能重复计数）。

| 产物 | 大小 |
| --- | ---: |
| Release 主程序 / Hook DLL | 4.93 / 0.45 MiB |
| Release 构建目录 | 约 2.44 GiB |
| Release deps | 2327.8 MiB，其中 rmeta 1173.8、rlib 1107.1 |
| 新构建 Debug 主程序 / Hook DLL | 8.71 / 1.17 MiB |
| Debug 主程序 / Hook PDB | 63.84 / 22.86 MiB |
| Debug deps / incremental / build | 564.9 / 261.4 / 62.3 MiB |

主要原因是 Windows API 绑定的编译元数据和依赖缓存、不同 feature/检查/测试构建留下的多组产物，
以及之前使用多个 target-dir 重复构建依赖。Release deps 中单份 windows rlib 约 90～98 MiB，
rmeta 约 90 MiB。新 Debug 的 windows rlib/rmeta 约 163.6/81.7 MiB。

依赖树同时包含 windows-core 0.62.2 与 0.100.0：前者来自 windows API 绑定，后者来自
windows-canvas/window 等图形窗口库。不能仅为去重直接替换版本；应在对应库支持下进行迁移。

已清理第一轮 20 个旧目录（55.30 GiB）及后续发现的两个深层实验构建目录（3.416 GiB），
保留历史测试数据和测量记录。清理清单位于忽略的 target/audit-cleanup-manifest.json 和
 target/audit-nested-cleanup.json。清理不是日常编译步骤：反复全量清理会丢失可复用缓存。
