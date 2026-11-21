//! The calls an app makes: four lines around opening its main window.

use std::path::Path;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use gpui_kit::{
    ActivationPolicy, AnyWindowHandle, App, AppContext as _, Application, AsyncApp, Bounds, Window, WindowBounds,
    WindowOptions, WindowingRequest, px, size,
};

use crate::protocol::{Config, IMAGE_FILE, Shot};

/// The settings for this run, read once from the environment.
pub fn config() -> &'static Config {
    static CONFIG: OnceLock<Config> = OnceLock::new();
    CONFIG.get_or_init(Config::from_env)
}

/// Whether this run is hidden (`SLATE=1`).
pub fn enabled() -> bool {
    config().enabled
}

/// `gpui_kit::application()`, which under Slate starts as an accessory app:
/// no Dock icon, no menu bar, and never the frontmost app.
pub fn application() -> Application {
    hidden(gpui_kit::application())
}

/// Makes an app built some other way start as an accessory app under Slate.
pub fn hidden(app: Application) -> Application {
    if enabled() { app.with_windowing(WindowingRequest::Headless) } else { app }
}

/// The main window's options. Under Slate the window doesn't take focus, and
/// it gets the size the `slate` command asked for (`SLATE_SIZE`), if any.
/// Unchanged otherwise.
pub fn window_options(mut options: WindowOptions, cx: &App) -> WindowOptions {
    let config = config();
    if !config.enabled {
        return options;
    }
    options.focus = false;
    options.show = true;
    if let Some(wanted) = config.size {
        let bounds = Bounds::centered(None, size(px(wanted.width), px(wanted.height)), cx);
        options.window_bounds = Some(WindowBounds::Windowed(bounds));
    }
    options
}

/// Call from the main window's build closure in `open_window`. Under Slate it
/// hides the window below the desktop, where it keeps drawing but is never
/// seen and lets clicks through. Once `SLATE_WAIT` is up it then plays the
/// steps file (`SLATE_STEPS`) and quits the app, or, when the `slate` command
/// is waiting for a single shot (`SLATE_DIR`), takes it. Does nothing
/// otherwise.
pub fn attach(window: &mut Window, cx: &mut App) {
    let config = config();
    if !config.enabled {
        return;
    }
    // Backs up `application()` for apps that start some other way. Set after
    // launch, it can show a Dock icon for a moment first.
    cx.set_activation_policy(ActivationPolicy::Accessory);
    #[cfg(target_os = "macos")]
    if let Err(problem) = crate::mac::hide(window) {
        eprintln!("slate: couldn't hide the window: {problem}");
    }
    let handle = window.window_handle();
    if let (Some(steps), Some(out)) = (config.steps.clone(), config.out.clone()) {
        // The steps need the window to act as the key window: focus events,
        // text cursors and hover all wait for it.
        #[cfg(target_os = "macos")]
        if let Err(problem) = crate::mac::make_key(window) {
            eprintln!("slate: couldn't make the window act as the key window: {problem}");
        }
        crate::drive::start(handle, steps, out, cx);
        return;
    }
    let Some(dir) = config.dir.clone() else {
        return;
    };
    let wait = config.wait;
    cx.spawn(async move |cx| {
        draw_for(handle, wait, cx).await;
        // A fresh frame last, presented so `screencapture -l` sees it too.
        // No await between here and the shot, so the window can't close.
        present(handle, cx);
        let shot = cx.update_window(handle, |_, window, _| shoot(window, &dir));
        match shot {
            Ok(Ok(())) => {}
            Ok(Err(problem)) => eprintln!("slate: no shot: {problem}"),
            Err(problem) => eprintln!("slate: no shot, the window is gone: {problem}"),
        }
    })
    .detach();
}

/// `cx.activate(true)`, except under Slate, where the app must never take
/// focus from the app in front.
pub fn activate(cx: &mut App) {
    if !enabled() {
        cx.activate(true);
    }
}

/// Waits `wait`, drawing and presenting a frame now and every
/// [`SETTLE`](crate::steps::SETTLE) until it's up. A hidden window gets no
/// frames otherwise, so a view the app builds while rendering, and whatever
/// it loads after that, would never show.
pub(crate) async fn draw_for(handle: AnyWindowHandle, wait: Duration, cx: &mut AsyncApp) {
    let start = Instant::now();
    present(handle, cx);
    while let Some(pause) = next_pause(wait, start.elapsed()) {
        cx.background_executor().timer(pause).await;
        present(handle, cx);
    }
}

/// How long to wait before the next frame, `elapsed` into a wait of `wait`,
/// or `None` once it's up.
fn next_pause(wait: Duration, elapsed: Duration) -> Option<Duration> {
    let left = wait.saturating_sub(elapsed);
    (!left.is_zero()).then(|| left.min(crate::steps::SETTLE))
}

/// Has GPUI draw the window and present the frame now. GPUI stops drawing a
/// window below the desktop on its own, since it's occluded.
pub(crate) fn present(handle: AnyWindowHandle, cx: &mut AsyncApp) {
    // Presenting runs the window's frame callback, which updates the App, so
    // it happens outside `update_window`, with nothing awaited in between.
    #[cfg(target_os = "macos")]
    if let Ok(Ok(view)) = cx.update_window(handle, |_, window, _| {
        window.refresh();
        crate::mac::View::of(window)
    }) {
        crate::mac::present(view);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = cx.update_window(handle, |_, window, _| window.refresh());
}

/// Writes the shot file, with the window rendered by the app when it can.
fn shoot(window: &mut Window, dir: &Path) -> Result<(), String> {
    shot(window, &dir.join(IMAGE_FILE))?.write(dir).map_err(|error| format!("writing {}: {error}", dir.display()))
}

/// Describes the window and renders it to `path` when the app can. When it
/// can't, the shot says why and has no image.
pub(crate) fn shot(window: &mut Window, path: &Path) -> Result<Shot, String> {
    let viewport = window.viewport_size();
    let mut shot = Shot {
        window_id: window_id(window)?,
        scale: window.scale_factor(),
        width: f32::from(viewport.width),
        height: f32::from(viewport.height),
        image: None,
        render_error: None,
    };
    match render(window, path) {
        Ok(()) => shot.image = Some(path.to_path_buf()),
        Err(problem) => shot.render_error = Some(problem),
    }
    Ok(shot)
}

#[cfg(target_os = "macos")]
fn window_id(window: &Window) -> Result<u32, String> {
    crate::mac::window_number(window)
}

#[cfg(not(target_os = "macos"))]
fn window_id(_: &Window) -> Result<u32, String> {
    Err("Slate captures windows on macOS only".into())
}

/// Renders the last frame to a PNG with Metal, which needs no Screen
/// Recording permission and doesn't care whether the window can be seen.
#[cfg(feature = "render")]
fn render(window: &mut Window, path: &Path) -> Result<(), String> {
    let image = window.render_to_image().map_err(|error| format!("render_to_image: {error:#}"))?;
    image
        .save_with_format(path, image::ImageFormat::Png)
        .map_err(|error| format!("writing {}: {error}", path.display()))
}

#[cfg(not(feature = "render"))]
fn render(_: &mut Window, _: &Path) -> Result<(), String> {
    Err("the app was built without slate's `render` feature".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SETTLE: Duration = crate::steps::SETTLE;

    #[test]
    fn frames_come_every_settle_until_the_wait_is_up() {
        let wait = Duration::from_millis(120);
        assert_eq!(next_pause(wait, Duration::ZERO), Some(SETTLE));
        assert_eq!(next_pause(wait, Duration::from_millis(100)), Some(Duration::from_millis(20)));
        assert_eq!(next_pause(wait, wait), None);
        assert_eq!(next_pause(wait, Duration::from_secs(1)), None);
    }

    #[test]
    fn no_wait_means_no_pause() {
        assert_eq!(next_pause(Duration::ZERO, Duration::ZERO), None);
    }
}
