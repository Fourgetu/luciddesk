//! Reversible live geometry validation. Does not use native work-area or position-write messages.
use desktop_hook::{HookSession, desktop_view, conflicting_desktop_extension, protocol::*};
use desktop_shell::{ShellApartment, native_desktop_snapshot};
use windows_sys::Win32::Foundation::POINT;
use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
use windows_sys::Win32::UI::WindowsAndMessaging::*;
use std::ptr::{null,null_mut};

fn icon_rect(session: &HookSession, item: i32) -> Result<[i32;4],String> {
    let mut output=[0;4];
    for (i,value) in output.iter_mut().enumerate() {
        let mut request=Request::new(QUERY_ICON_RECT); request.item=item;request.x=i as i32;
        *value=session.request(&request)? as i32;
    }
    Ok(output)
}
fn main() -> Result<(),String> {
    let _sta=ShellApartment::initialize_sta().map_err(|e|e.to_string())?;
    if conflicting_desktop_extension() { return Err("Fences is running; native geometry probe aborted".into()); }
    if desktop_shell::desktop_icons_hidden() { return Err("Native desktop icons are hidden".into()); }
    let before=native_desktop_snapshot()?;
    let view=desktop_view()?;
    let base=std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().to_path_buf();
    let data=std::fs::read(base.join("desktop_hook.dll")).map_err(|e|e.to_string())?;
    let hash=data.iter().fold(0xcbf2_9ce4_8422_2325_u64,|h,b|(h^u64::from(*b)).wrapping_mul(0x100_0000_01b3));
    let folder=base.join("hook-runtime").join(format!("{hash:016x}"));
    std::fs::create_dir_all(&folder).map_err(|e|e.to_string())?;
    let dll=folder.join("desktop_hook.dll");if !dll.exists(){std::fs::write(&dll,data).map_err(|e|e.to_string())?;}
    let owner=unsafe { CreateWindowExW(0,windows_sys::w!("STATIC"),windows_sys::w!("LucidPane native geometry validation"),0,0,0,1,1,null_mut(),null_mut(),null_mut(),null()) };
    let outcome=(|| {
        let session=HookSession::connect_geometry(view,owner as isize,&dll)?;
        let auto=session.request(&Request::new(QUERY_AUTOARRANGE))?;
        if auto!=1 { return Err("Automatic arrangement must remain enabled for this test".into()); }
        let index=before.items.iter().position(|(i,_,_)|i.display_name.eq_ignore_ascii_case("Zed")).ok_or("Zed test item is absent; no item was moved")?;
        let item=before.view_indices[index];
        let baseline=icon_rect(&session,item)?;
        println!("Native geometry attached: items={} auto={auto} baseline={baseline:?}",before.items.len());
        if std::env::args().any(|a|a=="--connect-only") {
            drop(session);return Ok(());
        }
        let mut origin=POINT::default();unsafe { ClientToScreen(view as _,&raw mut origin); }
        let monitor=desktop_window::enumerate_monitors().into_iter().next().ok_or("No monitor")?;
        let pane=Area { left:monitor.work_area.x+monitor.work_area.width-420-origin.x,top:monitor.work_area.y+120-origin.y,right:monitor.work_area.x+monitor.work_area.width-40-origin.x,bottom:monitor.work_area.y+480-origin.y };
        session.set_areas(&[pane])?;
        session.move_item_named(item,pane.left+24,pane.top+24,&before.items[index].0.display_name)?;
        let moved=icon_rect(&session,item)?;
        let mut hit=Request::new(QUERY_HIT);hit.x=(moved[0]+moved[2])/2;hit.y=(moved[1]+moved[3])/2;
        let hit_index=session.request(&hit)?-1;
        assert_eq!(hit_index,item as isize,"Native hit-testing did not follow mapped icon");
        let during=native_desktop_snapshot()?;
        assert_eq!(during.items.len(),before.items.len());
        assert_eq!(session.request(&Request::new(QUERY_AUTOARRANGE))?,auto);
        println!("Native mapped icon={moved:?}, hit={hit_index}, auto={auto}; Shell position={},{}",during.items[index].1,during.items[index].2);
        std::thread::sleep(std::time::Duration::from_millis(800));
        drop(session);
        let after=native_desktop_snapshot()?;
        let unchanged=before.items.iter().all(|(old,x,y)|after.items.iter().any(|(new,nx,ny)|old.identity.equivalent_to(&new.identity)&&x==nx&&y==ny));
        assert!(unchanged,"Native positions differed after detach");
        let style=unsafe { GetWindowLongW(view as _,GWL_STYLE) };
        assert_ne!(style & windows_sys::Win32::UI::Controls::LVS_AUTOARRANGE as i32,0);
        println!("PASS: real native icon geometry/hit-test; {} items and every original position restored; automatic arrangement remains enabled",after.items.len());
        Ok(())
    })();
    unsafe { DestroyWindow(owner); }
    outcome
}
