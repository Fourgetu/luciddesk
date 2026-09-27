# Release LTO 体积对比

2026-09-28，rustc 1.98.1 / Cargo 1.98.1。同一份源码，分别使用全新 target 目录，
离线构建 lucidpane 和 desktop-hook。每组仅执行一次，耗时包含依赖编译与链接；
不代表重复统计、日常增量构建或运行性能。未修改 opt-level、图标、功能和 panic 策略。

| 配置 | EXE MiB | DLL MiB | 合计 MiB | 缩小 | 全新构建秒 |
| --- | ---: | ---: | ---: | ---: | ---: |
| 默认 Release | 4.936 | 0.449 | 5.384 | 0.00% | 48.5 |
| Thin LTO + codegen-units=1 | 4.697 | 0.415 | 5.112 | 5.06% | 84.2 |
| Fat LTO + codegen-units=1 | 4.636 | 0.415 | 5.051 | 6.19% | 98.8 |

结论：此项目收益有限，暂不改变默认 Release 配置。Fat 相比 Thin 只再节省 62.5 KiB；
需要更小发布包时可以接受额外链接成本，再选用 LTO 并做对应版本的功能验收。
本次三个版本均编译成功，但未执行候选版本的运行/交互回归，也未测运行性能。

复现：在新 PowerShell 会话设置 CARGO_PROFILE_RELEASE_LTO=thin（或 fat）、
CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1，然后运行同一条 cargo build --release
-p lucidpane -p desktop-hook --locked --offline。基线不设置这两个覆盖变量。
测试后移除覆盖变量；不修改日常 Debug 配置。

原始结果保存在 target/size-comparison-results.json，三个日志分别为
 target/size-baseline.log、target/size-thin.log、target/size-fat.log。
临时 target/size-comparison 构建目录在比较后清理，当前运行 Debug 保持不变。
