# Rust API 与模块结构审查

日期：2026-09-13。

依据 [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/checklist.html)，
重点检查公共领域模型、资源守卫和 crate／模块边界，并对整个工作区运行编译与 Clippy。
这不是对全部 Win32 unsafe 路径的完整安全审计，也不表示工作区已满足全部指南条目。

## 已处理的问题

| 优先级 | 问题与修改 | 指南 |
| --- | --- | --- |
| P1 | `ShellApartment` 原为可直接构造的公共单元结构，自动实现 Send/Sync；现在通过私有 PhantomData 阻止跨线程传递和未初始化构造，提供 Debug、must_use 与生命周期文档 | C-STRUCT-PRIVATE、C-VALIDATE、C-DEBUG |
| P2 | `Panel::new` 直接接受公开 RectDip 字段，能绕过 set_rect 的最小尺寸处理；现在两个入口一致 | C-VALIDATE |
| P2 | 面板用 search 与 folder 分别记录互斥状态；改为私有 PanelSource 枚举，保留已有 setter 行为，包括清除空文件夹时不退出搜索 | C-CUSTOM-TYPE |
| P2 | desktop-core 根文件混合多个领域，材质实现位于测试之后；拆分为六个领域模块和测试模块，并维持根级重导出 | C-HIDDEN、C-CRATE-DOC |
| P2 | 存储根文件混合错误定义和大量测试；独立 error.rs、tests.rs，为 WorkspaceStore 提供 Debug | C-GOOD-ERR、C-DEBUG |
| P3 | PanelId、MonitorId 缺少常用转换／显示 trait；增加 From、Display，MonitorId 增加 AsRef<str> 和消费式 into_inner | C-CONV-TRAITS、C-CONV |
| P3 | 图标通知与设置布局通过 path 属性放在父目录；移入所属模块目录，文件内容保持不变 | 项目模块组织约定 |

OLE 的线程与配对要求依据 [Microsoft OleInitialize 文档](https://learn.microsoft.com/en-us/windows/win32/api/ole2/nf-ole2-oleinitialize)。
每次成功调用，包括重复初始化返回的 S_FALSE，都必须有对应的反初始化。

生成 DComp 绑定中的 HWND 保持 Windows ABI 命名，只在包含它的私有模块局部允许 acronym lint，未修改生成文件。
独立开关、坐标数据结构的公开字段没有机械改成枚举或 getter；它们没有相应的互斥语义或封装收益。

## 验证

- 全工作区 `cargo check --workspace --all-targets --offline` 通过，示例仍有既有 dead_code 告警。
- 核心、存储、Hook、Shell 库测试合计 52 项通过，2 项交互测试按原标记忽略。
- 主程序测试 127 项通过，9 项按原标记忽略；Canvas 集成测试 4 项通过。
- 核心／Shell 文档测试 5 项通过，包含禁止 Send、Sync、直接构造的编译失败测试。
- 新增回归覆盖尺寸入口一致性、内容来源切换，以及嵌套 OLE 初始化的配对释放。
- `cargo clippy -p desktop-core --all-targets --offline -- -D warnings` 通过。
- 全工作区普通 Clippy 通过，但仍有 pedantic 告警，不能声称全工作区通过 `-D warnings`。

## 剩余范围

应用和平台层还有较长函数、通配符导入、数值转换与文档告警，需结合 Win32 数据范围逐项判断。
本次未批量应用 Clippy 自动修复，也未重写 Explorer Hook 或渲染架构。
自动测试不能代替真实桌面的拖放、托盘、Everything 和混合 DPI 手工验收。
