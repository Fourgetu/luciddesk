# 多语言

应用使用 Fluent，七种语言资源位于 `app/locales/*.ftl`，通过 `include_str!` 编译进 EXE。`app/src/i18n.rs` 负责解析、系统语言匹配、英文回退、参数格式化和原生 UTF-16 文本。每种资源首次使用时加载，后续复用；布局测量仅缓存数值，不持有 COM 资源。

## 语言选择

`config.toml` 根级 `language` 默认为 `system`。设置页保存选择后立即刷新文案、字体候选、绘制缓存、窗口标题与托盘。跟随系统时，在收到 Windows 设置变更通知后重新读取已生效的显示语言；Windows 自身需要注销的语言变更仍遵循系统要求。读取 Windows 显示语言，不使用地区数字格式来推断语言。中文按简繁脚本与地区区分；英语、日语、韩语、德语及俄语按语言匹配，其他语言使用英语。

语言不改变 Shell 身份、文件路径、备份文件前缀或用户保存的面板名称。Windows Shell 菜单、系统错误和文件类型描述由系统提供，可能使用系统语言。新建面板使用当前语言的默认名称。

## 热切换与状态保留

设置保存后由 `pane/runtime.rs` 的通知窗口读取有效语言。语言实际变化时重新检查已保存字体是否适用，在释放 `PaneApp` 借用后向同一 UI 线程的顶层窗口投递 `i18n::CHANGED` 并请求重绘。重复选择同一有效语言不重新广播。手动编辑配置文件仍需执行“重新加载配置”或重启，不监听配置文件变化。

- 普通、文件夹和搜索面板沿用现有窗口及绘制表面，仅刷新语言相关文本格式；不重建面板或重新获取项目，不主动取消重命名，也不重置选择与滚动位置。
- 设置页保留当前页面、字体搜索词和滚动偏移，清除旧布局的悬停、按下和滚动条拖动状态。布局改变后的有效滚动范围仍由页面布局计算。
- 设置绘制器同时检查字体和语言；创建失败时保留已有绘制器，下次绘制重试。字体偏好读取失败会记录错误，但不能阻止已改变语言的通知。
- 搜索面板标题、设置标题与托盘响应通知；用户标题和文件名保持原样。已经打开的系统菜单及系统错误文字不由应用重新翻译。

`i18n::CHANGED` 使用 `WM_APP + 200`，与面板恢复、关闭完成、关闭请求的 `WM_APP + 196..=198` 分离。新增线程级广播消息时必须检查所有接收窗口的消息编号；关闭动画还须验证 `request_close` 设置的状态，不能把任意消息当作删除面板的请求。

## 修改文案

- 资源键使用稳定的英文名称。修改文案时保留键；每个语言文件同时添加新键与相同的变量。
- 固定文字使用 `i18n::text`；带变量的文字使用 `i18n::format`，变量值与模板分离，文件名中的花括号不会被再次解析。
- 当前参数使用 `arg0`、`arg1` 或已有业务名称。翻译可以调整变量顺序，不能删改变量；新文案优先使用业务名称。
- 窗口标题和原生对话框使用 `i18n::wide`，其缓冲区随资源缓存存活，避免临时 UTF-16 指针失效。
- 资源中的简体中文注释供对照；译文为初稿，欢迎母语使用者校对。不要把同一用户输入当作资源键再翻译。

设置使用 DirectWrite 实际文本测量调整侧栏、卡片高度和选项排列，超宽的选项改为整行。默认字体按语言选择；自定义字体筛选按当前语言的代表字符核验，缺失字形通过 DirectWrite 优先回退到当前语言默认字体，再使用系统回退。字体列表仅绘制名称，不创建候选字体预览。首次进入字体页时后台枚举并筛选，窗口关闭后释放候选列表；切换语言时取消旧任务，清理轮询定时器，按需启动新语言的任务并重新应用保留的搜索词。

## 验证

```powershell
python tools/check-locales.py
cargo test -p luciddesk --bin luciddesk i18n::tests --locked
cargo test -p desktop-storage --lib language_round_trip_and_legacy_default --locked
cargo test -p luciddesk --bin luciddesk all_languages_layout_and_render_without_control_overflow --locked -- --test-threads=1
```

热切换回归在 Windows 上运行：

```powershell
cargo test -p luciddesk --release --target-dir target/production --locked --bin luciddesk language -- --test-threads=1
cargo test -p luciddesk --release --target-dir target/production --locked --bin luciddesk language_switch_preserves_panel_items_selection_and_scroll -- --ignored --test-threads=1
cargo test -p luciddesk --release --target-dir target/production --locked --bin luciddesk native_font_search_tracks_window_and_handles_clear_and_page_leave -- --ignored --test-threads=1
```

后两项创建原生合成窗口，必须各自独立运行以隔离 STA 图形生命周期。具体通过范围和未验证项见[验证记录](validation.md)。

资源验证命令的前三项检查资源键、变量、Fluent 解析、语言匹配和配置兼容；CI 执行资源检查和 Fluent 测试。最后一项在 Windows 中离屏绘制七种语言的设置页面，覆盖深浅主题、100%、150%、200% 缩放及导航宽度。可设置 `LUCIDPANE_TEST_EXPORT_SNAPSHOTS=1` 导出 `target/i18n-语言索引-页面编号.bmp`，检查后移除环境变量。离屏检查不代替母语审校、实际多显示器操作或 Windows Shell 界面验证。
