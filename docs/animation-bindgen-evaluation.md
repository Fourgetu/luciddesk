# Animation / Bindgen 评估（2026-09-11）

## 后续接入结果

后续菜单首帧修正：内容层的初始透明度在 SetRoot / Commit 前设置，材质根节点的初始透明度在挂载前设置；菜单保持隐藏直到首帧准备完成，再显示并继续淡入。初始透明材质、首帧上传及延迟终帧检查通过。最新应用测试 67 passed / 3 ignored。真实键盘消息测试与其他 UI 测试同进程运行时出现访问冲突，独立运行正常；现放入独立子进程执行全部原生消息断言，整套回归通过。该跨测试干扰的具体原生调用原因尚未定位，未将进程隔离描述成应用崩溃修复。

用户随后提供 Keyboard regression 测试程序的访问冲突弹窗截图。复查时旧测试和 WerFault 进程已不存在，混合模式主程序仍运行。现在仅在隔离测试子进程启用 SEM_NOGPFAULTERRORBOX，避免原生失败阻塞于弹窗；父进程限制 30 秒，超时结束并回收子进程，失败报告保留退出状态及输出。此措施不吞掉失败，不改变主程序错误处理，也不代表原生访问冲突根因已修复。修改后重新运行应用测试：67 passed / 3 ignored；退出后无测试或 WerFault 残留。

已接入 `windows-animation = 0.100.0`，菜单 120ms 淡入由 Animation Manager 提供同一个透明度值，同时应用到 DComp 内容层与原生材质层。首帧准备完成后才启动计时，延迟帧仍提交终值；初始化或采样失败则直接显示完整菜单。系统禁用动画时跳过动画。分组折叠仍保留原 cubic 曲线及真实 HWND 高度更新。

本次没有启用合成线程独立播放，仍使用原消息循环调度；不能据此宣称解决 UI 线程阻塞或提升帧率。背景仍使用原生 Windows.UI.Composition，不依赖 Windows App SDK。

新增私有 `desktop-graphics` crate，实际接管内容层 DComp 的设备、目标、visual、透明度效果及 Commit 调用，并提供生成的 DWM 绑定。其类型使用 core 0.100；应用仍需的 core 0.62 边界保留借用互操作。Bindgen 位于独立 `tools/windows-bindings` 工具中，按 API 清单生成并保存源码，支持 `--check` 一致性检查，不加入应用构建依赖。

验证：应用测试 66 passed / 3 ignored；Canvas 兼容测试 4 passed；示例构建通过；生成绑定一致性检查通过。包含菜单淡入中点、禁用动画、延迟终帧、真实隐藏 HWND 的 DComp / Acrylic 透明度应用、GPU 像素及缩放回归。未宣称完成可见桌面的逐帧同步测量。

## 接入前评估记录

约束：维持 Rust + Win32 + Canvas，使用现有 Hook 混合模式，不引入 Reactor、WinUI3 或 Windows App SDK。本次只增加独立探针和评估记录，未改变应用依赖或运行中的程序。

## 结论

- **windows-animation：有条件采用，暂不整体替换。** 当前两种动画很简单；真正值得试验的是将视觉属性曲线交给合成线程，减少 UI 线程繁忙时淡入停顿。单纯替换数值插值不会消除 WM_TIMER、SetWindowPos 或重绘。
- **windows-bindgen：适合先做小范围试点。** 私有 DWM / DirectComposition 绑定有明确可行性；依赖缩减、编译时间和产物大小需在实际替换后测量。只新增生成文件、保留所有旧依赖，不会自动解决两套 windows-core 的互操作。

## 当前代码对应关系

| 部分 | 当前实现 | 迁移判断 |
| --- | --- | --- |
| 分组折叠 | `preview/animation.rs` 计算 200ms cubic ease-out；`window.rs` 用 16ms WM_TIMER 更新真实 HWND 高度和 reveal | Animation Manager 可管理数值及中断，但仍须逐帧 SetWindowPos；当前收益有限 |
| 菜单淡入 | `menu.rs` 计算 120ms 透明度；`Surface::opacity` 同时更新 DComp 内容层及 Windows.UI.Composition 材质层 | 可试验合成线程动画，但必须同步两个合成体系的起点、终点和取消逻辑 |
| Canvas 互操作 | `composition.rs` 在 core 0.100 与 windows/core 0.62.2 之间做 QueryInterface 和 HRESULT 转换 | Bindgen 可让一个完整的私有模块采用 core 0.100；未迁移的调用边界仍需转换 |
| Hook / Shell | 原生消息、COM、PIDL、通知和文件操作覆盖面较广 | 不作为第一次生成绑定的迁移对象，避免把窄范围试验扩成 ABI 和生命周期重构 |

## windows-animation 0.100.0

已检查发布包，依赖 windows-core 0.100.0，无 Reactor / Windows App SDK。公开接口包含 Manager、Variable、TransitionLibrary、Storyboard、Keyframe；常用过渡包括 linear、accelerate_decelerate、instantaneous。

独立真实 COM 探针在 STA 和 MTA 下均通过：

1. 400 -> 38 的折叠在 80ms 时重新转向 400，反向起点位置连续。
2. 跳过中间时间点后仍到达终点。
3. 120ms 线性淡入在中点为 0.5，延迟更新后为 1.0。
4. 将 Variable 曲线复制到真实 IDCompositionAnimation，设置到 IDCompositionEffectGroup，并成功 Commit。

这里验证的是 API、数值与合成调用链，不是可见窗口上的帧率或流畅度。没有把曲线挂到当前 Pane，也未验证其与背景材质的同步。原生真实 HWND 高度变化不能仅靠视觉缩放替代，否则鼠标区域和布局会与显示不一致。

默认曲线也不等价：同为 200ms，使用 accelerate_decelerate(0, 1) 时 80ms 的高度为 168.320；现有 cubic ease-out 为 116.192。采用该预设会改变手感。公开封装还没有满足所有复杂中断、速度控制、完成回调需求的高层接口；调用方仍须控制帧调度、结束和系统关闭动画的设置。

建议只在后续出现多属性编排需求，或准备同时处理内容层与材质层动画时接入。针对单一线性淡入，直接使用已有 DComp / Composition 原生动画也值得比较，不必为了直线曲线额外增加 Animation Manager。

参考：[官方指南](https://github.com/microsoft/windows-rs/blob/master/docs/crates/windows-animation.md)、[曲线复制接口](https://github.com/microsoft/windows-rs/blob/master/crates/libs/animation/src/variable.rs)。

## windows-bindgen 0.100.0

探针采用独立 workspace，生成器为 build-dependency；实际运行只需要生成代码及 windows-core / windows-link，不需要携带生成器或 metadata。

| 绑定筛选范围 | 实际生成源码 |
| --- | --- |
| DwmSetWindowAttribute + SHQueryRecycleBinW，flat + sys | 823 bytes / 25 行 |
| DComp Device / Animation / EffectGroup 整接口，flat + minimal | 71,288 bytes / 1,950 行 |
| 收紧到创建设备、创建动画/效果组、SetOpacity、Commit 所需方法 | 6,756 bytes / 194 行 |

精简后的 DComp 绑定已与 windows-animation 0.100.0 直接互操作并执行成功，不需要旧版 core 的桥接。数据是生成源码量，不代表二进制减少量，也不构成运行性能结论。

推荐落地方式：

1. 先整理一个私有 DWM / DComp 模块，确定稳定的 API 筛选清单。
2. 用独立工具生成并保存绑定，固定生成器和锁文件，避免每次应用构建重新生成。
3. 公共业务接口继续使用项目自身类型；生成的 COM 类型限制在实现内部。
4. 对照 ABI、错误路径、COM 所有权和设备重建进行验证，再删除该模块不再需要的旧绑定。
5. 比较迁移前后的依赖树、冷构建时间和发布产物大小后，再决定是否扩大范围。

现有 `windows` 0.62.2 类型和新生成类型不是同一种 Rust 类型；即便 GUID / ABI 一样，也不能直接混用或靠转移裸指针所有权解决。Bindgen 不提供窗口/COM 生命周期策略，不会修复通知遗漏、缓存过期、命中范围或线程阻塞等应用问题。

发布包与 master 有差异：此探针使用已发布版本支持的 API 名称筛选；直接使用部分 master 文档式完整路径筛选在本机发布 metadata 下解析失败。另外 core 0.100.0 发布包尚无文档示例中的 init_sta，STA 探针使用生成的 CoInitializeEx / CoUninitialize 配对。

参考：[官方指南](https://github.com/microsoft/windows-rs/blob/master/docs/crates/windows-bindgen.md)。

## 复现

探针位于 `tools/dependency-evaluation`，独立锁定依赖，不加入应用 workspace：

```powershell
cargo run --locked --manifest-path tools/dependency-evaluation/Cargo.toml -- --sta
cargo run --locked --manifest-path tools/dependency-evaluation/Cargo.toml
```

不打开窗口、不访问回收站内容、不修改桌面、不加载 Hook。生成的 DWM / Shell sys 绑定只参与编译；执行验证使用独立的 COM / DComp 对象。
