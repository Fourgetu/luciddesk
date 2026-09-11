use std::path::PathBuf;
fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let check = std::env::args().any(|arg| arg == "--check");
    let output = root.join("crates/desktop-graphics/src/bindings");
    let temporary = root.join("target/bindings-check");
    std::fs::create_dir_all(&output).unwrap();
    std::fs::create_dir_all(&temporary).unwrap();
    for (name, style) in [("dwm", "--sys"), ("dcomp", "--minimal")] {
        let path = temporary.join(format!("{name}.rs"));
        let filters = std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("{name}.txt")),
        )
        .unwrap();
        let mut args = vec!["--out", path.to_str().unwrap(), "--flat", style, "--filter"];
        args.extend(filters.lines().filter(|line| !line.is_empty()));
        windows_bindgen::bindgen(args);
        let generated = std::fs::read(&path).unwrap();
        let destination = output.join(format!("{name}.rs"));
        if check {
            assert_eq!(
                std::fs::read(&destination).unwrap(),
                generated,
                "stale {name} bindings"
            );
        } else {
            std::fs::write(&destination, &generated).unwrap();
        }
        println!(
            "{name}: {} bytes ({})",
            generated.len(),
            if check { "verified" } else { "generated" }
        );
    }
}
