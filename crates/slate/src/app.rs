//! The calls an app makes: four lines around opening its main window.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use gpui_kit::{
    ActivationPolicy, App, AppContext as _, Application, Bounds, Window, WindowBounds, WindowOptions, WindowingRequest,
    px, size,
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
/// seen and lets clicks through, and, when the `slate` command is waiting for
/// a shot (`SLATE_DIR`), takes the shot once `SLATE_WAIT` is up. Does nothing
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
    let Some(dir) = config.dir.clone() else {
        return;
    };
    let handle = window.window_handle();
    let wait = config.wait;
    cx.spawn(async move |cx| {
        cx.background_executor().timer(wait).await;
        // A fresh frame first, presented so `screencapture -l` sees it too.
        // No await between here and the shot, so the window can't close.
        #[cfg(target_os = "macos")]
        if let Ok(Ok(view)) = cx.update_window(handle, |_, window, _| {
            window.refresh();
            crate::mac::View::of(window)
        }) {
            crate::mac::present(view);
        }
        let shot = cx.update_window(handle, |_, window, cx| shoot(window, cx, &dir));
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

/// Writes the shot file, with the window rendered by the app when it can.
fn shoot(window: &mut Window, cx: &mut App, dir: &Path) -> Result<(), String> {
    let viewport = window.viewport_size();
    let mut shot = Shot {
        window_id: window_id(window)?,
        scale: window.scale_factor(),
        width: f32::from(viewport.width),
        height: f32::from(viewport.height),
        image: None,
        render_error: None,
    };
    match render(window, cx, dir) {
        Ok(path) => shot.image = Some(path),
        Err(problem) => shot.render_error = Some(problem),
    }
    shot.write(dir).map_err(|error| format!("writing {}: {error}", dir.display()))
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
fn render(window: &mut Window, _: &mut App, dir: &Path) -> Result<PathBuf, String> {
    let image = window.render_to_image().map_err(|error| format!("render_to_image: {error:#}"))?;
    let path = dir.join(IMAGE_FILE);
    image
        .save_with_format(&path, image::ImageFormat::Png)
        .map_err(|error| format!("writing {}: {error}", path.display()))?;
    Ok(path)
}

#[cfg(not(feature = "render"))]
fn render(_: &mut Window, _: &mut App, _: &Path) -> Result<PathBuf, String> {
    let _ = IMAGE_FILE;
    Err("the app was built without slate's `render` feature".into())
}
