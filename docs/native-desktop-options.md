# 原生桌面与 pane：路线复核

2026-09-07。文档/API 调研，未修改 Explorer 图标、文件属性或运行架构。
本轮优先级：尽量保留原生体验；不关闭自动排列；归入 pane 后不重复。
此前全自绘接管方案不再作为已确定的下一步。

## 已确认的能力与边界

1. IExplorerBrowser 可以承载 Shell 原生视图。FillFromObject 可建立结果集合，
   再通过结果文件夹接口管理成员；微软有 Custom Contents 示例。
   这支持“集合而非真实目录”的原型方向，但不自动让原桌面隐藏这些成员。
   原生 Shell 文件夹视图也不等于 Explorer 桌面所有特有行为。
   [接口](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-iexplorerbrowser)
   [集合填充](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-iexplorerbrowser-fillfromobject)
   [官方示例](https://learn.microsoft.com/en-us/windows/win32/shell/samples-explorerbrowsercustomcontents)

2. FWF_AUTOARRANGE 有公开支持；FWF_TRANSPARENT 明确只用于桌面，FWF_DESKTOP
   也明确不用于一般 Shell 文件夹。因此不能把这两个标志当成任意嵌入视图透明的保证。
   [FOLDERFLAGS](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/ne-shobjidl_core-folderflags)

3. IShellFolderView::RemoveObject 是真实存在的“仅从视图移除项目”的旧接口。
   文档明确提示项目可随时被数据源重新加入，接口未来可能变化或不可用。
   没有验证现代 Explorer 桌面是否向外部进程提供可用对象；不能承诺刷新、重启或云同步后仍隐藏。
   [RemoveObject](https://learn.microsoft.com/en-us/windows/win32/api/shlobj_core/nf-shlobj_core-ishellfolderview-removeobject)

4. IFolderFilter 需要浏览器宿主接收过滤器。它在自建 ExplorerBrowser 中可研究，
   不能据此推断可以向现有 Explorer 桌面安装永久过滤器。
   本次没有找到公开受支持的、覆盖所有桌面项目的持久单项隐藏方案。
   这是检索结论，不是“Windows 内部绝无实现方式”的证明。
   [IFolderFilter](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-ifolderfilter)

## 路线比较

| 路线 | 原桌面体验 | 重复项目 | 主要代价 | 建议 |
|---|---|---|---|---|
| 原桌面 + Shell 原生 pane + 视图移除实验 | 未分组仍是原桌面，pane 使用原生 Shell | RemoveObject 等机制尚未验证持久性 | 刷新重现、透明视图、现代接口可用性 | 做受控可行性原型，不作为稳定承诺 |
| 原桌面 + 收纳目录/快捷方式管理 | 未分组保持原桌面 | 转移实际桌面快捷方式后可避免重复 | .lnk 所在位置改变；真实文件/目录不可默认移动；特殊图标另行处理 | 若接受显式收纳语义，是可控产品备选 |
| 多个原生 Shell 视图，含未分组桌面层 | 复用系统视图而非手工画图标，但不等于原 Explorer 桌面 | 自己分配集合可避免重复 | 仍然接管；透明、桌面特有行为、恢复与层级待验证 | 第二实验路线，不直接替换当前版本 |
| 原桌面外框，不动项目 | 原生 | 没有复制 | 自由分组位置受全局自动排列限制 | 已否定，不重新包装 |
| 自绘全部桌面 | 自己复刻 | 可自行控制 | 原生一致性工作量和偏差最大 | 暂停推进 |
| Explorer 内部挂钩/注入 | 理论上可改变原视图行为 | 理论上能做过滤 | 依赖实现细节，兼容和崩溃影响面大 | 不是默认路线；无证据认定 Fences 必然如此实现 |

隐藏属性不是独立的“仅桌面隐藏”机制，会改变文件/目录属性与普通枚举行为，
也不适用于回收站等所有命名空间项目。不能默认使用。
[SetFileAttributesW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-setfileattributesw)

“快捷方式收纳”只在用户明确接受时转移实际 .lnk 文件，不是移动目标程序，
也不是删除原图标再生成一份相似快捷方式。需要原路径、冲突、公共桌面权限、
OneDrive 同步及退出/卸载还原策略；真实文件、目录、特殊系统图标各自定义行为。

## 建议的验证顺序

先完成两个相互独立、可判定失败的原型，不继续给自绘桌面追加外观补丁。

A. 独立窗口内嵌 Shell 结果视图。只使用测试项目，不隐藏原桌面。
对比字体、阴影、名称换行、选择、多选、键盘、原生菜单、打开和拖放。
分别测不透明原生视图、桌面数据源透明标志、现有亚克力宿主；记录黑底、重绘、DPI、裁剪。
原生控件内部的“移动/删除文件”语义不能直接当成分组成员变更，必须单独拦截与定义。
验收门槛：如果原生图标区无法透明，不宣称完成透明 Fences，明确提出材质只用于标题/外框的取舍。

B. 单项视图隐藏实验。在隔离的 Windows 测试账户/虚拟机上研究接口可用性，
只用自行创建的临时快捷方式；禁止在用户真实桌面先做移除试验。
自动排列始终开启，测试 F5、创建/重命名项目、Explorer 重启、分辨率变化。
记录图标重现、空洞、跳动、接口失败；如果只能靠持续轮询压制重现，判为不适合默认产品方案。

两项都过关才评估“原桌面 + 原生 pane”。只过 A 时，保留原生 pane 成果，
选择显式快捷方式收纳，或者另外评估全部原生 Shell 视图方案；不悄悄改为全自绘接管。

Stardock 的公开资料描述用户功能和自己的布局引擎，未披露足以确定上述内部路线的技术细节。
不从效果反推其使用了某个 API。
[官方引擎更新说明](https://www.stardock.com/news/528036/fences-55-arrives-with-a-new-fences-engine)
