//! Offline installation instructions; copying never executes a command.
pub(super) fn installation_prompt() -> Result<String, String> {
    let cli = std::env::current_exe().map_err(|e| e.to_string())?.with_file_name("luciddesk-cli.exe");
    // JSON quoting keeps spaces, quotes and backslashes unambiguous in prose.
    let path = serde_json::to_string(&cli.to_string_lossy()).map_err(|e| e.to_string())?;
    Ok(crate::i18n::format("agent-install-prompt", &[("path", path)]))
}

#[cfg(test)]
mod tests {
    #[test]
    fn prompt_contains_the_matching_cli_and_offline_installation_steps() {
        let prompt = super::installation_prompt().unwrap();
        let path = std::env::current_exe().unwrap().with_file_name("luciddesk-cli.exe");
        assert!(prompt.contains(&serde_json::to_string(&path.to_string_lossy()).unwrap()));
        assert!(prompt.contains("luciddesk-control/SKILL.md"));
        assert!(prompt.contains("schema --json"));
    }
}
