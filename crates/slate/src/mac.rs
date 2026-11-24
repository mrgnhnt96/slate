//! The AppKit calls that hide a window, read and choose its menus, and send
//! events to the native views an app lays over GPUI's. This and the web view
//! pictures in `snapshot.rs` are the only unsafe code in Slate, so apps that
//! forbid `unsafe_code` can still run hidden.

#![allow(unsafe_code)]

use std::ffi::{CStr, CString, c_char, c_void};
use std::sync::atomic::{AtomicPtr, Ordering};

use objc::declare::ClassDecl;
use objc::runtime::{BOOL, Class, NO, Object, Sel, YES};
use objc::{class, msg_send, sel, sel_impl};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::keys::AppKitKey;
use crate::menus::{self, MenuItem};

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
    fn sel_getName(selector: *const c_void) -> *const c_char;
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct NSPoint {
    pub x: f64,
    pub y: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct NSSize {
    pub width: f64,
    pub height: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct NSRect {
    pub origin: NSPoint,
    pub size: NSSize,
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
///
/// It's made the app's main window too: GPUI sends menu bar choices to the
/// main window, and decides from it which menu items are enabled. AppKit has
/// no main window while the app is inactive, as an app under Slate always is,
/// so the app's `mainWindow` answers with this window when AppKit's is nil.
///
/// And it says it can be seen: a web view the app hosts stops drawing in a
/// window macOS reports as covered, as one below the desktop is, and drops
/// what it drew once a shot has read it.
pub fn make_key(window: &impl HasWindowHandle) -> Result<(), String> {
    let ns_window = ns_window(window)?;
    // SAFETY: `ns_window` is the live NSWindow behind a GPUI window, on the
    // main thread. Neither subclass adds ivars, so the window's and the app's
    // layouts are the same under either class, and each only overrides a
    // getter.
    unsafe {
        let class: *const Class = msg_send![ns_window, class];
        let class = &*class;
        if !class.name().starts_with(KEY_PREFIX) {
            object_setClass(ns_window, key_subclass(class)?);
        }
        let _: () = msg_send![ns_window, makeMainWindow];
        MAIN.store(ns_window, Ordering::Relaxed);
        let app: Id = msg_send![class!(NSApplication), sharedApplication];
        let app_class: *const Class = msg_send![app, class];
        let app_class = &*app_class;
        if !app_class.name().starts_with(MAIN_PREFIX) {
            object_setClass(app, main_subclass(app_class)?);
        }
        let center: Id = msg_send![class!(NSNotificationCenter), defaultCenter];
        let _: () = msg_send![center, postNotificationName: ns_string("NSWindowDidChangeOcclusionStateNotification")
            object: ns_window];
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

/// `NSWindowOcclusionStateVisible`.
const OCCLUSION_VISIBLE: usize = 1 << 1;

/// `SlateKey_<class>`: the window's own class with `isKeyWindow` answering YES
/// and `occlusionState` saying it can be seen.
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
    extern "C" fn occlusion_state(_: &Object, _: Sel) -> usize {
        OCCLUSION_VISIBLE
    }
    // SAFETY: the method's signature matches
    // `-(NSWindowOcclusionState)occlusionState`, an NSUInteger.
    unsafe { decl.add_method(sel!(occlusionState), occlusion_state as extern "C" fn(&Object, Sel) -> usize) };
    Ok(decl.register())
}

/// The hidden window, for the app's `mainWindow` to answer with.
static MAIN: AtomicPtr<Object> = AtomicPtr::new(std::ptr::null_mut());

const MAIN_PREFIX: &str = "SlateMain_";

/// `NSNotFound`.
const NOT_FOUND: usize = isize::MAX as usize;

/// `SlateMain_<class>`: the app's own class with `mainWindow` answering the
/// hidden window when AppKit has no main window, while that window is open.
fn main_subclass(superclass: &Class) -> Result<&'static Class, String> {
    let name = format!("{MAIN_PREFIX}{}", superclass.name());
    if let Some(class) = Class::get(&name) {
        return Ok(class);
    }
    let mut decl = ClassDecl::new(&name, superclass).ok_or_else(|| format!("couldn't declare {name}"))?;
    extern "C" fn main_window(this: &Object, _: Sel) -> Id {
        // SAFETY: `this` is the app, whose class is a `SlateMain_` subclass,
        // so its superclass has `mainWindow`. The hidden window is compared
        // by pointer against the app's open windows before it's handed out,
        // so a closed one is never returned.
        unsafe {
            let Some(superclass) = this.class().superclass() else {
                return std::ptr::null_mut();
            };
            let main: Id = msg_send![super(this, superclass), mainWindow];
            let hidden = MAIN.load(Ordering::Relaxed);
            if !main.is_null() || hidden.is_null() {
                return main;
            }
            let windows: Id = msg_send![this, windows];
            let index: usize = msg_send![windows, indexOfObjectIdenticalTo: hidden];
            if index == NOT_FOUND { std::ptr::null_mut() } else { hidden }
        }
    }
    // SAFETY: the method's signature matches `-(NSWindow *)mainWindow`.
    unsafe { decl.add_method(sel!(mainWindow), main_window as extern "C" fn(&Object, Sel) -> Id) };
    Ok(decl.register())
}

/// The window's number, the CGWindowID other tools know it by.
pub fn window_number(window: &impl HasWindowHandle) -> Result<u32, String> {
    let ns_window = ns_window(window)?;
    // SAFETY: as in `hide`.
    let number: isize = unsafe { msg_send![ns_window, windowNumber] };
    u32::try_from(number).map_err(|_| format!("the window has no number yet ({number})"))
}

/// The window's GPUI view, for [`present`] and for finding the native views
/// laid over it.
#[derive(Clone, Copy)]
pub struct View(Id);

impl View {
    pub fn of(window: &impl HasWindowHandle) -> Result<Self, String> {
        content_view(window).map(Self)
    }

    /// GPUI's NSView.
    #[cfg(feature = "render")]
    pub(crate) fn id(self) -> Id {
        self.0
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

/// The selectors of the menu items GPUI makes from the app's own menus. The
/// rest (Services, Enter Full Screen and the like) are macOS's.
const APP_ITEM_ACTIONS: [&str; 5] = ["handleGPUIMenuItem:", "cut:", "copy:", "paste:", "selectAll:"];

fn main_menu() -> Result<Id, String> {
    // SAFETY: the shared NSApplication exists while GPUI runs, on its thread.
    let menu: Id = unsafe { msg_send![class!(NSApplication), sharedApplication] };
    let menu: Id = unsafe { msg_send![menu, mainMenu] };
    if menu.is_null() { Err("the app has no menu bar".into()) } else { Ok(menu) }
}

/// The menu bar as macOS would show it now: each menu is updated the way it
/// is when it opens, which asks the app which items are enabled.
///
/// Updating a menu runs GPUI's menu validation, which updates the App: call
/// this while nothing in the App is borrowed, as with [`present`].
pub fn menu_bar() -> Result<Vec<MenuItem>, String> {
    Ok(read_menu(main_menu()?))
}

/// Chooses the item at the end of `path` (menu titles, then the item's), as
/// clicking it in the menu bar would. Refuses a disabled item and macOS's
/// own items. Borrows the App like [`menu_bar`].
pub fn choose_menu(path: &[&str]) -> Result<(), String> {
    let mut menu = main_menu()?;
    let mut place = "the menu bar".to_string();
    for (depth, wanted) in path.iter().enumerate() {
        update(menu);
        let items = visible_items(menu);
        let Some(item) = items.iter().copied().find(|item| item_title(*item) == *wanted) else {
            let titles: Vec<String> = items.iter().map(|item| item_title(*item)).filter(|title| !title.is_empty()).collect();
            return Err(format!("{place} has no {wanted:?}; it has {}", titles.join(", ")));
        };
        // SAFETY: `item` is a live item of `menu`, on the main thread.
        let submenu: Id = unsafe { msg_send![item, submenu] };
        if depth + 1 < path.len() {
            if submenu.is_null() {
                return Err(format!("{wanted:?} in {place} is an item, not a menu"));
            }
            menu = submenu;
            place = format!("{wanted:?}");
            continue;
        }
        if !submenu.is_null() {
            return Err(format!("{wanted:?} is a menu; name an item in it"));
        }
        // SAFETY: as above. Performing the item sends its action the way a
        // click in the menu bar does.
        unsafe {
            let enabled: BOOL = msg_send![item, isEnabled];
            if enabled != YES {
                return Err(format!("{wanted:?} in {place} is disabled"));
            }
            let action = action_name(item);
            if !APP_ITEM_ACTIONS.contains(&action.as_str()) {
                return Err(format!("{wanted:?} is one of macOS's own items ({action}), not the app's"));
            }
            let index: isize = msg_send![menu, indexOfItem: item];
            let _: () = msg_send![menu, performActionForItemAtIndex: index];
        }
    }
    Ok(())
}

fn update(menu: Id) {
    // SAFETY: `menu` is a live NSMenu, on the main thread.
    unsafe {
        let _: () = msg_send![menu, update];
    }
}

fn visible_items(menu: Id) -> Vec<Id> {
    // SAFETY: `menu` is a live NSMenu and every index is below its count.
    unsafe {
        let count: isize = msg_send![menu, numberOfItems];
        (0..count)
            .map(|index| -> Id { msg_send![menu, itemAtIndex: index] })
            .filter(|item| {
                let hidden: BOOL = msg_send![*item, isHidden];
                hidden != YES
            })
            .collect()
    }
}

fn read_menu(menu: Id) -> Vec<MenuItem> {
    update(menu);
    visible_items(menu).into_iter().map(read_item).collect()
}

fn read_item(item: Id) -> MenuItem {
    // SAFETY: `item` is a live NSMenuItem, on the main thread.
    unsafe {
        let separator: BOOL = msg_send![item, isSeparatorItem];
        if separator == YES {
            return MenuItem { separator: true, ..MenuItem::default() };
        }
        let key: Id = msg_send![item, keyEquivalent];
        let mask: usize = msg_send![item, keyEquivalentModifierMask];
        let enabled: BOOL = msg_send![item, isEnabled];
        let state: isize = msg_send![item, state];
        let submenu: Id = msg_send![item, submenu];
        MenuItem {
            title: item_title(item),
            separator: false,
            shortcut: menus::shortcut(&string(key), mask),
            enabled: enabled == YES,
            checked: state == 1,
            items: if submenu.is_null() { Vec::new() } else { read_menu(submenu) },
        }
    }
}

/// An item's title, or its menu's for a menu bar item that has none.
fn item_title(item: Id) -> String {
    // SAFETY: `item` is a live NSMenuItem.
    unsafe {
        let title: Id = msg_send![item, title];
        let title = string(title);
        let submenu: Id = msg_send![item, submenu];
        if !title.is_empty() || submenu.is_null() {
            return title;
        }
        let title: Id = msg_send![submenu, title];
        string(title)
    }
}

fn action_name(item: Id) -> String {
    // SAFETY: `item` is a live NSMenuItem; a selector's name is a static C string.
    unsafe {
        let action: *const c_void = msg_send![item, action];
        if action.is_null() {
            return String::new();
        }
        CStr::from_ptr(sel_getName(action)).to_string_lossy().into_owned()
    }
}

fn string(text: Id) -> String {
    if text.is_null() {
        return String::new();
    }
    // SAFETY: `text` is a live NSString; its UTF-8 copy lives as long as it.
    unsafe {
        let utf8: *const c_char = msg_send![text, UTF8String];
        if utf8.is_null() { String::new() } else { CStr::from_ptr(utf8).to_string_lossy().into_owned() }
    }
}

fn ns_string(text: &str) -> Id {
    let text = CString::new(text.replace('\0', "")).expect("no NUL is left");
    // SAFETY: `stringWithUTF8String:` copies the C string.
    unsafe { msg_send![class!(NSString), stringWithUTF8String: text.as_ptr()] }
}

/// A pointer button, for [`Native::mouse`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mouse {
    Down,
    Up,
    Dragged,
    Moved,
}

/// A view of the app's own that it laid over GPUI's view, like a `wry` web
/// view, which gets AppKit's events instead of GPUI's.
#[derive(Clone, Copy)]
pub struct Native {
    view: View,
    target: Id,
}

impl View {
    fn window(self) -> Id {
        // SAFETY: `self.0` is GPUI's live view.
        unsafe { msg_send![self.0, window] }
    }

    /// A point of GPUI's window (points from the top left) in AppKit's
    /// window coordinates.
    fn window_point(self, x: f32, y: f32) -> NSPoint {
        // SAFETY: `self.0` is GPUI's live view; these only convert geometry.
        unsafe {
            let flipped: BOOL = msg_send![self.0, isFlipped];
            let bounds: NSRect = msg_send![self.0, bounds];
            let y = if flipped == YES { f64::from(y) } else { bounds.size.height - f64::from(y) };
            let nil: Id = std::ptr::null_mut();
            msg_send![self.0, convertPoint: NSPoint { x: f64::from(x), y } toView: nil]
        }
    }

    /// The app's own view at a point of the window, when it isn't GPUI's.
    pub fn native_at(self, x: f32, y: f32) -> Option<Native> {
        let point = self.window_point(x, y);
        // SAFETY: as in `window_point`; `hitTest:` takes a point in the
        // superview's coordinates and returns a view or nil.
        unsafe {
            let superview: Id = msg_send![self.0, superview];
            let point: NSPoint = if superview.is_null() {
                point
            } else {
                let nil: Id = std::ptr::null_mut();
                msg_send![superview, convertPoint: point fromView: nil]
            };
            let hit: Id = msg_send![self.0, hitTest: point];
            self.native(hit)
        }
    }

    /// The app's own view with the keyboard, when it isn't GPUI's.
    pub fn native_keys(self) -> Option<Native> {
        // SAFETY: the window's first responder is a live object or nil.
        let responder: Id = unsafe { msg_send![self.window(), firstResponder] };
        self.native(responder)
    }

    fn native(self, view: Id) -> Option<Native> {
        if view.is_null() || view == self.0 {
            return None;
        }
        // SAFETY: `view` is a live NSResponder; only views answer
        // `isDescendantOf:`, so check it's one first.
        unsafe {
            let is_view: BOOL = msg_send![view, isKindOfClass: class!(NSView)];
            if is_view != YES {
                return None;
            }
            let inside: BOOL = msg_send![view, isDescendantOf: self.0];
            (inside == YES).then_some(Native { view: self, target: view })
        }
    }

    /// Gives the keyboard back to GPUI's view, as a click on it would.
    pub fn take_keys(self) {
        if self.native_keys().is_some() {
            // SAFETY: GPUI's live view in its own window.
            unsafe {
                let _: BOOL = msg_send![self.window(), makeFirstResponder: self.0];
            }
        }
    }
}

impl Native {
    /// Sends the view a left button event at a point of the window, as the
    /// window would. A press gives the view the keyboard when it takes it.
    pub fn mouse(self, kind: Mouse, x: f32, y: f32, flags: usize, clicks: isize) {
        let (event_type, selector): (usize, Sel) = match kind {
            Mouse::Down => (1, sel!(mouseDown:)),
            Mouse::Up => (2, sel!(mouseUp:)),
            Mouse::Moved => (5, sel!(mouseMoved:)),
            Mouse::Dragged => (6, sel!(mouseDragged:)),
        };
        let window = self.view.window();
        let location = self.view.window_point(x, y);
        // SAFETY: a mouse event built for this window, sent to a live view in
        // it with the selector the window would use, on the main thread.
        unsafe {
            let number: isize = msg_send![window, windowNumber];
            let nil: Id = std::ptr::null_mut();
            let pressure: f32 = if kind == Mouse::Up { 0.0 } else { 1.0 };
            let event: Id = msg_send![class!(NSEvent), mouseEventWithType: event_type location: location
                modifierFlags: flags timestamp: uptime() windowNumber: number context: nil eventNumber: 0isize
                clickCount: clicks pressure: pressure];
            if kind == Mouse::Down {
                let takes: BOOL = msg_send![self.target, acceptsFirstResponder];
                if takes == YES {
                    let _: BOOL = msg_send![window, makeFirstResponder: self.target];
                }
            }
            let _: Id = msg_send![self.target, performSelector: selector withObject: event];
        }
    }

    /// Presses and releases a key. A key with Command or Control goes to the
    /// window's views and then the menu bar as a key equivalent first, the
    /// way AppKit sends it; else to the view with the keyboard.
    pub fn key(self, key: &AppKitKey) {
        let window = self.view.window();
        let event = |event_type: usize| -> Id {
            // SAFETY: a key event built for this window.
            unsafe {
                let number: isize = msg_send![window, windowNumber];
                let nil: Id = std::ptr::null_mut();
                msg_send![class!(NSEvent), keyEventWithType: event_type location: NSPoint { x: 0.0, y: 0.0 }
                    modifierFlags: key.flags timestamp: uptime() windowNumber: number context: nil
                    characters: ns_string(&key.characters) charactersIgnoringModifiers: ns_string(&key.ignoring)
                    isARepeat: NO keyCode: key.code]
            }
        };
        let (down, up) = (event(10), event(11));
        // SAFETY: live window, menu and responder, on the main thread.
        unsafe {
            if key.is_equivalent() {
                let handled: BOOL = msg_send![window, performKeyEquivalent: down];
                if handled == YES {
                    return;
                }
                if let Ok(menu) = main_menu() {
                    let handled: BOOL = msg_send![menu, performKeyEquivalent: down];
                    if handled == YES {
                        return;
                    }
                }
            }
            let target: Id = msg_send![window, firstResponder];
            let target = if target.is_null() { self.target } else { target };
            let _: () = msg_send![target, keyDown: down];
            let _: () = msg_send![target, keyUp: up];
        }
    }
}

/// Seconds since the Mac started, the clock NSEvent timestamps use.
fn uptime() -> f64 {
    // SAFETY: a plain getter on the process info singleton.
    unsafe {
        let info: Id = msg_send![class!(NSProcessInfo), processInfo];
        msg_send![info, systemUptime]
    }
}

/// The smallest size the window's content takes, in points: the app's own
/// minimum, or zero when it set none.
pub fn min_size(window: &impl HasWindowHandle) -> Result<crate::Size, String> {
    let ns_window = ns_window(window)?;
    // SAFETY: `ns_window` is the live window; `contentMinSize` returns an NSSize.
    let min: NSSize = unsafe { msg_send![ns_window, contentMinSize] };
    Ok(crate::Size { width: min.width as f32, height: min.height as f32 })
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
