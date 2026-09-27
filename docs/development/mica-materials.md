# Mica and Mica Alt

## Native recipe verification

On 2026-09-28, queried Windows App Runtime 1.6.618 (CBS package
6000.900.156.100) using `MicaController` with a composition target attached,
`SystemBackdropConfiguration.Theme` explicitly set, and dispatcher messages pumped.
Without an attached target the theme-dependent getters can retain light defaults.
Values below are defaults at LucidPane material strength 50.

| Material | Theme | TintColor | TintOpacity | LuminosityOpacity | FallbackColor |
| --- | --- | --- | --- | --- | --- |
| Mica / Base | Light | #F3F3F3 | 0.5 | 1 | #F3F3F3 |
| Mica / Base | Dark | #202020 | 0.8 | 1 | #202020 |
| Mica Alt / BaseAlt | Light | #DADADA | 0.5 | 1 | #E8E8E8 |
| Mica Alt / BaseAlt | Dark | #0A0A0A | 0 | 1 | #202020 |

Tint colors and fallback colors are distinct. The XAML solid BaseAlt token
is #DADADA / #0A0A0A; it must not be substituted for the controller fallback.
These are measured version-specific defaults, not a promise that every Windows
App SDK version uses identical values. The diagnostic is kept in the ignored
`target/mica-probe` directory; no runtime DLLs are shipped with this change.

## Composition and layers

The GPU graph uses the blurred wallpaper backdrop, a luminosity blend, then a
color blend. The native graph notes that Direct2D Color/Luminosity mode names
are swapped. Both preview and live material consume the same palette and
strength adjustment. The preview uses a deterministic illustrative wallpaper
and CPU blending; it is not a pixel-identical DWM capture.

Desktop panes expose a continuous material across the title bar and body.
Do not apply full-body content or command overlays: stacking them obscures the
wallpaper and creates an unwanted gray slab. Layer tokens are retained only
for local UI surfaces such as selected tabs:

| Token | Light | Dark |
| --- | --- | --- |
| LayerFillColorDefault | #80FFFFFF | #4C3A3A3A |
| LayerOnMicaBaseAltFillColorDefault | #B3FFFFFF | #733A3A3A |

Local overlays remain separate from the backdrop strength.
The settings preview also omits full-body Mica overlays. Acrylic and solid backgrounds retain their
existing rendering. Native-material failure uses the measured opaque fallback.

## Sources

- [Microsoft Mica design guidance](https://learn.microsoft.com/en-us/windows/apps/design/style/mica)
- [WinUI theme resources](https://github.com/microsoft/microsoft-ui-xaml/blob/main/controls/dev/CommonStyles/Common_themeresources_any.xaml)
- [Public native blend graph](https://github.com/microsoft/microsoft-ui-xaml/blob/v2.8.0/dev/Materials/Backdrop/SystemBackdropBrushFactory.cpp)

## Verification

The material regression covers native palette values, blend mode/source order,
strength endpoints, and preview differences across themes. Settings render
snapshots cover light/dark and multiple DPI scales. Visual comparison of the
preview does not establish exact native desktop backdrop parity.

## Local material styling

`theme::material_chrome` owns active/inactive/incoming tab fills, panel edges,
settings cards and card edges. Acrylic uses lighter local fills and a clearer
edge, Mica uses quiet surfaces, and BaseAlt uses a stronger selected surface.
These are application UI choices, not additional native controller defaults.
Acrylic and Mica strengths interpolate local fills around the reference at 50;
minimum strength retains selection/card cues, while pane borders fade to zero.
BaseAlt remains fixed because it has no strength control. No full-pane body
surface is introduced. Settings keep their original base material strength and neutral solid background.
Only the companion card/tab/edge styles respond to the selected pane strength;
the native material recipe, opacity and strength mapping are unchanged.

## WinUI 3 pane strokes

Pane/search outlines and their preview use `SurfaceStrokeColorDefault`
(#66757575 in both themes). Internal separators use `DividerStrokeColorDefault`
(#15FFFFFF dark, #0F000000 light), from the linked official theme resources.
The custom renderer uses these WinUI colors; this is not a migration to XAML
windows or the DWM window frame. The second DWM outline remains disabled.
Line width stays one physical pixel. Existing configured corner radii remain.
For custom transparency, line opacity fades below the default material strength;
at and above the default the official ARGB values are used without amplification.
No wallpaper sampling, material-specific hue, or custom-color tint is applied.
Native backdrop recipes and card/tab fills are unchanged by this border change.
