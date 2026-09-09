//! Read-only QI of the desktop view, or explicit restoration of the user's auto-arrange flag.
use desktop_shell::ShellApartment;
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL, IServiceProvider};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Shell::{IShellWindows, ShellWindows, CSIDL_DESKTOP, SWC_DESKTOP, SWFO_NEEDDISPATCH, IShellBrowser, SID_STopLevelBrowser, IFolderView2, FWF_AUTOARRANGE};
use windows::core::{Interface, GUID};

fn main()->windows::core::Result<()> {
    let _sta=ShellApartment::initialize_sta().unwrap();
    unsafe {
        let shell:IShellWindows=CoCreateInstance(&ShellWindows,None,CLSCTX_ALL)?;
        let mut hwnd=0;
        let dispatch=shell.FindWindowSW(&VARIANT::from(CSIDL_DESKTOP.cast_signed()),&VARIANT::default(),SWC_DESKTOP,&raw mut hwnd,SWFO_NEEDDISPATCH)?;
        let provider:IServiceProvider=dispatch.cast()?;
        let browser:IShellBrowser=provider.QueryService(&SID_STopLevelBrowser)?;
        let view=browser.QueryActiveShellView()?;
        let folder:IFolderView2=view.cast()?;
        let before=folder.GetCurrentFolderFlags()?;
        if std::env::args().any(|a|a=="--restore-auto-arrange") {
            folder.SetCurrentFolderFlags(FWF_AUTOARRANGE.0 as u32,FWF_AUTOARRANGE.0 as u32)?;
            let after=folder.GetCurrentFolderFlags()?;
            println!("Restored requested auto-arrange: before={before:08x}, after={after:08x}");
            assert!(after & FWF_AUTOARRANGE.0 as u32 != 0);
        }
        let iid=GUID::from_u128(0x44c09d56_8d3b_419d_a462_7b956b105b47);
        let mut callback=std::ptr::null_mut();
        let hr=view.query(&iid,&raw mut callback);
        println!("IOwnerDataCallback QI={hr:?}; available={}",!callback.is_null());
        if !callback.is_null(){let owned=windows::core::IUnknown::from_raw(callback);drop(owned);}
        Ok(())
    }
}
