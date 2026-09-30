# Changelog

[简体中文](CHANGELOG.md) · English

User-facing changes in each release. The current version is **0.14.0**. Windows 11 x64 is the primary validation platform.

Exit the app before upgrading a portable installation and preserve its `data` folder. See the [portable guide](docs/portable.md) (Chinese).

## 0.14.0 · 2026-09-30

### feat

- Use the current folder's classic Explorer background menu, including native commands and extensions.

### fix

- Follow the app language for folder type labels.
- Avoid reloading panes when dismissing file menus.

### docs

- Complete screenshots in both READMEs and show the illustration expanded and centered.

### ci

- Build and publish only on tag pushes; the `v` prefix is optional for version tags.

## 0.13.0 · 2026-09-30

- Add tray entries for file search, refreshing all panels and opening the configuration folder, with clearer menu grouping.
- Add a Windows 11 context-menu toggle for regular panels, enabled by default; apply system light/dark styling and DPI-aware spacing to classic fallback menus.
- Reorganize refresh, rename, view and creation commands in panel menus, and show collapse or expand according to the current state.
- Skip automatic backups when content is unchanged, prioritize manual actions and backup history, and unify back buttons on settings subpages.
- Introduce the green three-panel icon with 15 ICO sizes and an updated About icon; verify embedded resources during packaging and add actual screenshots to both READMEs.
- Remove startup DLL copying and load `luciddesk_desktop.dll` directly beside the executable. Extract the complete new package when upgrading to avoid mixing DLL versions.
- Adopt the MIT License with attribution to Yuchen95, and synchronize license information in About and release packages.
- Fix language changes incorrectly closing panels, and require an explicit close request before starting the close animation.
- Refresh language-dependent text and fonts in place, preserving panel items, selection, scroll positions and font search text without explicitly cancelling renames.
- Stop the font-loading timer when cancelling a previous language's task; retry language-dependent settings renderer refreshes and clear interaction state tied to the old layout.
- Forward the file menu wrapper's site interface to address the null host used while enumerating Open with commands, which caused Explorer crashes. Live compact-menu regression testing remains pending.

## 0.12.0 · 2026-09-30

- Add font-name search with multiple keywords, full-width Latin letters and digits, and common separators. Replace candidate previews with a continuously scrolling list that marks the current and default fonts.
- Enumerate fonts and check language coverage in the background on the first visit to the font page, keeping settings interactive. Reuse the list within the open settings window, release it on close, and cancel unfinished loading without a process-wide candidate cache.
- Improve the font search field's light and dark colors, placeholder contrast, focus feedback, and text cursor. Support a clear button and Escape; keep the current font and search field fixed above the scrolling list.
- Align settings control spacing and selection states, reduce temporary font enumeration resources, and validate only the selected family when saving instead of rescanning every candidate.
- Fix font handle cleanup when closing settings and release the text measurement cache, including its allocated capacity, with the settings renderer.

## 0.11.2 · 2026-09-30

- Simplify the Change folder menu wording and save the updated panel title when switching folders; reselecting the mapped root now leaves the current subfolder.
- Refine folder column sorting: modification time defaults to newest first and size to largest first, with name ordering for ties and unknown values last within their group.
- Reduce path allocations, unnecessary type sorting and temporary sort memory during folder icon updates, and improve selection matching when refreshing large selections.
- Reload previously loaded search pages before replacing results, restoring selection, focus and the range-selection anchor by path; prevent false double-clicks when results change.

## 0.11.1 · 2026-09-30

- Improve pending icons in desktop and folder panels with scalable file and folder outlines, aligned with loaded icons and adapted to light and dark themes.
- Keep placeholders independent of icon fonts and free of continuous animations; retain recognizable outlines if icon extraction fails.

## 0.11.0 · 2026-09-30

- Add an optional Show all panels global shortcut, disabled by default with `Ctrl + Shift + D` as the preset, with the same behavior as the tray action, customizable keys and conflict reporting.
- Switch interface languages immediately and follow changes to the effective Windows display language in system mode.
- Preview fonts in the current language, show the default fallback font, and refresh candidates when the language changes. Unify translucent accent states across settings materials and light/dark themes.
- Update About to use the public project website and add links to downloads, the changelog and issue reporting.
- Use “panel” consistently throughout the interface and both READMEs, preserving user-defined names.

## 0.10.6 · 2026-09-30

- Add Simplified Chinese, Traditional Chinese, English, Japanese, Korean, German and Russian interfaces. Follow the system language or choose one manually; changes take effect after restarting.
- Adapt default fonts to the selected language and improve the settings sidebar, descriptions and option layouts for longer text.
- Add an English README, bilingual feature illustrations and privacy policy, and bilingual changelogs.
- Fix compilation of the icon diagnostic example. CI now produces standard and portable ZIPs with SHA256 checksums and generates Release notes from the matching Chinese and English changelog sections.

## 0.10.5 · 2026-09-29

- Fix delays, failed drops and temporarily missing icons when moving batches of items into or out of desktop groups; reduce latency while waiting for desktop synchronization.
- Fix other panels unexpectedly covering the current panel when opening or closing menus, and reduce unnecessary stacking changes and flicker on clicks.
- Keep the search input at the same stacking level and topmost state as its panel. Clicking raises it only among desktop panels, and the search shortcut preserves desktop stacking.
- Improve background fallback when Windows disables Acrylic, Mica or Mica Alt, and improve sidebar selection and hover contrast in light and dark themes across all four materials.
- Improve panel and menu corners, icon font fallback and icon alignment on Windows 10; hide unsupported Mica and Mica Alt options.
- Fix a DXGI crash while releasing graphics caches during shutdown, and improve rendering logs and crash dump collection in optional diagnostic packages.

Windows 11 x64 is the primary maintenance platform. Windows 10 has been tested on a user's device; see the [validation notes](docs/development/validation.md) (Chinese) for coverage.

## 0.10.4 · 2026-09-29

- Update the underlying configuration reading and writing components.
- Reorganize the README and changelog to make startup, upgrades, backups and features easier to find.

## 0.10.3 · 2026-09-28

- Reduce duplicate memory use when multiple desktop groups and folder panels display the same icons.
- Improve icon cache memory management while preserving icon updates and refresh behavior.

Actual memory savings depend on the number of repeated icons.

## 0.10.2 · 2026-09-28

- Rename the tray entry to “Show panels” to make panels hidden behind other windows easier to find.
- Simplify the About page, bringing together the product version, developer, system information and desktop connection status.
- Improve executable file properties and portable package documentation.

## 0.10.1 · 2026-09-28

- Rename the product to **LucidDesk**, retaining compatibility with the previous data directory.
- Add portable mode to keep configuration and layouts beside the application.
- Raise all panels with a tray click without changing their topmost settings.
- Default the first group to 3 columns × 4 rows in the upper-right corner of the primary work area; existing layouts are unchanged.
- Use “New group” consistently for new groups and group tabs.

## 0.10.0 · 2026-09-28

- Add desktop group tabs with switching, reordering, renaming, closing and keyboard shortcuts.
- Support detaching tabs into separate panels and dragging panels together to merge tabs; folder and search panels remain independent.
- Add global font and title separator settings, and unify text, borders and menus across materials.
- Improve search input and results with clearing, full-path hints, result counts and error retries.
- Let folder panels open subfolders inside the panel or in File Explorer.
- Disable search and preview by default until enabled in Settings, while preserving existing explicit choices.
- Improve stacking, dragging and recovery after desktop disconnection; fix brief window flashes.
- Reduce repeated background checks and icon loading to improve idle resource use.
- Fall back from unavailable Mica effects to Acrylic or solid backgrounds while preserving the material selection.

## 0.9.1 · 2026-09-14

- Improve backup management, default folder columns and settings interactions.
- Scale icons, text, rename boxes and drag previews together; adjust grid scaling to 50%–200%.
- Reduce repeated memory allocations during folder browsing, search and drawing.
- Improve folder refresh, desktop dragging and layout saving.

## 0.9.0 · 2026-09-14

- Add a list view for desktop groups and adjustable icon grid sizes.
- Add a size column to folder lists with resizable, automatically saved column widths.
- Improve sorting by name, type and size, with natural ordering for numbers in file names.
- Add Back and Home buttons and fix missing folder contents in some cases.
- Prioritize thumbnails for visible files and reduce repeated waits when returning to visited folders.
- Adapt file menus to the system light or dark theme and add “Open file location”.

## 0.8.1 · 2026-09-13

- Fix empty desktop groups remaining on “Reading desktop items…”.
- Expand documentation for the three panel types, shortcuts, appearance and backups.

## 0.8.0 · 2026-09-13

- Store global settings separately from panel layouts; back up and restore both.
- Add the app icon to the About page and simplify version information.

Starting with this version, the old experimental `hook-desktop.db` database is no longer read. Automatic migration from that format is not provided.

## 0.7.1 · 2026-09-13

- Reduce repeated idle checks and folder refreshes to lower background resource use.
- Make panel borders respond to background opacity and material strength.
- Adjust the About page layout and fix clipped settings content.

## 0.7.0 · 2026-09-13

- Adapt panel text to the background automatically, with manual light or dark text options.
- Add an independent text backdrop protection option, disabled by default.
- Support fractional corner radii and fine adjustments with arrow keys.
- Adjust dark borders and settings page spacing.

## 0.6.1 · 2026-09-13

- Unify application, window and tray icons across display scaling levels.
- Add rounded corners, icons and app theme support to tray menus.
- Fix a possible crash when clicking the tray repeatedly.

## 0.6.0 · 2026-09-13

- Show the system version and allow copying diagnostic information on the About page.
- Restrict enabling search or preview when the required program is not detected.
- Improve panel collapsing, settings toggles and menu animations; fix occasional settings window flashes.
- Improve folder sorting and settings saving.

## 0.5.0 · 2026-09-13

- Allow choosing PowerToys Peek or QuickLook for file preview, with separate paths and shortcuts.
- Support QuickLook previews for files, folders and system items such as the Recycle Bin.
- Add search panel width adjustment, saved dimensions and edge snapping.
- Improve search result text, icons and context menu layouts.
- Simplify settings pages and allow resetting panel layout defaults.

## 0.4.2 · 2026-09-13

- Reorganize settings categories to make appearance, search, preview and backup options easier to find.
- Remove repeated explanations and clarify shortcut scope and error states.

## 0.4.1 · 2026-09-13

- Fix multi-selection drag previews showing only the item under the pointer; previews now show all selected items.
- Preserve relative positions of selected items and improve drag visuals across display scaling levels.

## 0.4.0 · 2026-09-13

- Add a color page with presets, RGB adjustments and live previews.
- Allow adjusting Acrylic and Mica strength and remember their settings independently.
- Use a fixed Mica Alt effect without a separate strength control.
- Configure panel color and opacity independently while retaining the default theme background in Settings.

## 0.3.0 · 2026-09-12

- Add solid backgrounds with custom colors, HEX input and 0–100% opacity.
- Keep text and icons clear while adjusting background opacity.
- Add live slider previews, keyboard fine-tuning and percentage input; remember the previous color when switching materials.

## 0.2.2 · 2026-09-12

- Restore the settings window fade-in animation and respect the Windows animation setting.
- Reduce flashes when opening windows.

## 0.2.1 · 2026-09-12

- Fix settings content and its background appearing out of sync.
- Improve rendering during material changes and window resizing.
- Improve the About page version details, feature introduction and project homepage link.

## 0.2.0 · Feature baseline, not released separately

- Desktop groups, tray menus, file renaming and keyboard multi-selection.
- Folder panels, directory navigation, list sorting and Everything search.
- File preview, configuration backups, monitor layout persistence and desktop connection recovery.
- Panel moving, resizing and edge snapping.
