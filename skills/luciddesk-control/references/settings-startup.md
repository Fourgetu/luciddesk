## Settings and fonts

Read `settings get --json`. `values` holds loaded TOML configuration and `workspace_values` holds database preferences. Submit typed dotted fields in a `settings.update` operation's `values` map. It must be the sole operation, and must not mix the two persistence domains. Omitted fields are preserved; disk edits not yet reloaded are not reflected in this query.

Use only keys returned by `settings get`. Common TOML fields include `diagnostics.level` (`error`, `warn`, `info`, `debug`, `trace`), `search.enabled`, and `panel_defaults.snap`. `cli.enabled:false` intentionally cuts off subsequent online verification and receipt queries; change it only when the user requests disabling control. Verify the apply response and explain that re-enabling requires the Settings UI.

Verify saved values and runtime activation separately. For failed activation, inspect the error, resolve the dependency, then preview a fresh same-value update if activation should be retried.

Discover supported keys and value constraints through `settings get` and `help settings update --json`; do not treat a copied preference catalog as authoritative. Folder defaults affect newly created panels. Saving a backup policy does not imply a backup completed. For `font.family`, an empty string selects the language default.

Before choosing a font, query `font list --json` and boundedly wait for `busy:false` with no error. Choose an exact returned family and verify `runtime.font_family` after applying. Discovery is cached for 60 seconds and invalidated by language sample changes; retain generation when consistency matters.

## Login startup

Query `startup get` and boundedly wait until not busy and without error. Distinguish `status`, `registered`, and `effective_enabled`; require `editable:true` before changing OS login behavior within the user's requested scope.

Submit `startup.set` alone with `enabled:boolean` and the observed `expected_status`. The app checks current OS state before writing. Do not bypass Windows restrictions or another installation. Poll the original receipt until `operation_status` is completed/failed and inspect `commit_status`, `startup_status`, and error. Initial acceptance is not completion. Unknown outcomes require rechecking OS state before planning another action.
