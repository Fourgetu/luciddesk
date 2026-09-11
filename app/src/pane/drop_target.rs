//! OLE link-style collection drops. A successful drop never requests a source file move.
use desktop_core::ShellIdentity;
use std::{cell::RefCell, rc::Rc};
use windows::{
    Win32::{
        Foundation::{HWND, POINT, POINTL},
        System::{
            Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, IDataObject},
            Ole::*,
            SystemServices::MODIFIERKEYS_FLAGS,
        },
        UI::Shell::{CLSID_DragDropHelper, IDropTargetHelper},
    },
    core::{Ref, Result, implement},
};

type Accept = Rc<dyn Fn(&[ShellIdentity], bool) -> bool>;
#[implement(IDropTarget)]
struct Target {
    hwnd: HWND,
    helper: Option<IDropTargetHelper>,
    accept: Accept,
    items: RefCell<Vec<ShellIdentity>>,
}
impl IDropTarget_Impl for Target_Impl {
    fn DragEnter(
        &self,
        data: Ref<IDataObject>,
        _: MODIFIERKEYS_FLAGS,
        point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> Result<()> {
        let items = data
            .as_ref()
            .and_then(|d| desktop_shell::drag_shell_identities(d).ok())
            .unwrap_or_default();
        *self.items.borrow_mut() = items;
        self.update_effect(effect);
        if let (Some(helper), Some(data)) = (&self.helper, data.as_ref()) {
            // Explorer's drag image (including its label) needs an OLE helper on
            // every target it enters, even though collection drops only link.
            let point = POINT {
                x: point.x,
                y: point.y,
            };
            unsafe {
                let _ = helper.DragEnter(self.hwnd, data, &raw const point, *effect);
            }
        }
        Ok(())
    }
    fn DragOver(
        &self,
        _: MODIFIERKEYS_FLAGS,
        point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> Result<()> {
        self.update_effect(effect);
        if let Some(helper) = &self.helper {
            let point = POINT {
                x: point.x,
                y: point.y,
            };
            unsafe {
                let _ = helper.DragOver(&raw const point, *effect);
            }
        }
        Ok(())
    }
    fn DragLeave(&self) -> Result<()> {
        self.items.borrow_mut().clear();
        if let Some(helper) = &self.helper {
            unsafe {
                let _ = helper.DragLeave();
            }
        }
        Ok(())
    }
    fn Drop(
        &self,
        data: Ref<IDataObject>,
        _: MODIFIERKEYS_FLAGS,
        point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> Result<()> {
        let items = self.items.take();
        unsafe {
            *effect = if !items.is_empty()
                && (*effect & DROPEFFECT_LINK) != DROPEFFECT_NONE
                && (self.accept)(&items, true)
            {
                DROPEFFECT_LINK
            } else {
                DROPEFFECT_NONE
            };
        }
        if let (Some(helper), Some(data)) = (&self.helper, data.as_ref()) {
            let point = POINT {
                x: point.x,
                y: point.y,
            };
            unsafe {
                let _ = helper.Drop(data, &raw const point, *effect);
            }
        }
        Ok(())
    }
}
impl Target_Impl {
    fn update_effect(&self, effect: *mut DROPEFFECT) {
        let items = self.items.borrow().clone();
        unsafe {
            *effect = if !items.is_empty()
                && (*effect & DROPEFFECT_LINK) != DROPEFFECT_NONE
                && (self.accept)(&items, false)
            {
                DROPEFFECT_LINK
            } else {
                DROPEFFECT_NONE
            };
        }
    }
}
pub(super) struct Registration {
    hwnd: HWND,
    _target: IDropTarget,
}
impl Registration {
    pub fn window(&self) -> HWND {
        self.hwnd
    }
    pub fn new(
        hwnd: HWND,
        accept: impl Fn(&[ShellIdentity], bool) -> bool + 'static,
    ) -> Result<Self> {
        let target: IDropTarget = Target {
            hwnd,
            helper: unsafe {
                CoCreateInstance(&CLSID_DragDropHelper, None, CLSCTX_INPROC_SERVER).ok()
            },
            accept: Rc::new(accept),
            items: RefCell::new(Vec::new()),
        }
        .into();
        unsafe {
            RegisterDragDrop(hwnd, &target)?;
        }
        Ok(Self {
            hwnd,
            _target: target,
        })
    }
}
impl Drop for Registration {
    fn drop(&mut self) {
        let _ = unsafe { RevokeDragDrop(self.hwnd) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::Shell::{IDropTargetHelper_Impl, SHCreateDataObject};

    #[implement(IDropTargetHelper)]
    struct Helper(Rc<RefCell<Vec<(&'static str, i32, i32)>>>);
    impl IDropTargetHelper_Impl for Helper_Impl {
        fn DragEnter(
            &self,
            _: HWND,
            _: Ref<IDataObject>,
            p: *const POINT,
            _: DROPEFFECT,
        ) -> Result<()> {
            let p = unsafe { *p };
            self.0.borrow_mut().push(("enter", p.x, p.y));
            Ok(())
        }
        fn DragOver(&self, p: *const POINT, _: DROPEFFECT) -> Result<()> {
            let p = unsafe { *p };
            self.0.borrow_mut().push(("over", p.x, p.y));
            Ok(())
        }
        fn DragLeave(&self) -> Result<()> {
            self.0.borrow_mut().push(("leave", 0, 0));
            Ok(())
        }
        fn Drop(&self, _: Ref<IDataObject>, p: *const POINT, _: DROPEFFECT) -> Result<()> {
            let p = unsafe { *p };
            self.0.borrow_mut().push(("drop", p.x, p.y));
            Ok(())
        }
        fn Show(&self, _: windows::core::BOOL) -> Result<()> {
            Ok(())
        }
    }
    #[test]
    fn drag_image_helper_receives_screen_coordinates_and_full_lifecycle() {
        let _apartment = desktop_shell::ShellApartment::initialize_sta().unwrap();
        let events = Rc::new(RefCell::new(Vec::new()));
        let target: IDropTarget = Target {
            hwnd: HWND::default(),
            helper: Some(Helper(events.clone()).into()),
            accept: Rc::new(|_, _| false),
            items: RefCell::new(Vec::new()),
        }
        .into();
        unsafe {
            let data: IDataObject = SHCreateDataObject(None, None, None).unwrap();
            let mut effect = DROPEFFECT_LINK;
            let point = POINTL { x: -640, y: 230 };
            target
                .DragEnter(&data, MODIFIERKEYS_FLAGS(0), point, &raw mut effect)
                .unwrap();
            target
                .DragOver(
                    MODIFIERKEYS_FLAGS(0),
                    POINTL { x: -620, y: 240 },
                    &raw mut effect,
                )
                .unwrap();
            target.DragLeave().unwrap();
            target
                .DragEnter(&data, MODIFIERKEYS_FLAGS(0), point, &raw mut effect)
                .unwrap();
            target
                .Drop(&data, MODIFIERKEYS_FLAGS(0), point, &raw mut effect)
                .unwrap();
        }
        assert_eq!(
            *events.borrow(),
            [
                ("enter", -640, 230),
                ("over", -620, 240),
                ("leave", 0, 0),
                ("enter", -640, 230),
                ("drop", -640, 230)
            ]
        );
    }
}
