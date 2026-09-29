# 多语言

应用使用 Fluent，七种语言资源位于 `app/locales/*.ftl`，通过 `include_str!` 编译进 EXE。`app/src/i18n.rs` 负责解析、系统语言匹配、英文回退、参数格式化和原生 UTF-16 文本。每种资源首次使用时加载，后续复用；布局测量仅缓存数值，不持有 COM 资源。

## 语言选择

`config.toml` 根级 `language` 默认为 `system`。设置页保存选择后立即刷新文案、字体候选、绘制缓存、窗口标题与托盘。跟随系统时，在收到 Windows 设置变更通知后重新读取已生效的显示语言；Windows 自身需要注销的语言变更仍遵循系统要求。读取 Windows 显示语言，不使用地区数字格式来推断语言。中文按简繁脚本与地区区分；英语、日语、韩语、德语及俄语按语言匹配，其他语言使用英语。

语言不改变 Shell 身份、文件路径、备份文件前缀或用户保存的分组名称。Windows Shell 菜单、系统错误和文件类型描述由系统提供，可能使用系统语言。新建分组使用当前语言的默认名称。

## 修改文案

- 资源键使用稳定的英文名称。修改文案时保留键；每个语言文件同时添加新键与相同的变量。
- 固定文字使用 `i18n::text`；带变量的文字使用 `i18n::format`，变量值与模板分离，文件名中的花括号不会被再次解析。
- 当前参数使用 `arg0`、`arg1` 或已有业务名称。翻译可以调整变量顺序，不能删改变量；新文案优先使用业务名称。
- 窗口标题和原生对话框使用 `i18n::wide`，其缓冲区随资源缓存存活，避免临时 UTF-16 指针失效。
- 资源中的简体中文注释供对照；译文为初稿，欢迎母语使用者校对。不要把同一用户输入当作资源键再翻译。

设置使用 DirectWrite 实际文本测量调整侧栏、卡片高度和选项排列，超宽的选项改为整行。默认字体按语言选择；自定义字体筛选按当前语言的代表字符核验，缺失字形通过 DirectWrite 优先回退到当前语言默认字体，再使用系统回退。字体预览按候选字体绘制，缓存仅保留有限数量的预览格式。

## 验证

```powershell
python tools/check-locales.py
cargo test -p luciddesk --bin luciddesk i18n::tests --locked
cargo test -p desktop-storage --lib language_round_trip_and_legacy_default --locked
cargo test -p luciddesk --bin luciddesk all_languages_layout_and_render_without_control_overflow --locked -- --test-threads=1
```

前三项检查资源键、变量、Fluent 解析、语言匹配和配置兼容；CI 执行资源检查和 Fluent 测试。最后一项在 Windows 中离屏绘制七种语言的设置页面，覆盖深浅主题、100%、150%、200% 缩放及导航宽度。可设置 `LUCIDPANE_TEST_EXPORT_SNAPSHOTS=1` 导出 `target/i18n-语言索引-页面编号.bmp`，检查后移除环境变量。离屏检查不代替母语审校、实际多显示器操作或 Windows Shell 界面验证。
