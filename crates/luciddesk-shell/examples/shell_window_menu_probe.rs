//! Creates only a new owned Explorer window, then closes that exact instance.
use windows::{core::{Interface}, Win32::{System::{Com::{CoCreateInstance, CLSCTX_LOCAL_SERVER, IServiceProvider}, Variant::VARIANT}, UI::Shell::*}};
use windows_sys::Win32::UI::WindowsAndMessaging::*;
struct Browser(IWebBrowser2);
impl Drop for Browser { fn drop(&mut self) { unsafe { let _=self.0.Quit(); } } }
fn main() -> Result<(),Box<dyn std::error::Error>> {
 let _sta=luciddesk_shell::ShellApartment::initialize_sta()?;
 unsafe {
  let shell: IShellWindows = CoCreateInstance(&ShellWindows,None,CLSCTX_LOCAL_SERVER)?;
  let target=std::env::args().nth(1).ok_or("Expected exact probe folder")?;
  let wanted=target.replace('\\',"/").to_lowercase();
  let mut found=None;
  for index in 0..shell.Count()? {
   if let Ok(item)=shell.Item(&VARIANT::from(index)) {
    if let Ok(browser)=item.cast::<IWebBrowser2>() {
     if browser.LocationURL()?.to_string().replace("%20"," ").to_lowercase().ends_with(&wanted) {found=Some(browser);break;}
    }
   }
  }
  let browser=Browser(found.ok_or("Probe Explorer window not found")?);
  let root=browser.0.HWND()?.0 as windows_sys::Win32::Foundation::HWND;
  println!("owned_window={root:?}");
  SetWindowPos(root,std::ptr::null_mut(),GetSystemMetrics(SM_XVIRTUALSCREEN)+GetSystemMetrics(SM_CXVIRTUALSCREEN)+1000,0,640,480,SWP_NOACTIVATE|SWP_NOZORDER);
  let empty=VARIANT::default();
  browser.0.Navigate2(&VARIANT::from("shell:Desktop"),Some(&empty),Some(&empty),Some(&empty),Some(&empty))?;
  browser.0.SetVisible(false.into())?;
  let provider:IServiceProvider=browser.0.cast()?;
  let shell:IShellBrowser=provider.QueryService(&SID_STopLevelBrowser)?;
  let deadline=std::time::Instant::now()+std::time::Duration::from_secs(5);
  loop {
   if let Ok(view)=shell.QueryActiveShellView() {
    let folder:IFolderView2=view.cast()?;
    let count=folder.ItemCount(SVGIO_ALLVIEW)?;
    if count > 10 {
     println!("isolated_desktop_count={count} context_site={}",view.cast::<IContextMenuSite>().is_ok());
     if std::env::args().any(|arg| arg=="--menu") {
      folder.SelectItem(0,(SVSI_SELECT.0|SVSI_FOCUSED.0|SVSI_DESELECTOTHERS.0) as u32)?;
      ShowWindow(root,SW_SHOWNOACTIVATE);
      use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
      keybd_event(VK_MENU as u8,0,0,0);SetForegroundWindow(root);keybd_event(VK_MENU as u8,0,KEYEVENTF_KEYUP,0);
      std::thread::sleep(std::time::Duration::from_millis(200));
      view.UIActivate(SVUIA_ACTIVATE_FOCUS.0 as u32)?;
      let menu:IContextMenu=view.GetItemObject(SVGIO_SELECTION)?;
      let site:IContextMenuSite=view.cast()?;
      println!("probe_menu_view={:?}",view.GetWindow()?);
      site.DoContextMenuPopup(&menu,CMF_ITEMMENU|CMF_CANRENAME,windows::Win32::Foundation::POINT{x:600,y:450})?;
      let end=std::time::Instant::now()+std::time::Duration::from_secs(12);
      while std::time::Instant::now()<end {
       let mut msg=MSG::default();
       for _ in 0..32 { if PeekMessageW(&raw mut msg,std::ptr::null_mut(),0,0,PM_REMOVE)==0 {break;} TranslateMessage(&msg);DispatchMessageW(&msg); }
       std::thread::sleep(std::time::Duration::from_millis(10));
      }
     }
     break;
    }
   }
   if std::time::Instant::now()>deadline {break;}
   std::thread::sleep(std::time::Duration::from_millis(50));
  }
 }
 Ok(())
}
