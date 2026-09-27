# 模块依赖与 Release 链接审计

2026-09-28。核对 workspace 清单、生产/测试调用、Cargo normal/build 依赖树和 MSVC 链接 map。
本报告包含初始审计与后续验证结果；已落地的裁剪见下方。

## 代码区粗估

按 target/dependency-audit.map 的代码符号地址间隔归类，同地址折叠符号单列，
包含对齐空间；内联和泛型实例归入实际输出对象。因此不是模块完整体积，也不包含资源、
只读数据和异常表，不能与依赖缓存大小混用。

| 输出模块/库 | 代码区粗估 KiB | 判断 |
| --- | ---: | --- |
| lucidpane | 1468.6 | 业务、UI、内联代码与泛型实例，不能全部视为手写业务代码 |
| SQLite C 引擎 | 1335.6 | 最大第三方部分；数据库需要，但默认扩展可研究裁剪 |
| desktop-storage | 255.4 | 数据库、配置和备份恢复 |
| toml_edit + toml_parser | 155.9 | 配置读写实际使用 |
| desktop-shell | 96.8 | Shell 访问、菜单、拖放、图标等 |
| desktop-hook 客户端部分 | 20.4 | 主程序调用 DLL 所需；不是把整个 DLL 再装进 EXE |
| tempfile | 7.4 | 原子配置写入、备份恢复需要，不仅是测试依赖 |

## 可收窄的部分

1. libsqlite3-sys 0.35.0 bundled 构建默认启用 FTS3、FTS5、RTREE、DBSTAT、扩展加载等。
   在应用与存储代码中未找到这些功能的调用或虚拟表定义。它们是下一轮优先裁剪候选，
   不是删掉 SQLite 本身；应对比构建并运行存储、迁移和备份恢复测试后再采纳。
   上游 build.rs 支持 LIBSQLITE3_FLAGS 的 -D/-U 开关，无须复制维护整份 SQLite 源码。
2. 更正初始审计：Win32_System_ProcessStatus 不能移出生产声明。编译验证发现
   app/src/pane/peek.rs 的 QuickLook/Seer 检测使用 EnumProcesses；已保留该 feature。
3. workspace 的 Foundation_Numerics、Win32_Graphics_DirectComposition 未检索到生产直接使用；
   可作为 feature 收窄候选，但必须验证生成绑定的间接依赖及 examples，不能仅凭文本搜索删除。

## 应保留的依赖

- windows-animation：面板动画；windows-version：诊断信息；windows-numerics：材质使用。
- windows-core：除了显式调用，也被 COM implement 宏展开引用，不能按无直接路径匹配删除。
- 两代 windows-core 分属系统 API 绑定和 canvas/window 库，需配套迁移，不能强行合并版本。
- embed-resource 及其依赖属于构建端，不因出现在 Cargo 树就等于打入最终 EXE。
- 未发现可直接删除且能显著缩小产物的大型顶层依赖；优先验证 SQLite 未使用扩展的裁剪。

原始记录：target/dependency-audit-tree.txt、target/dependency-audit.map、
target/dependency-code-estimates.json。生成 map 的 Release 编译通过；未运行新的候选功能版本。

## 已验证并落地的裁剪

- `.cargo/config.toml` 通过 LIBSQLITE3_FLAGS 关闭 FTS3、FTS5、RTREE、DBSTAT 和动态扩展加载。
  保留 bundled SQLite、JSON、备份 API、事务、外键、线程安全和 API 参数检查。
  配置作用于从本仓库构建的 SQLite，不修改用户数据库；若在仓库外作为依赖构建或环境变量
  被外部覆盖，不能假定相同选项。新增存储测试验证实际 SQLite 编译选项。
- 删除 workspace 中未使用的 Foundation_Numerics、Win32_Graphics_DirectComposition feature。
  图形模块的独立 ABI 绑定保持不变。
- ProcessStatus 保留；没有删除 tempfile、TOML、动画或其他现有功能。

| 默认 Release 产物 | 修改前 | 修改后 |
| --- | ---: | ---: |
| lucidpane.exe | 5,168,128 B | 4,607,488 B |
| desktop_hook.dll | 470,528 B | 470,528 B |
| 合计 | 5.377 MiB | 4.843 MiB |

减少 560,640 B（547.5 KiB），约 9.94%。未启用 LTO，未改图标资源。数字为本机本次构建，
不表示运行速度提升。

验证：存储模块 38 项测试通过（包含数据库升级、配置、备份恢复及编译选项检查），
应用恢复模块 5 项测试通过；全 workspace/all-targets 并显式开启 desktop-menu-diagnostics
的编译检查通过。实验模块有既有未使用函数警告。Release 构建成功，日常启动继续使用 Debug。
原始记录：target/dependency-trim-before.json、target/dependency-trim-after.json、
target/dependency-trim-storage-tests.log、target/dependency-trim-recovery-tests.log、
target/dependency-trim-all-targets.log。
