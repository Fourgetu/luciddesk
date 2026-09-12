//! A transient attach failure must not poison this pinned DLL for Explorer's lifetime.
use std::sync::OnceLock;

// The caller holds ACTIVE while initializing, so native hooks are installed once.
pub(super) fn get(
    cache: &OnceLock<[usize; 5]>,
    install: impl FnOnce() -> Result<[usize; 5], String>,
) -> Result<&[usize; 5], String> {
    if let Some(targets) = cache.get() {
        return Ok(targets);
    }
    let targets = install()?;
    let _ = cache.set(targets);
    Ok(cache.get().expect("successful installation was cached"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn temporary_failure_can_retry_and_success_is_reused() {
        let cache = OnceLock::new();
        assert!(get(&cache, || Err("previous controller is detaching".into())).is_err());
        assert!(cache.get().is_none());
        assert_eq!(
            get(&cache, || Ok([1, 2, 3, 4, 5])).unwrap(),
            &[1, 2, 3, 4, 5]
        );
        assert_eq!(
            get(&cache, || panic!("must reuse installed trampolines")).unwrap(),
            &[1, 2, 3, 4, 5]
        );
    }
}
