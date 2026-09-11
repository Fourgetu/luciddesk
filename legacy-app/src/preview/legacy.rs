//! Optional standalone preview entry point.
use super::*;
use desktop_shell::enumerate_desktop_references;

#[allow(clippy::too_many_lines)]
fn inventory(folder: Option<&Path>) -> Result<Vec<DesktopItem>, String> {
    // All reads and image extraction are independent of the native desktop's layout settings.
    let inventory = if let Some(folder) = folder {
        if !folder.is_dir() {
            return Err(format!("目录不存在：{}", folder.display()));
        }
        std::fs::read_dir(folder)
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .map(|entry| {
                let path = entry.path();
                let label = entry.file_name().to_string_lossy().into_owned();
                DesktopItem::new(
                    ShellIdentity::FileSystem {
                        path,
                        volume_id: None,
                        file_id: None,
                    },
                    label,
                )
            })
            .collect()
    } else {
        enumerate_desktop_references(0)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|item| DesktopItem::new(item.identity, item.display_name))
            .collect()
    };
    Ok(inventory)
}

pub fn run(path: &Path, folder: Option<&Path>, title: Option<String>) -> Result<(), String> {
    // Explicit folder previews are independent from the saved Desktop preview inventory.
    let owned_path;
    let path = if folder.is_some() {
        owned_path = path.with_file_name("folder-preview.db");
        &owned_path
    } else {
        path
    };
    let mut store = WorkspaceStore::open(path).map_err(|e| e.to_string())?;
    let mut workspace = store.load_workspace().map_err(|e| e.to_string())?;
    if workspace.panels().is_empty() {
        for (id, title, x) in [(1, "桌面项目 · 独立预览", 160.0), (2, "新建分组", 700.0)]
        {
            workspace
                .add_panel(Panel::new(
                    PanelId::new(id),
                    title,
                    PanelSource::DesktopCollection,
                    RectDip::new(x, 160.0, 480.0, 400.0),
                ))
                .map_err(|e| e.to_string())?;
            workspace
                .panel_mut(PanelId::new(id))
                .unwrap()
                .set_backdrop(desktop_core::Backdrop::Acrylic);
        }
    }
    if let Some(title) = title {
        workspace
            .panel_mut(PanelId::new(1))
            .unwrap()
            .set_title(title);
    }
    let valid: Vec<_> = workspace.panels().iter().map(Panel::id).collect();
    store
        .save_workspace(&workspace)
        .map_err(|e| e.to_string())?;
    let (sender, receiver) = mpsc::channel();
    let folder = folder.map(Path::to_path_buf);
    std::thread::spawn(move || {
        let _apartment = match ShellApartment::initialize_sta() {
            Ok(apartment) => apartment,
            Err(error) => {
                let _ = sender.send(Loaded::Inventory(Err(error.to_string())));
                return;
            }
        };
        let result = inventory(folder.as_deref());
        let requests: Vec<_> = result
            .as_ref()
            .map(|items| items.iter().map(|item| item.identity().clone()).collect())
            .unwrap_or_default();
        if sender.send(Loaded::Inventory(result)).is_err() {
            return;
        }
        for identity in requests {
            if let Ok(image) = assets::load(&identity, 96)
                && sender
                    .send(Loaded::Image(identity.persistent_key(), image))
                    .is_err()
            {
                break;
            }
        }
    });
    let state = Rc::new(RefCell::new(Preview {
        settings: None,
        workspace,
        store,
        views: Vec::new(),
        images: HashMap::new(),
        receiver,
        desktop: None,
    }));
    for id in valid {
        create_view(&state, id)?;
    }
    windows_window::run();
    Ok(())
}
