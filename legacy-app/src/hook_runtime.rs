use std::path::{Path, PathBuf};

pub(crate) fn runtime_dll(database: &Path) -> Result<PathBuf, String> {
    let source = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .with_file_name("desktop_hook.dll");
    let bytes = std::fs::read(&source).map_err(|e| {
        format!(
            "缺少原生 Hook DLL {}。请构建整个 workspace 后启动：{e}",
            source.display()
        )
    })?;
    let hash = bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x100_0000_01b3)
    });
    let folder = database
        .parent()
        .ok_or("配置路径无效")?
        .join("hook-runtime")
        .join(format!("{hash:016x}"));
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    let target = folder.join("desktop_hook.dll");
    if !target.exists() {
        std::fs::write(&target, bytes).map_err(|e| e.to_string())?;
    }
    Ok(target)
}
