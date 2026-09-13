fn main() {
    let root = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .parent()
        .unwrap()
        .to_path_buf();
    for path in [
        "app/src",
        "app/assets",
        "app/build.rs",
        "app/Cargo.toml",
        "Cargo.toml",
        "Cargo.lock",
        "README.md",
        "CHANGELOG.md",
        "tools",
        ".github",
        "crates",
        "docs",
        ".git/HEAD",
        ".git/refs",
        ".git/index",
    ] {
        println!("cargo:rerun-if-changed={}", root.join(path).display());
    }
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(&root)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
    };
    println!("cargo:rerun-if-env-changed=LUCIDPANE_BUILD_REVISION");
    let revision = git(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown".into());
    let dirty = git(&["status", "--porcelain", "--untracked-files=no"])
        .is_some_and(|status| !status.is_empty());
    let revision = std::env::var("LUCIDPANE_BUILD_REVISION")
        .unwrap_or_else(|_| format!("{revision}{}", if dirty { "-dirty" } else { "" }));
    println!("cargo:rustc-env=LUCIDPANE_BUILD_REVISION={revision}");
    embed_resource::compile("assets/app.rc", embed_resource::NONE)
        .manifest_required()
        .expect("failed to embed the LucidPane application icon");
    // Native search controls and the backdrop fixture use Explorer's v6 controls.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        let manifest = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap())
            .join("../crates/desktop-hook/native-controls.manifest");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rustc-link-arg-bin=lucidpane=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-bin=lucidpane=/MANIFESTINPUT:{}",
            manifest.display()
        );
        println!("cargo:rustc-link-arg-examples=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-examples=/MANIFESTINPUT:{}",
            manifest.display()
        );
    }
}
