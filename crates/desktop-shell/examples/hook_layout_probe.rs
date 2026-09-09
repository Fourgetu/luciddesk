//! A bounded live desktop work-area experiment. Never changes files or folder flags.
use desktop_hook::{HookSession, desktop_view, conflicting_desktop_extension, protocol::*};
use desktop_shell::{ShellApartment,native_desktop_snapshot};
use desktop_window::enumerate_monitors;
use std::ptr::{null,null_mut};
use windows_sys::Win32::Foundation::POINT;
use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

fn main() -> Result<(),String> {
    if std::env::args().any(|a|a=="--snapshot-only") {
        let _sta=ShellApartment::initialize_sta().map_err(|e|e.to_string())?;
        let view=desktop_view()?;
        let style=unsafe{GetWindowLongW(view as _,-16)};
        let extended=unsafe{SendMessageW(view as _,windows_sys::Win32::UI::Controls::LVM_GETEXTENDEDLISTVIEWSTYLE,0,0)};
        println!("Native extended ListView style={extended:08x}");
        println!("Native view={view}, style={style:08x}, ownerdata={}, auto={}",style&0x1000!=0,style&0x100!=0);
        let current=native_desktop_snapshot()?;
        let target=std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().join("hook-layout-recovered.txt");
        std::fs::write(target,format!("{current:#?}")).map_err(|e|e.to_string())?;
        println!("Snapshot only: items={}, style={style:08x}, ownerdata={}, auto={}",current.items.len(),style&0x1000!=0,style&0x100!=0);
        return Ok(());
    }
    if conflicting_desktop_extension() { return Err("Fences is still running".into()); }
    let _sta = ShellApartment::initialize_sta().map_err(|e|e.to_string())?;
    let before = native_desktop_snapshot()?;
    let view = desktop_view()?;
    let mut origin=POINT::default();unsafe{ClientToScreen(view as _,&raw mut origin);}
    let exe=std::env::current_exe().unwrap();let build=exe.parent().unwrap().parent().unwrap();
    std::fs::write(build.join("hook-layout-before.txt"),format!("{before:#?}")).map_err(|e|e.to_string())?;
    let data=std::fs::read(build.join("desktop_hook.dll")).map_err(|e|e.to_string())?;
    let hash=data.iter().fold(0xcbf2_9ce4_8422_2325_u64,|h,b|(h^u64::from(*b)).wrapping_mul(0x100_0000_01b3));
    let dir=build.join("hook-runtime").join(format!("{hash:016x}"));std::fs::create_dir_all(&dir).map_err(|e|e.to_string())?;
    let dll=dir.join("desktop_hook.dll");if !dll.exists(){std::fs::write(&dll,data).map_err(|e|e.to_string())?;}
    let owner=unsafe{CreateWindowExW(0,windows_sys::w!("STATIC"),windows_sys::w!("LucidPane bounded layout probe"),0,0,0,1,1,null_mut(),null_mut(),null_mut(),null())};
    let result=(||{
        let session=HookSession::connect(view,owner as isize,&dll)?;
        let initial_auto=session.request(&Request::new(QUERY_AUTOARRANGE))?;
        let count=session.request(&Request::new(QUERY_AREA_COUNT))?;
        let mut baseline=Vec::new();
        for i in 0..count {
            let mut q=Request::new(QUERY_BASELINE_COORD);q.item=i as i32;
            let mut coords=[0;4];for (j,c) in coords.iter_mut().enumerate(){q.x=j as i32;*c=session.request(&q)? as i32;}
            baseline.push(Area{left:coords[0],top:coords[1],right:coords[2],bottom:coords[3]});
        }
        let monitors:Vec<_>=enumerate_monitors().iter().map(|m|Area{left:m.work_area.x-origin.x,top:m.work_area.y-origin.y,right:m.work_area.x+m.work_area.width-origin.x,bottom:m.work_area.y+m.work_area.height-origin.y}).collect();
        let m=monitors[0];let pane=Area{left:m.right-450,top:m.top+80,right:m.right-30,bottom:(m.top+480).min(m.bottom)};
        let areas=partition(&monitors,&[pane])?;
        let experiment=(||{
            session.set_areas(&areas)?;
            let current=native_desktop_snapshot()?;
            let selected=current.items.iter().position(|(item,_,_)|item.display_name.eq_ignore_ascii_case("Zed")).unwrap_or(0);
            session.move_item_named(current.view_indices[selected],pane.left+10,pane.top+10,&current.items[selected].0.display_name)?;
            let mut q=Request::new(QUERY_ITEM_AREA);q.item=current.view_indices[selected];
            assert_eq!(session.request(&q)?,(areas.len()-1) as isize);
            unsafe{SendMessageW(view as _,windows_sys::Win32::UI::Controls::LVM_ARRANGE,0,0);}
            assert_eq!(session.request(&q)?,(areas.len()-1) as isize);
            assert_eq!(session.request(&Request::new(QUERY_AUTOARRANGE))?,initial_auto);
            println!("Native Explorer move/rearrange PASS; auto={initial_auto}, items={}, group_area={}",current.items.len(),areas.len()-1);
            Ok(())
        })();
        // Restore baseline and original membership before detaching; no auto-arrange toggle.
        if !baseline.is_empty(){
            session.set_areas(&baseline)?;
            let current=native_desktop_snapshot()?;
            for (old,x,y) in &before.items {
                if let Some(i)=current.items.iter().position(|(item,_,_)|item.identity.equivalent_to(&old.identity)){
                    session.move_item_named(current.view_indices[i],x-origin.x,y-origin.y,&current.items[i].0.display_name)?;
                }
            }
        }
        assert_eq!(session.request(&Request::new(QUERY_AUTOARRANGE))?,initial_auto);
        drop(session);
        let after=native_desktop_snapshot()?;
        std::fs::write(build.join("hook-layout-after.txt"),format!("{after:#?}")).map_err(|e|e.to_string())?;
        let moved=before.items.iter().filter(|(old,x,y)|after.items.iter().any(|(new,nx,ny)|new.identity.equivalent_to(&old.identity)&&(nx!=x||ny!=y))).count();
        println!("Restored: items={}, changed_positions={moved}, baseline_areas={count}",after.items.len());
        experiment
    })();
    unsafe{DestroyWindow(owner);}
    result
}
