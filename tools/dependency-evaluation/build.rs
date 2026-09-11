fn main() {
    let out = std::env::var("OUT_DIR").unwrap();
    let started = std::time::Instant::now();
    windows_bindgen::bindgen([
        "--out",
        &format!("{out}/com.rs"),
        "--flat",
        "--sys",
        "--filter",
        "CoInitializeEx",
        "CoUninitialize",
    ]);
    windows_bindgen::bindgen([
        "--out",
        &format!("{out}/system.rs"),
        "--flat",
        "--sys",
        "--filter",
        "DwmSetWindowAttribute",
        "SHQueryRecycleBinW",
    ]);
    windows_bindgen::bindgen([
        "--out",
        &format!("{out}/composition.rs"),
        "--flat",
        "--minimal",
        "--filter",
        "DCompositionCreateDevice",
        "IDCompositionDevice::{CreateAnimation,CreateEffectGroup,Commit}",
        "IDCompositionAnimation::{}",
        "IDCompositionEffectGroup::SetOpacity",
    ]);
    for name in ["system", "composition"] {
        let text = std::fs::read_to_string(format!("{out}/{name}.rs")).unwrap();
        println!(
            "cargo:warning={name} binding: {} bytes, {} lines",
            text.len(),
            text.lines().count()
        );
    }
    println!(
        "cargo:warning=binding generations took {} ms",
        started.elapsed().as_millis()
    );
    println!("cargo:rerun-if-changed=build.rs");
}
