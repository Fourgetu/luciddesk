//! Windows Shell enumeration, file operations, and UI integration.
mod activation;
mod apartment;
mod desktop;
mod error;
mod file_command;
mod namespace;
mod native_layout;
mod native_menu;
mod menu_theme;
mod menu_frame;
mod notification;
mod rename;

pub use activation::{drag_shell_identities, open_shell_identity};
pub use apartment::ShellApartment;
pub use desktop::desktop_icons_hidden;
pub use desktop_core::ShellIdentity;
pub use error::ShellError;
pub use file_command::{
    FileCommand, copy_to_folder, drag_file_items, invoke_file_commands,
    paste_into_folder, show_file_items_menu,
};
pub use namespace::{
    DesktopShellItem, ShellAttributes, enumerate_desktop_namespace, enumerate_folder,
    local_app_data_path,
};
pub use native_layout::{
    NativeDesktopReader, NativeDesktopRevision, NativeDesktopSnapshot, native_desktop_snapshot,
};
pub use native_menu::{
    MenuInvocation, peek_desktop_item, show_desktop_item_menu, show_desktop_items_menu,
};
pub use notification::DesktopChangeSubscription;
pub use rename::rename_shell_identity;

#[cfg(test)]
mod tests;
