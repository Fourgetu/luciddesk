# LucidPane 蓝青对齐生成稿 v16

已选为应用图标。运行资源为 [`app/assets/lucidpane.ico`](../../app/assets/lucidpane.ico)，EXE、窗口与托盘共用；保留原始设计稿及提示词。ICO 导出去除多余透明留白，使主体最长边占图标画布的 87.5%，再等比缩放和转换格式，不修改本稿几何或配色。

用户要求左侧上下与右侧上下对齐、左右同宽、右侧两块为正方形且整体为正方形，保留明亮蓝青配色和透明底。

通过内置 imagegen 迭代后，用户明确选择保留当前生成稿，不切换脚本精确排版。最终文件：`lucidpane-icon-aligned-cyan-v16.png`。此前保留的三个版本不变。

## 验证与限制

- 画布 1254×1254，角点 alpha 为 0。
- 按 alpha >= 192 的主要连通区域测量：左侧 x=268、y=257、341×739；右上 x=646、y=257、340×350；右下 x=646、y=645、340×351。
- 顶部、底部已经对齐；两列宽度相差 1 像素。主体外接框约 718×739，右侧块和整体均仍有少量非正方形误差。这些是生成稿实际测量值，并非提示词的精确坐标承诺。

## 最终生成提示词

Generate an isolated PNG icon asset with a REAL TRANSPARENT BACKGROUND. Precision geometric design, exactly three separate flat rectangular panes: one tall rectangle at left, two exact equal squares stacked at right. CRITICAL: left and right columns have exactly the same width; left top aligns perfectly with top-right top; left bottom aligns perfectly with bottom-right bottom; horizontal gap equals vertical gap; the outer bounding box is a perfect square. Use exact normalized coordinates in 1000x1000 square canvas: LEFT x200 y200 w280 h600, UPPER RIGHT x520 y200 w280 h280, LOWER RIGHT x520 y520 w280 h280. These boxes INCLUDE thin bottom-edge thickness. Same 24px corner radius on all three, no perspective distortion. Color: left bright sky blue gradient #38BDF8 to #209FE8; top right light aqua #8CEDE8 to #55D9DC; lower right bright cyan #4DD9F0 to #23BDD9. Each has a 12px darker bottom-edge sliver contained within its bounding box, minimal Fluent layer depth. No other embellishments. Three separate panes with empty alpha-transparent gutters and exterior. No baseplate, no outer border, no white or black background, no pattern, no shadows cast into empty space, no text. Front-facing precise geometry with equal width columns and square whole silhouette, optically centered. All geometry is crisp, no texture or glare. Transparent cutout asset.
