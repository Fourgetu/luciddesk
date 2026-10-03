//! Read the namespace identity of a stock ExplorerBrowser results folder.
use windows::{core::*, Win32::{Foundation::{HWND,RECT}, System::Com::*, UI::Shell::*}};
fn main() -> Result<()> {
    let _sta = luciddesk_shell::ShellApartment::initialize_sta().map_err(|e| Error::new(windows::Win32::Foundation::E_FAIL,e.to_string()))?;
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let hwnd=CreateWindowExW(0, windows_sys::w!("STATIC"), windows_sys::w!("Results identity probe"), WS_POPUP,0,0,100,100,std::ptr::null_mut(),std::ptr::null_mut(),std::ptr::null_mut(),std::ptr::null());
        let browser:IExplorerBrowser=CoCreateInstance(&ExplorerBrowser,None,CLSCTX_INPROC_SERVER)?;
        browser.Initialize(HWND(hwnd),&RECT{left:0,top:0,right:100,bottom:100},None)?;
        browser.FillFromObject(None::<&IUnknown>,EBF_NODROPTARGET)?;
        println!("filled=true");
        let view:IFolderView2=browser.GetCurrentView()?;
        let folder:IPersistFolder2=view.GetFolder()?;
        println!("persist=true");
        let pidl=folder.GetCurFolder()?;
        println!("pidl={:02x?}",std::slice::from_raw_parts(pidl.cast::<u8>(),ILGetSize(Some(pidl)) as usize));
        let name=SHGetNameFromIDList(pidl,SIGDN_DESKTOPABSOLUTEPARSING)?;
        println!("namespace={}",name.to_string()?);
        println!("pidl={:02x?}",std::slice::from_raw_parts(pidl.cast::<u8>(),ILGetSize(Some(pidl)) as usize));
        CoTaskMemFree(Some(name.0.cast())); CoTaskMemFree(Some(pidl.cast()));
        browser.Destroy()?; DestroyWindow(hwnd);
    }
    Ok(())
}
