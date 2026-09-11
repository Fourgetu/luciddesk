# Canvas drawing boundary

The hybrid application's drawing layer uses `windows_canvas::Rect`, `ColorF`,
`RoundedRect`, `Ellipse`, `Vector2`, `Brush`, `TextFormat`, `TextLayout`, `Bitmap`
and the Canvas device context directly. The former Format/Brush holders and
per-primitive forwarding/conversion methods have been removed. Text alignment,
wrapping, shaping, metrics and layout drawing use Canvas. Settings controls use
a separate centered format instead of mutating the label format during paint.

`canvas::DrawPass` is a lifetime/error guard that dereferences to the actual
Canvas DrawingSession. It does not reimplement or forward drawing primitives.
Native operations retained because Canvas 0.100 does not expose the required
behavior:

- Axis-aligned clipping, including balanced cleanup after a failed draw.
- Explicit grayscale text antialiasing on transparent composition targets.
- BeginDraw/EndDraw with all errors propagated; the library session destructor
  records device loss but does not return other EndDraw failures.
- Ellipsis trimming on a Canvas text format through its raw interface.
- Offscreen target binding around readback, for the explicit draw/error path.
- DWM/DirectComposition attachment and efficient packed-pixel upload to the
  swap-chain buffer. D3D staging readback is test-only.

The old ID2D1RenderTarget parameter is gone from painters. The native binding
conversion is confined to the interop/lifetime boundaries; ordinary geometry,
brush creation and text drawing do not translate between duplicate types.
No Reactor or Windows App SDK dependency was added. Legacy application sources
remain independent and were not changed by this migration.

Validation: full hybrid regression suite, fractional-DPI clipping, transparent
text antialiasing, failed-draw cleanup, live composition/readback and settings
rendering/interaction tests. One interactive tray test remains ignored.
