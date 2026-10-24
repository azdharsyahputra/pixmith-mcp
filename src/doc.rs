//! A safe handle around one core document, plus the small FFI helpers every tool module shares.
//!
//! The server talks to the core through the same `pixmith_*` entry points the macOS and egui apps use, so it
//! gets the same behavior: panic guards, undo checkpoints, linked cels, stroke state. Raw pointer calls live
//! in the tool modules next to the arguments they convert, always on a live `Doc` owned by the engine thread.

use pixmith_core::document::Sprite;
use pixmith_core::ffi::{self, document as core};
use pixmith_core::{Editor, history, paint::shapes};
use std::ffi::{CStr, CString, c_char};

/// Marker written into `pixmith_last_error` before a call, so a message left over from an earlier failure is
/// never reported for a later one. Opening a file that cannot exist is the only side-effect-free way to set it.
const NO_ERROR: &str = "\u{1}pixmith-mcp-no-error";

/// Resets the core's last error message (see [`NO_ERROR`]).
pub fn clear_error() {
    let c = cstr(NO_ERROR);
    // SAFETY: `c` is a valid C string; opening it fails and returns null (nothing to free).
    let p = unsafe { core::pixmith_open(c.as_ptr()) };
    if !p.is_null() {
        // SAFETY: a document the core just allocated.
        unsafe { core::pixmith_free(p) };
    }
}

/// The core's message for the last failing call on this thread, or `None` when none failed since
/// [`clear_error`].
pub fn core_error() -> Option<String> {
    // SAFETY: the core returns a valid, NUL-terminated thread-local string.
    let s = unsafe { CStr::from_ptr(ffi::pixmith_last_error()) }.to_string_lossy().into_owned();
    (!s.is_empty() && !s.starts_with(NO_ERROR)).then_some(s)
}

/// `what` plus the core's reason, when it gave one.
pub fn fail(what: impl Into<String>) -> String {
    let what = what.into();
    match core_error() {
        Some(e) => format!("{what}: {e}"),
        None => what,
    }
}

/// A NUL-free C string (interior NULs are dropped).
pub fn cstr(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap_or_default()
}

/// Takes ownership of a string the core allocated; null becomes "".
///
/// # Safety
/// `s` must be null or a string returned by the core that has not been freed.
pub unsafe fn take_string(s: *mut c_char) -> String {
    if s.is_null() {
        return String::new();
    }
    let out = unsafe { CStr::from_ptr(s) }.to_string_lossy().into_owned();
    unsafe { ffi::pixmith_string_free(s) };
    out
}

/// One open document. Owned by the engine thread; freed on drop.
pub struct Doc {
    ptr: *mut Editor,
}

impl Doc {
    pub fn new(width: u32, height: u32) -> Result<Doc, String> {
        let ptr = core::pixmith_new(width, height);
        if ptr.is_null() { Err(fail("could not create the document")) } else { Ok(Doc { ptr }) }
    }

    /// Takes ownership of a document the core allocated (open, import, snapshot); `what` describes the
    /// operation for the error message when `ptr` is null.
    pub fn adopt(ptr: *mut Editor, what: &str) -> Result<Doc, String> {
        if ptr.is_null() { Err(fail(what)) } else { Ok(Doc { ptr }) }
    }

    /// The raw document pointer for `pixmith_*` calls. Valid as long as `self` lives.
    pub fn raw(&self) -> *mut Editor {
        self.ptr
    }

    /// The sprite, with pending linked-cel edits flushed (exactly what `pixmith_render` reads).
    pub fn sprite(&self) -> &Sprite {
        // SAFETY: `ptr` is a live editor owned by `self`; the reference lives no longer than `&self`, and the
        // engine thread is the only user of the document.
        unsafe {
            (*self.ptr).sprite.flush_links();
            &(*self.ptr).sprite
        }
    }

    pub fn width(&self) -> u32 {
        unsafe { core::pixmith_width(self.ptr) }
    }

    pub fn height(&self) -> u32 {
        unsafe { core::pixmith_height(self.ptr) }
    }

    pub fn layer_count(&self) -> u32 {
        unsafe { core::pixmith_layer_count(self.ptr) }
    }

    pub fn frame_count(&self) -> u32 {
        unsafe { core::pixmith_frame_count(self.ptr) }
    }

    pub fn is_dirty(&self) -> bool {
        unsafe { history::pixmith_is_dirty(self.ptr) }
    }

    /// Applied undo steps and undone (redoable) steps.
    pub fn history_counts(&self) -> (u32, u32) {
        let (mut undo, mut redo) = (0, 0);
        unsafe { history::pixmith_history_counts(self.ptr, &mut undo, &mut redo) };
        (undo, redo)
    }

    /// Names the newest undo step, unless the core already named it.
    pub fn label(&self, label: &str) {
        let c = cstr(label);
        unsafe { history::pixmith_checkpoint_label(self.ptr, c.as_ptr()) }
    }

    /// Marks the document changed (the apps call this after every mutation).
    pub fn touch(&self) {
        unsafe { history::pixmith_touch(self.ptr) }
    }

    /// Opens an undo step for an edit that does not take its own checkpoint.
    pub fn begin_stroke(&self) {
        unsafe { core::pixmith_begin_stroke(self.ptr) }
    }

    pub fn begin_transaction(&self) {
        unsafe { history::pixmith_begin_transaction(self.ptr) }
    }

    pub fn end_transaction(&self) {
        unsafe { history::pixmith_end_transaction(self.ptr) }
    }

    /// Drops the newest undo step and restores its state, leaving no redo step.
    pub fn rollback(&self) -> bool {
        unsafe { shapes::pixmith_rollback(self.ptr) }
    }

    pub fn mark_saved(&self) {
        unsafe { history::pixmith_mark_saved(self.ptr) }
    }

    /// Composites the visible layers of `frame` (straight-alpha RGBA, `width * height * 4` bytes).
    pub fn render(&self, frame: u32) -> Result<Vec<u8>, String> {
        let mut out = vec![0u8; self.width() as usize * self.height() as usize * 4];
        clear_error();
        if unsafe { core::pixmith_render(self.ptr, frame, out.as_mut_ptr(), out.len()) } {
            Ok(out)
        } else {
            Err(fail(format!("could not render frame {frame}")))
        }
    }
}

impl Drop for Doc {
    fn drop(&mut self) {
        // SAFETY: the pointer came from the core and is freed exactly once.
        shapes::pixmith_stroke_end_doc(self.ptr);
        unsafe { core::pixmith_free(self.ptr) }
    }
}
