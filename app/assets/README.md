# 应用图标

`lucidpane.ico` 来自用户选定的 [`lucidpane-icon-aligned-cyan-v16.png`](../../docs/design/lucidpane-icon-aligned-cyan-v16.png)，保留该图的构图和透明通道。导出时按可见主体裁去多余透明留白，居中放入方形画布，使主体最长边占 87.5%，再缩放并转换格式；源设计稿不变。

ICO 包含 16、20、24、32、40、48、64、96、128、256 像素资源。`app.rc` 将其以资源 ID 1 嵌入 EXE，窗口和托盘也从此资源加载，无需运行时读取外部图片。

修改源图后，在仓库根目录执行 `./tools/generate-app-icon.ps1`（需要 ImageMagick 7）。生成后的 ICO 纳入版本管理，正常 Cargo 构建无需 ImageMagick。
