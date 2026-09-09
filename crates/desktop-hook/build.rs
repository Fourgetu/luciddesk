fn main() {
    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("x86_64") {
        cc::Build::new()
            .include("vendor/minhook/include")
            .files([
                "vendor/minhook/src/buffer.c",
                "vendor/minhook/src/hook.c",
                "vendor/minhook/src/trampoline.c",
                "vendor/minhook/src/hde/hde64.c",
            ])
            .compile("lucidpane_minhook");
        println!("cargo:rerun-if-changed=vendor/minhook");
    }
    let manifest = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join("native-controls.manifest");
    println!("cargo:rerun-if-changed=native-controls.manifest");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        for kind in ["cdylib", "examples"] {
            println!("cargo:rustc-link-arg-{kind}=/MANIFEST:EMBED");
            println!(
                "cargo:rustc-link-arg-{kind}=/MANIFESTINPUT:{}",
                manifest.display()
            );
        }
    }
}
