//! The AppKit calls that hide a window. This is the only unsafe code in
//! Slate, so apps that forbid `unsafe_code` can still run hidden.

#![allow(unsafe_code)]

use objc::declare::ClassDecl;
use objc::runtime::{BOOL, Class, Object, Sel, YES};
use objc::{msg_send, sel, sel_impl};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

type Id = *mut Object;

/// `kCGDesktopWindowLevelKey` from CoreGraphics' `CGWindowLevel.h`.
const DESKTOP_WINDOW_LEVEL_KEY: i32 = 2;

// NSWindowCollectionBehavior bits.
const CAN_JOIN_ALL_SPACES: usize = 1 << 0;
const STATIONARY: usize = 1 << 4;
const IGNORES_CYCLE: usize = 1 << 6;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGWindowLevelForKey(key: i32) -> i32;
}

#[link(name = "objc")]
unsafe extern "C" {
    fn object_setClass(object: Id, class: *const Class) -> *const Class;
}

/// One level below the desktop picture: nothing at this level is ever seen,
/// yet the window keeps its backing store and can still be captured. This is
/// -2147483624 on macOS 15 and 26.
pub fn below_desktop_level() -> i32 {
    // SAFETY: a pure lookup of a constant table in CoreGraphics.
    unsafe { CGWindowLevelForKey(DESKTOP_WINDOW_LEVEL_KEY) - 1 }
}

/// Puts the window below the desktop, lets clicks go through it, and keeps it
/// out of window cycling and Mission Control. Returns its window number
/// (the CGWindowID `screencapture -l` takes).
///
/// Moving the window off screen isn't enough: macOS clamps a titled window
/// back until a strip of it shows, even from (-20000, -20000).
pub fn hide(window: &impl HasWindowHandle) -> Result<u32, String> {
    let ns_window = ns_window(window)?;
    // SAFETY: `ns_window` is the live NSWindow behind a GPUI window, and these
    // are plain NSWindow setters called on the main thread GPUI runs on.
    unsafe {
        let _: () = msg_send![ns_window, setLevel: below_desktop_level() as isize];
        let _: () = msg_send![ns_window, setIgnoresMouseEvents: YES];
        let behavior: usize = msg_send![ns_window, collectionBehavior];
        let behavior = behavior | CAN_JOIN_ALL_SPACES | STATIONARY | IGNORES_CYCLE;
        let _: () = msg_send![ns_window, setCollectionBehavior: behavior];
        let _: () = msg_send![ns_window, setExcludedFromWindowsMenu: YES];
    }
    window_number(window)
}

/// Makes GPUI treat the window as the key window, without macOS making it
/// key or activating the app, so the app in front keeps focus.
///
/// GPUI hides focus from a window that isn't key: focus listeners see no
/// focused element, text fields hide their cursor and selection, and on macOS
/// nothing shows hover. A window below the desktop in an app that never
/// activates is never key, so Slate swaps this one window's class for a
/// subclass whose `isKeyWindow` says YES (the way key-value observing swaps
/// classes), then tells GPUI's window delegate the window became key. GPUI
/// reads `isKeyWindow` there and marks the window active. AppKit's own key
/// window and the frontmost app don't change.
pub fn make_key(window: &impl HasWindowHandle) -> Result<(), String> {
    let ns_window = ns_window(window)?;
    // SAFETY: `ns_window` is the live NSWindow behind a GPUI window, on the
    // main thread. The subclass adds no ivars, so the object's layout is the
    // same under either class, and it only overrides a getter.
    unsafe {
        let class: *const Class = msg_send![ns_window, class];
        let class = &*class;
        if !class.name().starts_with(KEY_PREFIX) {
            object_setClass(ns_window, key_subclass(class)?);
        }
        let delegate: Id = msg_send![ns_window, delegate];
        if delegate.is_null() {
            return Err("the window has no delegate to tell".into());
        }
        let responds: BOOL = msg_send![delegate, respondsToSelector: sel!(windowDidBecomeKey:)];
        if responds != YES {
            return Err("the window's delegate doesn't handle windowDidBecomeKey:".into());
        }
        let nil: Id = std::ptr::null_mut();
        let _: () = msg_send![delegate, windowDidBecomeKey: nil];
    }
    Ok(())
}

const KEY_PREFIX: &str = "SlateKey_";

/// `SlateKey_<class>`: the window's own class with `isKeyWindow` answering YES.
fn key_subclass(superclass: &Class) -> Result<&'static Class, String> {
    let name = format!("{KEY_PREFIX}{}", superclass.name());
    if let Some(class) = Class::get(&name) {
        return Ok(class);
    }
    let mut decl = ClassDecl::new(&name, superclass).ok_or_else(|| format!("couldn't declare {name}"))?;
    extern "C" fn is_key_window(_: &Object, _: Sel) -> BOOL {
        YES
    }
    // SAFETY: the method's signature matches `-(BOOL)isKeyWindow`.
    unsafe { decl.add_method(sel!(isKeyWindow), is_key_window as extern "C" fn(&Object, Sel) -> BOOL) };
    Ok(decl.register())
}

/// The window's number, the CGWindowID other tools know it by.
pub fn window_number(window: &impl HasWindowHandle) -> Result<u32, String> {
    let ns_window = ns_window(window)?;
    // SAFETY: as in `hide`.
    let number: isize = unsafe { msg_send![ns_window, windowNumber] };
    u32::try_from(number).map_err(|_| format!("the window has no number yet ({number})"))
}

/// The window's GPUI view, for [`present`].
pub struct View(Id);

impl View {
    pub fn of(window: &impl HasWindowHandle) -> Result<Self, String> {
        content_view(window).map(Self)
    }
}

/// Makes GPUI draw and present a frame now. A window below the desktop is
/// occluded, and GPUI stops its display link for occluded windows, so without
/// this the window keeps showing the frame from before it was hidden.
///
/// GPUI's view answers `displayLayer:` by running the window's frame callback,
/// which updates the App: call this while nothing in the App is borrowed (not
/// inside `update_window`), and while the window is still open.
pub fn present(view: View) {
    // SAFETY: `view.0` is GPUI's live NSView (the caller hasn't let the window
    // close since `View::of`), and GPUI ignores the layer argument.
    unsafe {
        let nil: Id = std::ptr::null_mut();
        let _: () = msg_send![view.0, displayLayer: nil];
    }
}

fn content_view(window: &impl HasWindowHandle) -> Result<Id, String> {
    let handle = window.window_handle().map_err(|error| format!("no window handle: {error}"))?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return Err("not an AppKit window".into());
    };
    Ok(handle.ns_view.as_ptr() as Id)
}

fn ns_window(window: &impl HasWindowHandle) -> Result<Id, String> {
    let view = content_view(window)?;
    // SAFETY: `view` is the window's live content view.
    let ns_window: Id = unsafe { msg_send![view, window] };
    if ns_window.is_null() { Err("the view has no window yet".into()) } else { Ok(ns_window) }
}

#[cfg(test)]
mod tests {
    #[test]
    fn below_desktop_is_the_level_proven_on_kaepora() {
        assert_eq!(super::below_desktop_level(), -2147483624);
    }
}
