# Private graphics bindings

This standalone tool generates the checked-in private bindings for `desktop-graphics`.
It is not an application build dependency. `dwm.txt` selects raw C APIs/constants;
`dcomp.txt` selects the exact COM methods required by the content layer.

From the repository root:

```powershell
cargo run --locked --offline --manifest-path tools/windows-bindings/Cargo.toml
cargo run --locked --offline --manifest-path tools/windows-bindings/Cargo.toml -- --check
```

Omit `--offline` when fetching dependencies for the first time. Commit the filter
files, tool lockfile, and generated output together. Do not edit files under
`crates/desktop-graphics/src/bindings` by hand. `--check` regenerates into
`target/bindings-check` and fails on a difference without replacing checked-in output.

The private crate has its own `windows-core` 0.100 dependency because generated
interface macros use crate-qualified core types; a module alias inside the app's
0.62 binding context is insufficient. Application adapters borrow COM references
across the boundary and translate HRESULT errors explicitly.
