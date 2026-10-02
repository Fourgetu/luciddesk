use crate::{ShellApartment, enumerate_desktop_namespace};
use std::collections::HashSet;

#[test]
fn desktop_namespace_provides_stable_shell_identities() {
    let _apartment = ShellApartment::initialize_sta().unwrap();
    let items = enumerate_desktop_namespace(0).unwrap();
    assert!(!items.is_empty());

    let keys: HashSet<_> = items
        .iter()
        .map(|item| item.identity.persistent_key())
        .collect();
    assert_eq!(keys.len(), items.len());
    assert!(items.iter().all(|item| !item.display_name.is_empty()));
}
