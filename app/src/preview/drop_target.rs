//! OLE link-style collection drops. A successful drop never requests a source file move.
use desktop_core::ShellIdentity;
use std::{cell::RefCell, rc::Rc};
use windows::{
    Win32::{
        Foundation::{HWND, POINTL},
        System::{Com::IDataObject, Ole::*, SystemServices::MODIFIERKEYS_FLAGS},
    },
    core::{Ref, Result, implement},
};

type Accept = Rc<dyn Fn(&[ShellIdentity], bool) -> bool>;
#[implement(IDropTarget)]
struct Target {
    accept: Accept,
    items: RefCell<Vec<ShellIdentity>>,
}
impl IDropTarget_Impl for Target_Impl {
    fn DragEnter(
        &self,
        data: Ref<IDataObject>,
        _: MODIFIERKEYS_FLAGS,
        _: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> Result<()> {
        let items = data
            .as_ref()
            .and_then(|d| desktop_shell::drag_shell_identities(d).ok())
            .unwrap_or_default();
        *self.items.borrow_mut() = items;
        self.DragOver(MODIFIERKEYS_FLAGS(0), &POINTL::default(), effect)
    }
    fn DragOver(&self, _: MODIFIERKEYS_FLAGS, _: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let items = self.items.borrow();
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
        Ok(())
    }
    fn DragLeave(&self) -> Result<()> {
        self.items.borrow_mut().clear();
        Ok(())
    }
    fn Drop(
        &self,
        _: Ref<IDataObject>,
        _: MODIFIERKEYS_FLAGS,
        _: &POINTL,
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
        Ok(())
    }
}
pub(super) struct Registration {
    hwnd: HWND,
    _target: IDropTarget,
}
impl Registration {
    pub fn new(
        hwnd: HWND,
        accept: impl Fn(&[ShellIdentity], bool) -> bool + 'static,
    ) -> Result<Self> {
        let target: IDropTarget = Target {
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
