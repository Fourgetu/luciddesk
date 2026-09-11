# Pane icon refresh validation (2026-09-11)

## Follow-up: C-drive stale namespace bitmap

The earlier E-drive test below did **not** establish correctness for every drive.
The user's follow-up reproduced an APK in the C-drive Recycle Bin while Pane
still showed an empty icon. With normal desktop permissions, a fresh process
reported one item / 51,140,131 bytes via `SHQueryRecycleBinW`, but a fresh
`IShellItemImageFactory::GetImage` still returned empty artwork. Notifications
were delivered and targeted the correct identity; extracting again was insufficient.

Recycle Bin extraction now queries the total item count across drives and uses
`SHGetStockIconInfo` for the explicit empty/full state, then extracts the resource
at 128 pixels. This uses Windows stock artwork rather than the namespace image
factory's cached dynamic bitmap. It runs on the existing notification worker;
there is no added timer polling or reload of unrelated icons. Failed queries
propagate as errors rather than deliberately selecting the empty state.

References: [item-count API](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shqueryrecyclebinw),
[stock icon API](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shgetstockiconinfo).

Verified the user's existing C-drive item now produces full artwork; startup
uploads that image to the GPU (`ca04536a3a78cc65`). Adding a unique zero-byte
C-drive temporary file changed count 1 -> 2 without changing byte count, and
restoring only that temporary item changed count 2 -> 1. Both notifications
targeted only Recycle Bin, and correctly kept full artwork without GPU replacement.
The user's APK was untouched; the owned temporary file was verified and removed.
Logs: `target/recycle-stale-factory.log`, `target/recycle-state-fixed.log`.

Application suite after this fix: 64 passed, 3 ignored. A new regression verifies
distinct transparent 128-pixel artwork for both explicit states; existing GPU
readback tests verify same-identity image replacement. Empty/full live transitions
were not repeated in this follow-up because the user's existing recycled file
was preserved. This follow-up supersedes the earlier broad correctness conclusion.

## Earlier E-drive validation

Validated on the local Windows desktop with `--hybrid-desktop` and the matching
desktop Hook DLL. Recycle Bin was empty before and after testing. Only disposable
files created under this repository's `target` directory were deleted/restored.

## Changes covered

- Startup requests only Pane icons, loads them on up to four STA workers, then
  publishes the completed batch together. Shell refresh waits for initial batches
  so that a late startup result cannot overwrite a newer icon.
- Subscribe directly to Recycle Bin namespace notifications, including changes
  made outside Pane menus. Debounce notifications and refresh only affected Pane
  identities. Replace cached images only when their pixels change.
- Optional `LUCIDPANE_ICON_TRACE` records notification targets, source pixel hashes,
  startup batch duration, and Recycle Bin GPU bitmap uploads.

## Live results

| Action | Verified result |
| --- | --- |
| Start with 11 Pane icons | One batch, 11 loaded, 185 ms (this local run) |
| Delete two test files into Recycle Bin | Empty to full; cache and GPU upload changed |
| Restore All with exactly those two test files | Full to empty; both original files restored and contents verified |
| Delete a test file, then empty Recycle Bin | Full to empty; cache and GPU upload changed |
| Repeated Shell notifications | Identical pixels did not replace the cached image |
| Unrelated Shell changes | No matching Pane icon targets were loaded |

Source pixel fingerprints matched GPU upload diagnostics:
empty `83f8ec6df4627198`, full `c91ca1a91b5852b1`; rendered icon size 72 x 72 pixels.
The application remained running and responsive during these operations.
Evidence is in local ignored files `target/validation-first.log` and
`target/validation-render.log`.

## Automated checks

- Application suite: 63 passed, 3 existing manual/environment-dependent checks ignored.
- Shell library suite: 6 passed with normal desktop permissions. Sandboxed access
  to the existing live Explorer COM test returned access denied; the same test
  passed outside that sandbox. The runner remained alive after printing its
  successful summary and was terminated after verification; clean test-runner
  teardown is not established by this run. Application shutdown/restart succeeded.
- Extended the GPU readback regression to replace image pixels twice while keeping
  the Shell identity unchanged; framebuffer pixels changed correctly both times.

Live evidence verifies Shell delivery, targeted extraction, cache replacement and
GPU bitmap upload. A desktop screenshot was not obtained because computer-use
approval timed out. Automated GPU framebuffer readback checks the rendered pixels.
Startup timing is one local measurement, not a bound for every Shell extension.
