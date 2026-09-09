# 桌面首次按下与拖动

修复首次按住未选中图标时进入框选、必须先选中才能拖动的问题。

## 根因与修复

真实 Explorer 日志中，同一图标未选中时按住拖动出现 `LVN_MARQUEEBEGIN (-156)`，已选中时才出现 `LVN_BEGINDRAG (-109)`。两次命中均有正确索引，但 Hook 只返回 `LVHT_ONITEMICON (0x2)` / `LVHT_ONITEMLABEL (0x4)`，遗漏 `LVHT_EX_ONCONTENTS (0x04000000)`。

经本机已校验版本 comctl32 的 PDB 和反汇编确认，`CLVMouseManager::HandleMouse` 会检查内容标志的第 26 位。在 Explorer 的 `RestrictSelectToContents` 模式下，缺少该位的未选中项会走框选路径。原生 `CLVIconView` 命中内容时返回 `0x04000002` / `0x04000004`。

修复仅在已判定命中图标/文字矩形的缓存和扫描路径补齐此标志。保持现有可见图标索引、坐标、候选优先级、空白判断、插入目标和 OLE Drop 不变；没有重新启用曾回退的原生网格逆向命中方案。

[Microsoft 的 LVHITTESTINFO 定义](https://learn.microsoft.com/en-us/windows/win32/api/commctrl/ns-commctrl-lvhittestinfo) 说明了内容命中标志的含义。

## 隐藏项状态隔离

原实现收到任意 `LVN_ITEMCHANGED` / `LVN_ODSTATECHANGED` 都遍历隐藏项，并向它们发送 `LVM_SETITEMSTATE`，即使状态已经清空。隔离的原生控件测试实测：首次按下压缩后的可见图标，会嵌套发生一次隐藏项状态写入。

修改后，普通可见项通知不再触发隐藏清理。隐藏项或包含隐藏项的范围确实获得选择/焦点时才清理；清理前查询实际状态，跳过已清空的项。场景提交、菜单结束仍保留清理，同数量排序后在释放状态借用之后补做清理，避免新隐藏的索引残留选择。

这部分清理减少了冗余状态重入，但用户实测确认它本身没有解决首次拖动问题。

## 回归验证

`geometry_probe` 在独立的原生 ListView 中验证：

- 未选中图标及文字首次按下都获得原生选中/焦点，额外状态写入从 1 次降至 0 次。
- 给该测试控件排入移动和释放消息后，单次按下直接产生 `LVN_BEGINDRAG`，无需预先选中；不向真实桌面发送输入。
- 使用 Explorer 主题和原生 `CListView::SetSelectionFlags` 在测试控件启用内容选择限制，并验证 `RestrictSelectToContents` 确实生效。旧命中标志稳定无法产生 `LVN_BEGINDRAG`；补齐标志后图标、文字都通过。私有函数调用只存在于隔离测试，且先经过同一 DLL 版本校验及函数字节检查。
- 隐藏项直接选择、全选和菜单选择例外保持隔离；同数量排序立即清除新隐藏项的选择。
- 79 个可见图标和文字的命中、尾部/中间/跨列分隔符以及 OLE 代理落点检查保持通过。

`input-trace` 是显式编译开关，正常构建不记录输入；启用时每次按下最多记录 160 项，并在原生处理返回后一次写入 `hook-input.log`。不读取文件内容或名称。

用户已在真实 Explorer 桌面按“先点空白取消选择，再直接按住图标拖动”复测，确认“可以直接拖动了”。确认后恢复默认编译，关闭诊断记录。
