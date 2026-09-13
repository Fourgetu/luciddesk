//! Compare Explorer's actual view order in a disposable fixture directory.
use windows::{core::{GUID, Interface}, Win32::{
    Foundation::PROPERTYKEY,
    System::{Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL, IServiceProvider}, Variant::VARIANT},
    UI::{Shell::*, WindowsAndMessaging::*},
}};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _sta = desktop_shell::ShellApartment::initialize_sta()?;
    let expected = std::env::args().nth(1).expect("fixture directory");
    assert!(expected.contains("lucid-sort-fixture"), "Use only a disposable fixture");
    unsafe {
        let shell: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL)?;
        for i in 0..shell.Count()? {
            let Ok(dispatch) = shell.Item(&VARIANT::from(i)) else { continue };
            let Ok(provider) = dispatch.cast::<IServiceProvider>() else { continue };
            let Ok(browser) = provider.QueryService::<IShellBrowser>(&SID_STopLevelBrowser) else { continue };
            let Ok(view) = browser.QueryActiveShellView() else { continue };
            let Ok(folder) = view.cast::<IFolderView2>() else { continue };
            let Ok(current) = folder.GetFolder::<IShellItem>() else { continue };
            let Ok(raw) = current.GetDisplayName(SIGDN_FILESYSPATH) else { continue };
            let path = raw.to_string()?;
            CoTaskMemFree(Some(raw.0.cast()));
            if !path.eq_ignore_ascii_case(&expected) { continue }
            folder.SetGroupBy(&PROPERTYKEY::default(), true)?;
            for (name, pid) in [("name", 10), ("type", 4), ("size", 12)] {
                for direction in [SORT_ASCENDING, SORT_DESCENDING] {
                    folder.SetSortColumns(&[SORTCOLUMN {
                        propkey: PROPERTYKEY { fmtid: GUID::from_u128(0xb725f130_47ef_101a_a5f1_02608c9eebac), pid }, direction,
                    }])?;
                    std::thread::sleep(std::time::Duration::from_millis(200));
                    let items: IShellItemArray = folder.Items(SVGIO_ALLVIEW | SVGIO_FLAG_VIEWORDER)?;
                    let mut names = Vec::new();
                    for index in 0..items.GetCount()? {
                        let raw = items.GetItemAt(index)?.GetDisplayName(SIGDN_PARENTRELATIVEPARSING)?;
                        names.push(raw.to_string()?);
                        CoTaskMemFree(Some(raw.0.cast()));
                    }
                    println!("{name} {}: {names:?}", direction.0);
                }
            }
            let _ = PostMessageW(Some(GetAncestor(view.GetWindow()?, GA_ROOT)), WM_CLOSE, Default::default(), Default::default());
            return Ok(());
        }
    }
    Err("fixture Explorer window not found".into())
}
