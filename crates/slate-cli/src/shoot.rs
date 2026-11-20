//! `slate shoot`: launch, wait for the shot, capture, scale, quit.

use std::path::Path;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use image::RgbaImage;
use image::imageops::FilterType;
use slate::protocol::{ENV_DIR, ENV_ENABLED, ENV_SIZE, ENV_WAIT};
use slate::{Shot, Size};

use crate::ShootArgs;

/// How the image was captured, for the summary line.
#[derive(Debug, PartialEq)]
enum Source {
    /// The app rendered its own window with Metal.
    App,
    /// `screencapture -l`, because the app couldn't render it.
    Screencapture { why: String },
}

pub fn run(args: &ShootArgs) -> Result<(), String> {
    let dir = tempfile::Builder::new()
        .prefix("slate-")
        .tempdir()
        .map_err(|error| format!("making a folder for the shot: {error}"))?;
    let mut app = launch(&args.app, args.size, args.wait, &[(ENV_DIR, dir.path())])?;
    let shot = wait_for_shot(&mut app, dir.path(), Duration::from_millis(args.wait + args.timeout))?;
    let (image, source) = capture(&shot, dir.path())?;
    // The window is captured: the app can go before the image is written.
    drop(app);
    let image = to_points(image, shot.points());
    if let Some(parent) = args.out.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|error| format!("making {}: {error}", parent.display()))?;
    }
    image
        .save_with_format(&args.out, image::ImageFormat::Png)
        .map_err(|error| format!("writing {}: {error}", args.out.display()))?;
    let how = match source {
        Source::App => "rendered by the app".to_string(),
        Source::Screencapture { why } => format!("screencapture -l, since the app couldn't render it: {why}"),
    };
    println!("{} ({}x{} points, {how})", args.out.display(), image.width(), image.height());
    Ok(())
}

/// The app, quit when this goes out of scope however `run` ends.
pub(crate) struct App {
    pub(crate) child: Child,
    pub(crate) name: String,
}

impl Drop for App {
    fn drop(&mut self) {
        quit(&mut self.child);
    }
}

/// Launches the app under Slate, with the window `size`, `wait` milliseconds
/// to load and the rest of the handshake in `env`.
pub(crate) fn launch(app: &[String], size: Option<Size>, wait: u64, env: &[(&str, &Path)]) -> Result<App, String> {
    let (program, rest) = app.split_first().ok_or("no app to launch")?;
    let mut command = Command::new(program);
    command.args(rest).env(ENV_ENABLED, "1").env(ENV_WAIT, wait.to_string());
    for (name, value) in env {
        command.env(name, value);
    }
    match size {
        Some(size) => command.env(ENV_SIZE, size.to_string()),
        None => command.env_remove(ENV_SIZE),
    };
    let child = command.spawn().map_err(|error| format!("launching {program}: {error}"))?;
    Ok(App { child, name: program.clone() })
}

fn wait_for_shot(app: &mut App, dir: &Path, timeout: Duration) -> Result<Shot, String> {
    let start = Instant::now();
    loop {
        if let Some(shot) = Shot::read(dir).map_err(|error| format!("reading the shot: {error}"))? {
            return Ok(shot);
        }
        if let Some(status) = app.child.try_wait().map_err(|error| error.to_string())? {
            return Err(format!(
                "{} quit ({status}) before its shot; does it call slate::attach on its main window?",
                app.name
            ));
        }
        if start.elapsed() > timeout {
            return Err(format!(
                "no shot from {} after {:.1}s; does it call slate::attach on its main window?",
                app.name,
                timeout.as_secs_f32()
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The app's own render when it made one, else `screencapture -l`.
fn capture(shot: &Shot, dir: &Path) -> Result<(RgbaImage, Source), String> {
    if let Some(path) = &shot.image {
        let image = image::open(path).map_err(|error| format!("reading {}: {error}", path.display()))?;
        return Ok((image.into_rgba8(), Source::App));
    }
    let why = shot.render_error.clone().unwrap_or_else(|| "no reason given".into());
    let path = dir.join("screencapture.png");
    let output = Command::new("screencapture")
        .args(screencapture_args(shot.window_id, &path))
        .output()
        .map_err(|error| format!("running screencapture: {error}"))?;
    let image = image::open(&path).map_err(|error| {
        format!(
            "screencapture -l {} gave no image ({error}; {}). It needs Screen Recording permission for the terminal, \
             or build the app with slate's `render` feature",
            shot.window_id,
            String::from_utf8_lossy(&output.stderr).trim()
        )
    })?;
    Ok((image.into_rgba8(), Source::Screencapture { why }))
}

/// No sound (`-x`), no shadow (`-o`), just the window (`-l`).
fn screencapture_args(window_id: u32, path: &Path) -> Vec<String> {
    vec!["-x".into(), "-o".into(), "-l".into(), window_id.to_string(), path.display().to_string()]
}

/// Scales a device-pixel image back to the window's size in points.
pub(crate) fn to_points(image: RgbaImage, (width, height): (u32, u32)) -> RgbaImage {
    if image.dimensions() == (width, height) {
        return image;
    }
    image::imageops::resize(&image, width, height, FilterType::Lanczos3)
}

/// SIGTERM, then a kill if the app is still there after two seconds.
pub(crate) fn quit(child: &mut Child) {
    if matches!(child.try_wait(), Ok(Some(_))) {
        return;
    }
    let _ = Command::new("kill").arg("-TERM").arg(child.id().to_string()).status();
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(2) {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn screencapture_takes_just_the_window_silently_without_its_shadow() {
        assert_eq!(screencapture_args(77, Path::new("/tmp/w.png")), ["-x", "-o", "-l", "77", "/tmp/w.png"]);
    }

    #[test]
    fn scales_a_retina_capture_to_points() {
        let image = RgbaImage::from_pixel(2880, 1800, Rgba([10, 20, 30, 255]));
        let points = to_points(image, (1440, 900));
        assert_eq!(points.dimensions(), (1440, 900));
        assert_eq!(points.get_pixel(700, 450), &Rgba([10, 20, 30, 255]));
    }

    #[test]
    fn leaves_a_one_x_capture_alone() {
        let image = RgbaImage::from_pixel(4, 3, Rgba([1, 2, 3, 4]));
        assert_eq!(to_points(image.clone(), (4, 3)), image);
    }

    #[test]
    fn reads_the_app_render_when_there_is_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.png");
        RgbaImage::from_pixel(8, 6, Rgba([200, 0, 0, 255])).save(&path).unwrap();
        let shot = Shot { window_id: 1, scale: 2., width: 4., height: 3., image: Some(path), render_error: None };
        let (image, source) = capture(&shot, dir.path()).unwrap();
        assert_eq!(source, Source::App);
        assert_eq!(image.dimensions(), (8, 6));
    }

    #[test]
    fn an_app_that_quits_without_a_shot_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let child = Command::new("true").spawn().unwrap();
        let mut app = App { child, name: "true".into() };
        let problem = wait_for_shot(&mut app, dir.path(), Duration::from_secs(10)).unwrap_err();
        assert!(problem.contains("quit"), "{problem}");
    }

    #[test]
    fn an_app_that_never_shoots_times_out_and_is_quit() {
        let dir = tempfile::tempdir().unwrap();
        let child = Command::new("sleep").arg("30").spawn().unwrap();
        let mut app = App { child, name: "sleep".into() };
        let problem = wait_for_shot(&mut app, dir.path(), Duration::from_millis(200)).unwrap_err();
        assert!(problem.contains("no shot"), "{problem}");
        let start = Instant::now();
        quit(&mut app.child);
        assert!(app.child.try_wait().unwrap().is_some(), "quit leaves no app behind");
        assert!(start.elapsed() < Duration::from_secs(2), "sleep goes on SIGTERM");
    }
}
