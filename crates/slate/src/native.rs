//! Steps that land on a native view the app laid over its GPUI view, like a
//! `wry` web view. GPUI never sees those clicks and keys, so they go to the
//! view as AppKit events, the way the window would send them.
//!
//! Each runs while nothing in the App is borrowed: a web view can answer by
//! giving the keyboard back to GPUI, and a key equivalent can choose a menu
//! item, and both update the App.

use gpui_kit::{AnyWindowHandle, AppContext as _, AsyncApp, Keystroke};

use crate::keys::{self, AppKitKey};
use crate::mac::{Mouse, Native, View};
use crate::menus::{COMMAND, CONTROL, OPTION, SHIFT};
use crate::steps::{self, Button, Step};

fn view(handle: AnyWindowHandle, cx: &mut AsyncApp) -> Result<View, String> {
    cx.update_window(handle, |_, window, _| View::of(window)).map_err(|error| format!("the window is gone: {error}"))?
}

/// The native view at a point of the window, if one is there.
pub(crate) fn at(handle: AnyWindowHandle, x: f32, y: f32, cx: &mut AsyncApp) -> Result<Option<Native>, String> {
    Ok(view(handle, cx)?.native_at(x, y))
}

/// Plays a click, move, key or type step when it goes to a native view.
/// False when GPUI should play it. A click on GPUI's view takes the keyboard
/// back from a native view first, as a click would.
pub(crate) fn play(handle: AnyWindowHandle, step: &Step, cx: &mut AsyncApp) -> Result<bool, String> {
    let view = view(handle, cx)?;
    match step {
        Step::Click(click) => {
            let Some(native) = view.native_at(click.x, click.y) else {
                view.take_keys();
                return Ok(false);
            };
            if click.button != Button::Left {
                return Err("only the left button clicks in a web view: its context menu would stay open".into());
            }
            let flags = flags(click.modifiers.as_deref())?;
            for clicks in 1..=click.count as isize {
                native.mouse(Mouse::Down, click.x, click.y, flags, clicks);
                native.mouse(Mouse::Up, click.x, click.y, flags, clicks);
            }
            Ok(true)
        }
        Step::Move(to) => Ok(view.native_at(to.x, to.y).map(|native| native.mouse(Mouse::Moved, to.x, to.y, 0, 0)).is_some()),
        Step::Key(text) => {
            for key in text.split_whitespace() {
                let keystroke = Keystroke::parse(key).map_err(|error| format!("{key:?}: {error}"))?;
                match view.native_keys() {
                    Some(native) => native.key(&named(&keystroke)?),
                    None => gpui_key(handle, keystroke, cx)?,
                }
            }
            Ok(true)
        }
        Step::Type(text) => {
            for character in text.chars() {
                match view.native_keys() {
                    Some(native) => native.key(&keys::typed(character)),
                    None => gpui_key(handle, crate::drive::typed(character), cx)?,
                }
            }
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn gpui_key(handle: AnyWindowHandle, keystroke: Keystroke, cx: &mut AsyncApp) -> Result<(), String> {
    cx.update_window(handle, |_, window, cx| {
        window.dispatch_keystroke(keystroke, cx);
    })
    .map_err(|error| format!("the window is gone: {error}"))
}

fn named(keystroke: &Keystroke) -> Result<AppKitKey, String> {
    let held = keystroke.modifiers;
    let modifiers = steps::Modifiers {
        control: held.control,
        alt: held.alt,
        shift: held.shift,
        platform: held.platform,
        function: held.function,
    };
    keys::named(&keystroke.key, modifiers)
}

/// Modifier keys as `NSEventModifierFlags`.
pub(crate) fn flags(text: Option<&str>) -> Result<usize, String> {
    let Some(text) = text else {
        return Ok(0);
    };
    let held = steps::Modifiers::parse(text)?;
    Ok([(held.shift, SHIFT), (held.control, CONTROL), (held.alt, OPTION), (held.platform, COMMAND)]
        .into_iter()
        .filter(|(on, _)| *on)
        .fold(0, |flags, (_, flag)| flags | flag))
}
