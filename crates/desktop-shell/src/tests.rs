use crate::{ShellApartment, enumerate_desktop_namespace, namespace::stable_file_identity};
use std::collections::HashSet;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

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

#[test]
fn filesystem_identity_survives_a_rename() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("luciddesk-file-id-{unique}"));
    fs::create_dir_all(&root).unwrap();
    let before_path = root.join("before.txt");
    let after_path = root.join("after.txt");
    fs::write(&before_path, b"stable identity").unwrap();

    let before = stable_file_identity(&before_path).unwrap();
    fs::rename(&before_path, &after_path).unwrap();
    let after = stable_file_identity(&after_path).unwrap();

    assert_eq!(before, after);
    fs::remove_dir_all(root).unwrap();
}
