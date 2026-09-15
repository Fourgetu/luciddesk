//! A real Explorer browser hosting a separately populated Shell results folder.
//! All inputs are disposable workspace fixtures. Never changes desktop membership.
use std::{path::Path, time::{Duration, Instant}};
use windows::{core::{HSTRING, Interface}, Win32::{System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_INPROC_SERVER}, UI::Shell::*}};
use windows::Win32::{Storage::EnhancedStorage::PKEY_ItemPathDisplay, System::Search::{*, Common::*}};

fn pump() {
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let mut msg = MSG::default();
        while PeekMessageW(&raw mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

pub fn open_results(browser: &IShellBrowser, fixture: &Path) -> Result<IShellView, Box<dyn std::error::Error>> {
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let hwnd = CreateWindowExW(0, windows_sys::w!("STATIC"), windows_sys::w!("Results identity seed"), WS_POPUP,0,0,100,100,std::ptr::null_mut(),std::ptr::null_mut(),std::ptr::null_mut(),std::ptr::null());
        let seed:IExplorerBrowser = CoCreateInstance(&ExplorerBrowser,None,CLSCTX_INPROC_SERVER)?;
        seed.Initialize(windows::Win32::Foundation::HWND(hwnd),&windows::Win32::Foundation::RECT{left:0,top:0,right:100,bottom:100},None)?;
        seed.FillFromObject(None::<&windows::core::IUnknown>,EBF_NODROPTARGET)?;
        let source:IFolderView2=seed.GetCurrentView()?;
        let persist:IPersistFolder2=source.GetFolder()?;
        let pidl=persist.GetCurFolder()?;
        let result=browser.BrowseObject(pidl,SBSP_SAMEBROWSER|SBSP_ABSOLUTE);
        CoTaskMemFree(Some(pidl.cast()));
        result?;
        println!("results_navigation_requested=true");
        let deadline=Instant::now()+Duration::from_secs(12);
        let (view,folder,results)=loop {
            if let Ok(view)=browser.QueryActiveShellView() {
                if let Ok(folder)=view.cast::<IFolderView2>() {
                    if let Ok(results)=folder.GetFolder::<IResultsFolder>() { break (view,folder,results); }
                }
            }
            if Instant::now()>=deadline {return Err("Full Explorer cannot expose writable results collection".into());}
            pump();std::thread::sleep(Duration::from_millis(50));
        };
        seed.Destroy()?;DestroyWindow(hwnd);
        println!("results_interface=true");
        for (subfolder,name) in [("Source A","Pane alpha.txt"),("Source B","Pane beta.txt")] {
            let parent=fixture.join(subfolder);std::fs::create_dir_all(&parent)?;
            let path=parent.join(name);std::fs::write(&path,b"Disposable writable Shell collection.\r\n")?;
            let item:IShellItem=SHCreateItemFromParsingName(&HSTRING::from(path.to_string_lossy().as_ref()),None)?;
            results.AddItem(&item)?;
            println!("results_add={}",path.display());
        }
        let deadline=Instant::now()+Duration::from_secs(8);
        while folder.ItemCount(SVGIO_ALLVIEW)?!=2 {
            if Instant::now()>=deadline {return Err("Results collection did not populate".into());}
            pump();std::thread::sleep(Duration::from_millis(50));
        }
        folder.SetViewModeAndIconSize(FVM_ICON,48)?;
        println!("results_ready=true count=2");
        Ok(view)
    }
}

pub fn open(browser: &IShellBrowser, fixture: &Path) -> Result<IShellView, Box<dyn std::error::Error>> {
    unsafe {
        let conditions: IConditionFactory2 = CoCreateInstance(&ConditionFactory, None, CLSCTX_INPROC_SERVER)?;
        let mut leaves = Vec::new();
        let mut scopes = Vec::new();
        for (subfolder, name) in [("Source A", "Pane alpha.txt"), ("Source B", "Pane beta.txt")] {
            let parent = fixture.join(subfolder);
            std::fs::create_dir_all(&parent)?;
            let path = parent.join(name);
            std::fs::write(&path, b"Disposable Shell collection fixture.\r\n")?;
            std::fs::write(parent.join("Excluded.txt"), b"Must not appear in the Pane collection.\r\n")?;
            let parent_item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(parent.to_string_lossy().as_ref()), None)?;
            scopes.push(SHGetIDListFromObject(&parent_item)?);
            let leaf: ICondition = conditions.CreateStringLeaf(&PKEY_ItemPathDisplay, COP_EQUAL, &HSTRING::from(path.to_string_lossy().as_ref()), windows::core::PCWSTR::null(), CONDITION_CREATION_DEFAULT)?;
            leaves.push(Some(leaf));
            println!("collection_member={}", path.display());
        }
        let pointers: Vec<_> = scopes.iter().map(|p| p.cast_const()).collect();
        let scope = SHCreateShellItemArrayFromIDLists(&pointers);
        for pidl in scopes { CoTaskMemFree(Some(pidl.cast())); }
        let scope = scope?;
        let condition: ICondition = conditions.CreateCompoundFromArray(CT_OR_CONDITION, &leaves, CONDITION_CREATION_DEFAULT)?;
        let factory: ISearchFolderItemFactory = CoCreateInstance(&SearchFolderItemFactory, None, CLSCTX_INPROC_SERVER)?;
        factory.SetDisplayName(windows::core::w!("LucidPane collection probe"))?;
        factory.SetScope(&scope)?;
        factory.SetCondition(&condition)?;
        factory.SetFolderLogicalViewMode(FLVM_ICONS)?;
        factory.SetIconSize(48)?;
        let pidl = factory.GetIDList()?;
        let result = browser.BrowseObject(pidl, SBSP_SAMEBROWSER | SBSP_ABSOLUTE);
        CoTaskMemFree(Some(pidl.cast()));
        result?;
        println!("collection_navigation_requested=true");
        let deadline = Instant::now() + Duration::from_secs(12);
        let (view, folder) = loop {
            if let Ok(view) = browser.QueryActiveShellView() {
                if let Ok(folder) = view.cast::<IFolderView2>() {
                    if folder.ItemCount(SVGIO_ALLVIEW).unwrap_or(-1) == 2 {
                        break (view, folder);
                    }
                }
            }
            if Instant::now() >= deadline { return Err("Full Explorer did not enumerate the two exact collection members".into()); }
            pump();
            std::thread::sleep(Duration::from_millis(50));
        };
        println!("collection_results_interface={:?}", folder.GetFolder::<IResultsFolder>().map(|_| ()));
        folder.SetViewModeAndIconSize(FVM_ICON, 48)?;
        println!("collection_ready=true count=2 source_directories=2");
        Ok(view)
    }
}
