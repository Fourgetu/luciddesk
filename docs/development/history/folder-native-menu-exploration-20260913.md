# 文件夹原生 Win11 菜单探索（2026-09-13）

## 结论

复用已打开且位于目标父目录的 Explorer 视图，已在本机实际弹出非桌面文件的 Win11 精简菜单。当前不接入生产入口：需要处理 Explorer 窗口前置、没有对应目录窗口、标签页、菜单关闭和选择恢复等条件。独立进程中的 ExplorerBrowser 还不能提供同样的 XAML 菜单。

本轮只添加隔离探针，未修改生产菜单路径、未重启应用、未执行剪切、复制、删除或重命名。曾准备的自绘方案已撤回。

## 实验结果

### 独立 Shell 视图及 presenter

运行原有 `menu_host_probe --initialize E:\Project\LucidPane\README.md`。受限执行环境之前在 FillFromObject 返回 E_FAIL；本轮在可访问交互桌面 COM 的环境复验成功。

- ExplorerBrowser 创建、初始化、结果目录和单项选择成功。
- IContextMenu 可取得。
- 私有 presenter 的已知 vtable 与既有版本检查完全匹配。
- Initialize 返回 S_OK，`presenter_xaml_enabled=0`，Close 成功。

这证明此前的 E_FAIL 不能当作功能不支持的证据，但初始化成功也不等于启用了现代菜单。本轮未修改私有字段、挂钩或伪装 Explorer 进程。

### 把外部 IContextMenu 交给桌面 site

新探针默认使用 README.md 的 IShellItemArray 创建 IContextMenu，成功查询到 34 项传统菜单命令。桌面 IContextMenuSite 可取得，但 `DoContextMenuPopup` 返回 E_INVALIDARG，该次前台交接也返回 false。

仅能说明这一组调用没有成功，不能据此断言所有跨宿主方式都不可能。尤其不可忽略宿主服务、前台权限和选择上下文的差异。

### 复用真实 Explorer 文件夹视图

新探针增加 `--folder-view --show`：

1. 枚举 IShellWindows，查询活动 IShellView / IFolderView2。
2. 将该视图的文件系统目录与目标的父目录核对。
3. 通过完整 Shell parsing name 定位目标条目。
4. 保存选择和焦点，临时选择目标。
5. 将 Explorer 窗口前置，复用现有 WM_RBUTTONDOWN / WM_CONTEXTMENU 桥接。
6. 观察弹窗关闭，恢复原选择。

实际目标为 Downloads 中的 Maskit.app.tar.gz.sig，定位 index=2。观察到：

- 原生 XAML `Microsoft.UI.Content.PopupWindowSiteBridge` 弹窗。
- 顶部剪切、复制、重命名、共享、删除操作栏。
- “打开方式”、属性、第三方扩展、“显示更多选项”。
- Explorer 详细信息和高亮选择均为目标 SIG 文件。
- Esc 取消后 `folder_popup=Ok(())`、`selection_restored=Ok(())`；UI 再次显示未选择文件。

未执行实际文件命令，未验证多选、子目录、非活动标签页、隐藏窗口、Explorer 重启或多个 Windows 版本。不要将这些标为已通过。

## 接入建议

最可控的增量路线是“匹配的现有 Explorer 视图原生桥接 + 无匹配时保留经典 Shell 菜单”。但窗口前置是明确的体验代价，且行为随窗口是否已打开而不同；不能称为完整无感支持。

若要求不依赖用户打开目录且不出现 Explorer 窗口，则还需要独立的 Explorer 进程内宿主研究。隐藏普通 Explorer 窗口、借用非活动标签页和私有 presenter 均未验证成功，不建议现在直接放入生产。自行绘制 WinUI 菜单虽可控，但不符合本次原生精简菜单要求。

## 复现

```powershell
cargo build -p desktop-shell --example folder_menu_bridge_probe --offline
# 默认只查询接口，不弹出菜单。
target/debug/examples/folder_menu_bridge_probe.exe E:\Project\LucidPane\README.md
# 先在 Explorer 打开 Downloads。只观察并取消，不选择文件命令。
target/debug/examples/folder_menu_bridge_probe.exe C:\Users\Yuchen\Downloads\Maskit.app.tar.gz.sig --folder-view --show
```

## 官方接口资料

- [IExplorerBrowser](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-iexplorerbrowser)：嵌入 Shell 浏览视图；不承诺第三方宿主使用 Win11 精简菜单。
- [IContextMenuSite::DoContextMenuPopup](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-icontextmenusite-docontextmenupopup)：文档标记此接口不再可用，本机桥接成功不是跨版本兼容承诺。
- [扩展 Win11 文件菜单](https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/integrate-packaged-app-with-file-explorer)：IExplorerCommand 和应用身份用于向 Explorer 添加命令，不能据此推断提供了完整菜单宿主 API。
