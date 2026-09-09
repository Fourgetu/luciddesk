//! Disposable OLE receiver: records the insertion target without moving files.
#![allow(clippy::ref_as_ptr, clippy::inline_always, clippy::wildcard_imports)]
use std::{cell::Cell, rc::Rc};
use windows::{core::{implement, Interface, Ref, Result}, Win32::{Foundation::{HWND, POINTL}, System::{Com::IDataObject, Ole::*, SystemServices::MODIFIERKEYS_FLAGS}}};
use windows_sys::Win32::UI::{Controls::*, WindowsAndMessaging::{GetPropW, SendMessageW}};

type Record = (i32, bool, i32, i32);
#[implement(IDropTarget)]
struct Receiver { view: isize, record: Rc<Cell<Record>> }
impl IDropTarget_Impl for Receiver_Impl {
    fn DragEnter(&self, _: Ref<IDataObject>, _: MODIFIERKEYS_FLAGS, _: &POINTL, _: *mut DROPEFFECT) -> Result<()> { Ok(()) }
    fn DragOver(&self, _: MODIFIERKEYS_FLAGS, _: &POINTL, _: *mut DROPEFFECT) -> Result<()> { Ok(()) }
    fn DragLeave(&self) -> Result<()> { Ok(()) }
    fn Drop(&self, _: Ref<IDataObject>, _: MODIFIERKEYS_FLAGS, point: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let mut mark = LVINSERTMARK { cbSize: u32::try_from(std::mem::size_of::<LVINSERTMARK>()).unwrap(), iItem: -1, ..Default::default() };
        unsafe {
            SendMessageW(self.view as _, LVM_GETINSERTMARK, 0, (&raw mut mark) as isize);
            *effect = DROPEFFECT_MOVE;
        }
        self.record.set((mark.iItem, mark.dwFlags & LVIM_AFTER != 0, point.x, point.y));
        Ok(())
    }
}

pub struct Capture { view: HWND, record: Rc<Cell<Record>>, _receiver: IDropTarget }
impl Capture {
    pub fn attach(view: windows_sys::Win32::Foundation::HWND) -> Self {
        unsafe { OleInitialize(None).unwrap(); }
        let record = Rc::new(Cell::new((-1, false, 0, 0)));
        let receiver: IDropTarget = Receiver { view: view as isize, record: record.clone() }.into();
        unsafe { RegisterDragDrop(HWND(view), &receiver).unwrap(); }
        Self { view: HWND(view), record, _receiver: receiver }
    }
    pub fn drop_at(&self, x: i32, y: i32) -> Record {
        unsafe {
            let pointer = GetPropW(self.view.0, windows_sys::w!("OleDropTargetInterface"));
            let proxy = IDropTarget::from_raw_borrowed(&pointer).unwrap();
            let mut effect = DROPEFFECT_MOVE;
            proxy.Drop(None::<&IDataObject>, MODIFIERKEYS_FLAGS(0), POINTL { x, y }, &raw mut effect).unwrap();
        }
        self.record.get()
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        unsafe { RevokeDragDrop(self.view).unwrap(); OleUninitialize(); }
    }
}
