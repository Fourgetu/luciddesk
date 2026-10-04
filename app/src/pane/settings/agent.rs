//! Offline installation instructions; copying never executes a command.
pub(super) fn installation_prompt() -> Result<String, String> {
    let skill = std::env::current_exe().map_err(|e| e.to_string())?
        .with_file_name("skills").join("luciddesk-control");
    // JSON quoting keeps spaces, quotes and backslashes unambiguous in prose.
    let path = serde_json::to_string(&skill.to_string_lossy()).map_err(|e| e.to_string())?;
    Ok(crate::i18n::format("agent-install-prompt", &[("path", path)]))
}

#[cfg(test)]
mod tests {
    #[test]
    fn prompt_points_to_the_bundled_skill_directory() {
        let prompt = super::installation_prompt().unwrap();
        let path = std::env::current_exe().unwrap().with_file_name("skills").join("luciddesk-control");
        assert!(prompt.contains(&serde_json::to_string(&path.to_string_lossy()).unwrap()));
        assert!(prompt.contains("SKILL.md"));
        assert!(prompt.contains("references/"));
        assert!(!prompt.contains("skill show"));
    }
}
