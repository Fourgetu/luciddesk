#[allow(
    dead_code,
    non_snake_case,
    non_upper_case_globals,
    non_camel_case_types
)]
mod system {
    include!(concat!(env!("OUT_DIR"), "/system.rs"));
}
#[allow(
    dead_code,
    non_snake_case,
    non_upper_case_globals,
    non_camel_case_types
)]
mod composition {
    include!(concat!(env!("OUT_DIR"), "/composition.rs"));
}
#[allow(
    dead_code,
    non_snake_case,
    non_upper_case_globals,
    non_camel_case_types
)]
mod com {
    include!(concat!(env!("OUT_DIR"), "/com.rs"));
}

struct Sta;
impl Drop for Sta {
    fn drop(&mut self) {
        unsafe {
            com::CoUninitialize();
        }
    }
}

fn main() -> windows_core::Result<()> {
    let sta = std::env::args().any(|arg| arg == "--sta");
    let _apartment = if sta {
        windows_core::HRESULT(unsafe { com::CoInitializeEx(core::ptr::null(), 2) }).ok()?;
        Some(Sta)
    } else {
        windows_core::init_mta()?;
        None
    };
    println!("apartment={}", if sta { "STA" } else { "MTA" });
    let manager = windows_animation::Manager::new()?;
    let library = windows_animation::TransitionLibrary::new()?;
    let height = manager.create_variable(400.0)?;
    manager.schedule_transition(
        &height,
        &library.accelerate_decelerate(0.2, 38.0, 0.0, 1.0)?,
        0.0,
    )?;
    manager.update(0.08)?;
    let mid = height.value()?;
    assert!(mid > 38.0 && mid < 400.0);
    println!("fold midpoint={mid:.3}");
    manager.schedule_transition(
        &height,
        &library.accelerate_decelerate(0.2, 400.0, 0.0, 1.0)?,
        0.08,
    )?;
    manager.update(0.08)?;
    let reverse = height.value()?;
    assert!((reverse - mid).abs() < 0.001);
    manager.update(0.5)?;
    assert!((height.value()? - 400.0).abs() < 0.001);
    println!("interrupted fold stays continuous; delayed tick reaches endpoint");
    use windows_core::Interface;
    let fade = manager.create_variable(0.0)?;
    manager.schedule_transition(&fade, &library.linear(0.12, 1.0)?, 0.5)?;
    manager.update(0.5)?;
    unsafe {
        let mut raw = core::ptr::null_mut();
        composition::DCompositionCreateDevice(
            core::ptr::null_mut(),
            &composition::IDCompositionDevice::IID,
            &mut raw,
        )
        .ok()?;
        let device = composition::IDCompositionDevice::from_raw(raw);
        let curve = device.CreateAnimation()?;
        fade.copy_curve(&curve)?;
        let effect = device.CreateEffectGroup()?;
        effect.SetOpacity(&curve).ok()?;
        device.Commit().ok()?;
    }
    manager.update(0.56)?;
    assert!((fade.value()? - 0.5).abs() < 0.001);
    manager.update(0.9)?;
    assert!((fade.value()? - 1.0).abs() < 0.001);
    println!(
        "fade curve copied, assigned to DComp effect, and committed through generated bindings"
    );
    Ok(())
}
