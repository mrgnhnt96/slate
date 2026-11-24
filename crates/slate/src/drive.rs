//! Plays a steps file on the hidden window (`SLATE_STEPS`), saves a PNG for
//! each shot into `SLATE_OUT`, writes the drive's record and quits.
//!
//! A window below the desktop gets no key presses or clicks from macOS, so the
//! app plays them itself: keys through `Window::dispatch_keystroke`, which
//! runs the keymap in the focused element's key contexts and then types into
//! the focused text field, and the mouse through `Window::dispatch_event`.
//! Clicks, drags and keys meant for a native view the app laid over GPUI's
//! (a web view) go to that view as AppKit events instead (`native`).

use std::path::{Path, PathBuf};

use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, AsyncApp, Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, PlatformInput, ScrollDelta, ScrollWheelEvent, TouchPhase, Window, point, px, size,
};

use crate::steps::{self, Button, DRAG_FRAME, Drag, Drive, DriveShot, SETTLE, Step};

/// Plays the steps once the app has had `wait` to load, then quits the app.
pub(crate) fn start(handle: AnyWindowHandle, steps_file: PathBuf, out: PathBuf, cx: &mut App) {
    let wait = crate::config().wait;
    cx.spawn(async move |cx| {
        // Frames while the app loads, so a view it builds while rendering
        // gets to load before the first step.
        crate::app::draw_for(handle, wait, cx).await;
        let drive = play(handle, &steps_file, &out, cx).await;
        if let Some(problem) = &drive.error {
            eprintln!("slate: {problem}");
        }
        if let Err(error) = drive.write(&out) {
            eprintln!("slate: writing the drive into {}: {error}", out.display());
        }
        cx.update(|cx| cx.quit());
    })
    .detach();
}

async fn play(handle: AnyWindowHandle, steps_file: &Path, out: &Path, cx: &mut AsyncApp) -> Drive {
    let mut drive = Drive::default();
    let steps = match steps::read(steps_file) {
        Ok(steps) => steps,
        Err(problem) => {
            drive.error = Some(problem);
            return drive;
        }
    };
    if let Err(error) = std::fs::create_dir_all(out) {
        drive.error = Some(format!("making {}: {error}", out.display()));
        return drive;
    }
    // Draw first, so the first click lands on what the window shows now.
    crate::app::present(handle, cx);
    for (index, step) in steps.iter().enumerate() {
        let played = match step {
            Step::Wait(ms) => {
                cx.background_executor().timer(std::time::Duration::from_millis(*ms)).await;
                Ok(())
            }
            Step::Shot(name) => shoot(handle, out, name, cx).await.map(|shot| drive.shots.push(shot)),
            Step::Menus(name) => save_menus(out, name).map(|()| drive.menus.push(name.clone())),
            Step::Menu(path) => choose_menu(path),
            Step::Drag(drag) => self::drag(handle, drag, cx).await,
            step => play_input(handle, step, cx),
        };
        if let Err(problem) = played {
            drive.error = Some(format!("step {} ({}): {problem}", index + 1, step.describe()));
            break;
        }
        drive.played += 1;
        // Time for the app to handle the step (focus listeners, spawned
        // tasks, a resize), then a frame, so the next click hits what changed.
        cx.background_executor().timer(SETTLE).await;
        crate::app::present(handle, cx);
    }
    drive
}

fn play_input(handle: AnyWindowHandle, step: &Step, cx: &mut AsyncApp) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    if crate::native::play(handle, step, cx)? {
        return Ok(());
    }
    cx.update_window(handle, |_, window, cx| input(window, cx, step))
        .unwrap_or_else(|error| Err(format!("the window is gone: {error}")))
}

/// Presses the left button at `from`, moves the pointer to `to` with it held,
/// drawing a frame after each move so drag handlers registered on the way
/// see the next one, and releases it there.
async fn drag(handle: AnyWindowHandle, drag: &Drag, cx: &mut AsyncApp) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    if let Some(native) = crate::native::at(handle, drag.from.x, drag.from.y, cx)? {
        use crate::mac::Mouse;
        let flags = crate::native::flags(drag.modifiers.as_deref())?;
        native.mouse(Mouse::Down, drag.from.x, drag.from.y, flags, 1);
        for index in 1..=drag.moves {
            frame(handle, cx).await;
            let at = drag.at(index);
            native.mouse(Mouse::Dragged, at.x, at.y, flags, 1);
        }
        frame(handle, cx).await;
        native.mouse(Mouse::Up, drag.to.x, drag.to.y, flags, 1);
        return Ok(());
    }
    let modifiers = modifiers(drag.modifiers.as_deref())?;
    let from = point(px(drag.from.x), px(drag.from.y));
    let button = MouseButton::Left;
    dispatch(handle, PlatformInput::MouseMove(MouseMoveEvent { position: from, pressed_button: None, modifiers }), cx)?;
    let down = MouseDownEvent { button, position: from, modifiers, click_count: 1, first_mouse: false };
    dispatch(handle, PlatformInput::MouseDown(down), cx)?;
    for index in 1..=drag.moves {
        frame(handle, cx).await;
        let at = drag.at(index);
        let position = point(px(at.x), px(at.y));
        dispatch(handle, PlatformInput::MouseMove(MouseMoveEvent { position, pressed_button: Some(button), modifiers }), cx)?;
    }
    frame(handle, cx).await;
    let to = point(px(drag.to.x), px(drag.to.y));
    dispatch(handle, PlatformInput::MouseUp(MouseUpEvent { button, position: to, modifiers, click_count: 1 }), cx)
}

async fn frame(handle: AnyWindowHandle, cx: &mut AsyncApp) {
    cx.background_executor().timer(DRAG_FRAME).await;
    crate::app::present(handle, cx);
}

fn dispatch(handle: AnyWindowHandle, event: PlatformInput, cx: &mut AsyncApp) -> Result<(), String> {
    cx.update_window(handle, |_, window, cx| {
        window.dispatch_event(event, cx);
    })
    .map_err(|error| format!("the window is gone: {error}"))
}

/// Saves the menu bar as `<name>.json`.
fn save_menus(out: &Path, name: &str) -> Result<(), String> {
    let bar = menu_bar()?;
    let path = out.join(format!("{name}.json"));
    let text = serde_json::to_string_pretty(&bar).map_err(|error| error.to_string())?;
    std::fs::write(&path, text + "\n").map_err(|error| format!("writing {}: {error}", path.display()))
}

#[cfg(target_os = "macos")]
fn min_size(window: &Window) -> Result<crate::Size, String> {
    crate::mac::min_size(window)
}

#[cfg(not(target_os = "macos"))]
fn min_size(_: &Window) -> Result<crate::Size, String> {
    Ok(crate::Size { width: 0., height: 0. })
}

#[cfg(target_os = "macos")]
fn menu_bar() -> Result<Vec<crate::menus::MenuItem>, String> {
    crate::mac::menu_bar()
}

#[cfg(target_os = "macos")]
fn choose_menu(path: &str) -> Result<(), String> {
    crate::mac::choose_menu(&steps::menu_path(path)?)
}

#[cfg(not(target_os = "macos"))]
fn menu_bar() -> Result<Vec<crate::menus::MenuItem>, String> {
    Err("Slate reads the menu bar on macOS only".into())
}

#[cfg(not(target_os = "macos"))]
fn choose_menu(_: &str) -> Result<(), String> {
    Err("Slate chooses from the menu bar on macOS only".into())
}

/// How long the window server takes to show a presented frame, which
/// `screencapture` reads. Without it, the shot shows the step before.
const COMPOSITE: std::time::Duration = std::time::Duration::from_millis(150);

/// Saves the window as `<name>.png`, rendered by the app when it can, else
/// with `screencapture -l` (which needs Screen Recording permission).
async fn shoot(handle: AnyWindowHandle, out: &Path, name: &str, cx: &mut AsyncApp) -> Result<DriveShot, String> {
    let path = out.join(format!("{name}.png"));
    let mut shot = crate::app::shot(handle, &path, cx).await?;
    if shot.image.is_none() {
        cx.background_executor().timer(COMPOSITE).await;
        screencapture(shot.window_id, &path)?;
        shot.image = Some(path);
    }
    Ok(DriveShot { name: name.to_string(), shot })
}

fn screencapture(window_id: u32, path: &Path) -> Result<(), String> {
    let output = std::process::Command::new("screencapture")
        .args(["-x", "-o", "-l", &window_id.to_string()])
        .arg(path)
        .output()
        .map_err(|error| format!("running screencapture: {error}"))?;
    if path.exists() {
        return Ok(());
    }
    Err(format!(
        "screencapture -l {window_id} gave no image ({}). It needs Screen Recording permission for the terminal, or \
         build the app with slate's `render` feature",
        String::from_utf8_lossy(&output.stderr).trim()
    ))
}

/// Plays a key, mouse or resize step on the window.
fn input(window: &mut Window, cx: &mut App, step: &Step) -> Result<(), String> {
    match step {
        Step::Key(keys) => {
            for key in keys.split_whitespace() {
                let keystroke = Keystroke::parse(key).map_err(|error| format!("{key:?}: {error}"))?;
                window.dispatch_keystroke(keystroke, cx);
            }
        }
        Step::Type(text) => {
            for character in text.chars() {
                window.dispatch_keystroke(typed(character), cx);
            }
        }
        Step::Click(click) => {
            let position = point(px(click.x), px(click.y));
            let modifiers = modifiers(click.modifiers.as_deref())?;
            let button = match click.button {
                Button::Left => MouseButton::Left,
                Button::Right => MouseButton::Right,
                Button::Middle => MouseButton::Middle,
            };
            window.dispatch_event(
                PlatformInput::MouseMove(MouseMoveEvent { position, pressed_button: None, modifiers }),
                cx,
            );
            for click_count in 1..=click.count as usize {
                window.dispatch_event(
                    PlatformInput::MouseDown(MouseDownEvent {
                        button,
                        position,
                        modifiers,
                        click_count,
                        first_mouse: false,
                    }),
                    cx,
                );
                window.dispatch_event(
                    PlatformInput::MouseUp(MouseUpEvent { button, position, modifiers, click_count }),
                    cx,
                );
            }
        }
        Step::Move(to) => {
            let position = point(px(to.x), px(to.y));
            window.dispatch_event(
                PlatformInput::MouseMove(MouseMoveEvent {
                    position,
                    pressed_button: None,
                    modifiers: Modifiers::default(),
                }),
                cx,
            );
        }
        Step::Scroll(scroll) => {
            // GPUI's wheel delta moves the content: down the page is negative.
            let delta = ScrollDelta::Pixels(point(px(-scroll.dx), px(-scroll.dy)));
            let position = point(px(scroll.x), px(scroll.y));
            window.dispatch_event(
                PlatformInput::ScrollWheel(ScrollWheelEvent {
                    position,
                    delta,
                    modifiers: Modifiers::default(),
                    touch_phase: TouchPhase::Moved,
                }),
                cx,
            );
        }
        Step::Resize(to) => {
            // A person can't drag the window below the app's own minimum.
            let to = to.at_least(min_size(window)?);
            window.resize(size(px(to.width), px(to.height)));
        }
        Step::Wait(_) | Step::Shot(_) | Step::Drag(_) | Step::Menu(_) | Step::Menus(_) => {
            unreachable!("played by `play`")
        }
    }
    Ok(())
}

/// The keystroke that types `character`, the way the keyboard sends it.
pub(crate) fn typed(character: char) -> Keystroke {
    let (key, shift) = match character {
        ' ' => ("space".to_string(), false),
        '\n' => ("enter".to_string(), false),
        '\t' => ("tab".to_string(), false),
        c if c.is_uppercase() => (c.to_lowercase().collect(), true),
        c => (c.to_string(), false),
    };
    Keystroke { modifiers: Modifiers { shift, ..Modifiers::default() }, key, key_char: Some(character.to_string()) }
}

fn modifiers(text: Option<&str>) -> Result<Modifiers, String> {
    let Some(text) = text else {
        return Ok(Modifiers::default());
    };
    let read = steps::Modifiers::parse(text)?;
    Ok(Modifiers {
        control: read.control,
        alt: read.alt,
        shift: read.shift,
        platform: read.platform,
        function: read.function,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_sends_the_key_and_the_character() {
        let a = typed('a');
        assert_eq!((a.key.as_str(), a.key_char.as_deref(), a.modifiers.shift), ("a", Some("a"), false));
        let capital = typed('Q');
        assert_eq!(
            (capital.key.as_str(), capital.key_char.as_deref(), capital.modifiers.shift),
            ("q", Some("Q"), true)
        );
        assert_eq!(typed(' ').key, "space");
        assert_eq!(typed('\n').key, "enter");
        assert_eq!(typed('é').key_char.as_deref(), Some("é"));
    }

    #[test]
    fn click_modifiers_become_gpui_modifiers() {
        let read = modifiers(Some("cmd-shift")).unwrap();
        assert!(read.platform && read.shift && !read.control);
        assert_eq!(modifiers(None).unwrap(), Modifiers::default());
        assert!(modifiers(Some("hyper")).is_err());
    }
}
