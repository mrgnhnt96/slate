//! Plays a steps file on the hidden window (`SLATE_STEPS`), saves a PNG for
//! each shot into `SLATE_OUT`, writes the drive's record and quits.
//!
//! A window below the desktop gets no key presses or clicks from macOS, so the
//! app plays them itself: keys through `Window::dispatch_keystroke`, which
//! runs the keymap in the focused element's key contexts and then types into
//! the focused text field, and the mouse through `Window::dispatch_event`.

use std::path::{Path, PathBuf};

use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, AsyncApp, Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, PlatformInput, ScrollDelta, ScrollWheelEvent, TouchPhase, Window, point, px, size,
};

use crate::steps::{self, Button, Drive, DriveShot, SETTLE, Step};

/// Plays the steps once the app has had `wait` to load, then quits the app.
pub(crate) fn start(handle: AnyWindowHandle, steps_file: PathBuf, out: PathBuf, cx: &mut App) {
    let wait = crate::config().wait;
    cx.spawn(async move |cx| {
        cx.background_executor().timer(wait).await;
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
            step => cx
                .update_window(handle, |_, window, cx| input(window, cx, step))
                .unwrap_or_else(|error| Err(format!("the window is gone: {error}"))),
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

/// How long the window server takes to show a presented frame, which
/// `screencapture` reads. Without it, the shot shows the step before.
const COMPOSITE: std::time::Duration = std::time::Duration::from_millis(150);

/// Saves the window as `<name>.png`, rendered by the app when it can, else
/// with `screencapture -l` (which needs Screen Recording permission).
async fn shoot(handle: AnyWindowHandle, out: &Path, name: &str, cx: &mut AsyncApp) -> Result<DriveShot, String> {
    let path = out.join(format!("{name}.png"));
    let mut shot = cx
        .update_window(handle, |_, window, _| crate::app::shot(window, &path))
        .map_err(|error| format!("the window is gone: {error}"))??;
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
        Step::Resize(to) => window.resize(size(px(to.width), px(to.height))),
        Step::Wait(_) | Step::Shot(_) => unreachable!("played by `play`"),
    }
    Ok(())
}

/// The keystroke that types `character`, the way the keyboard sends it.
fn typed(character: char) -> Keystroke {
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
