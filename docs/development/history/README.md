# 历史资料

本目录保存早期架构提案、开源项目调研、兼容性故障、性能与交互实验。它们用于解释决策来源，不是当前构建或使用手册。

原记录中的版本号、耗时、测试数量和“当前”描述均属于当时环境。命令或源码路径可能已归档；请从[开发入口](../README.md)获取现行指南，不绕过历史故障涉及的系统校验。

| 文档 |
| --- |
| [截至 0.10.3 的原始技术变更记录](changelog-through-0.10.3.md) |
| [Hook 与绘图消融实验（2026-09-14）](hook-render-ablation-20260914.md) |
| [存储层代码消融实验（2026-09-13）](storage-ablation-20260913.md) |
| [Animation / Bindgen 评估（2026-09-11）](animation-bindgen-evaluation.md) |
| [Canvas 接入验证（2026-09-11）](canvas-evaluation.md) |
| [桌面尾部插入边界修正（2026-09-09）](desktop-tail-insertion-20260909.md) |
| [GitHub 类 Fences 项目与 Explorer Hook 路线核查](github-fences-hook-survey-20260908.md) |
| [原生 Hook 后端验证与限制（2026-09-08）](hook-backend-validation.md) |
| [桌面首次按下与拖动](hook-first-press-20260909.md) |
| [Hook 性能优化记录（基线 269831c）](hook-performance-20260909.md) |
| [分组图标刷新验证（2026-09-11）](icon-refresh-validation.md) |
| [原生桌面与 pane：路线复核](native-desktop-options.md) |
| [ViPad 与其他仿 Fences 项目补充调查](open-source-desktop-survey-vipad.md) |
| [开源桌面分组实现调查](open-source-desktop-survey.md) |
| [LucidPane 新方案：以 Fences 式使用体验为目标](redesign-fences.md) |
| [DesktopFramesPlus 与 MiniFences 技术路线调研及 LucidPane 借鉴建议](reference-projects-desktopframesplus-minifence.md) |
| [LucidPane 桌面图标管理程序技术方案](technical-design.md) |
| [Windows 11 原生精简菜单：宿主接口调查](windows11-native-menu-research.md) |

当前文档中的 `architecture.md` 整理原 `mode-separation.md`，`rendering.md` 整理原 `canvas-migration.md`；混合桌面说明已按单后端、v9 存储重写。生成绑定工具说明合并到现行开发指南。
