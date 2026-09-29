# 文档导航

LucidDesk 为 Windows 桌面提供分组、文件夹和 Everything 搜索面板。功能示意与快速开始见[项目首页](../README.md)。

## 使用 LucidDesk

| 任务 | 阅读入口 |
| --- | --- |
| 第一次启动、整理桌面 | [启动与三种面板](usage.md#启动) |
| 查看文件操作与快捷键 | [键盘操作](usage.md#键盘操作) |
| 调整文字、圆角与背景 | [外观设置](usage.md#外观设置) |
| 找到配置、导出和恢复备份 | [数据与备份](usage.md#数据与备份) · [配置示例](config.example.toml) |
| 排查连接与启动问题 | [常见问题](usage.md#常见问题) |
| 了解预览包与版本变化 | [预览包说明](preview.md) · [版本记录](../CHANGELOG.md) |

## 开发与维护

从[构建指南](development/build.md)开始，再按改动范围查看[开发文档](development/README.md)。[目录结构](development/structure.md)说明文件归属，[crates 导航](../crates/README.md)解释库边界，[Rust API 与资源约定](development/rust-api-review.md)说明接口与资源生命周期约束。

0.10.5 的面板交互规则见[面板层级](development/pane-drag-order.md)，测试结果和未验证范围见[验证记录](development/validation.md)。Windows 11 x64 为优先维护平台，Windows 10 已由用户完成实机验证。

文档仅保留当前使用与维护所需的说明；旧方案和历史实验可通过 Git 历史查询。
