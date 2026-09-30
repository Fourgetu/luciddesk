# 应用图标

`luciddesk.ico` 使用 [三色绿色原稿](../../docs/design/luciddesk-green-simple-v17.png)：左侧翡翠绿、右上薄荷绿、右下深松绿。保留曲面、圆角、透明背景和三面板布局。

## 尺寸与导出

按 [Microsoft 应用图标构建规范](https://learn.microsoft.com/en-us/windows/apps/design/iconography/app-icon-construction)提供多尺寸资源。Win32 窗口与 EXE 使用 ICO；MSIX 打包脚本从同一图标生成清单所需的 PNG 资源。

ICO 包含 16、20、24、30、32、36、40、48、60、64、72、80、96、128、256 px 共 15 个尺寸，覆盖系统托盘、标题栏、任务栏和开始菜单的常见缩放比例。关于页读取 256 px PNG 图层。

导出时以 25% Alpha 阈值测量主体边界，仅用于裁去多余透明留白，不修改原稿；将主体居中放入正方形画布，最长边占 87.5%。此留白比例是本项目的视觉选择，不是微软强制要求。保留原稿主体宽高比例，不拉伸为正方形。

在仓库根目录执行 `./tools/generate-app-icon.ps1`（需要 ImageMagick 7），同时生成 ICO 和 [README PNG](../../docs/images/app-icon.png)。导出资源纳入版本管理，正常 Cargo 构建无需 ImageMagick。

## Shell 透明度兼容

导出采用预乘 Alpha 的分级缩放，16–64 px 最后一级使用高质量双线性过滤，其余使用高质量双三次过滤。16–128 px 使用 32 位 DIB 和 AND mask，RGB 按 Alpha 预乘，以兼容 Explorer 详情面板绘制路径；256 px PNG 和文档 PNG 保留常规 Alpha。此兼容处理可能让其他绘制路径的半透明边缘略暗，需在不同 Windows/DPI 下继续验证。

关于页使用 256 px PNG 帧，避免使用经过 Shell 兼容调整的小图层。

## 构建验证与旧图标

`tools/verify-app-icon.ps1 -Executable <EXE路径>` 将嵌入的 15 个图层逐字节与当前 ICO 比较；打包时自动检查，不一致即中止。

EXE 资源正确但 Explorer 显示旧图标时，可运行 `tools/Refresh-App-Icon.ps1 -Executable <EXE路径>`。发布包内可直接运行 `Refresh-App-Icon.ps1`。脚本只通知该文件及所在目录发生变化，不删除全局缓存、不重启 Explorer；系统仍可能延迟刷新，可用新目录中的发布包排除旧路径缓存。旧程序文件本身不会因刷新而变为新版。

## 程序内使用

`app.rc` 将 ICO 以资源 ID 1 嵌入 EXE。窗口、托盘和关于页都从该资源加载，无需读取外部图片。中英文 README 和功能示意图使用同源 PNG。

验证入口：`cargo test -p luciddesk app_icon::tests`，检查各尺寸加载和窗口图标设置。
