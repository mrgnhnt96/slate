//! `slate drive`: launch the app hidden with a steps file, wait while it plays
//! them, then save each shot in points and each menu bar record.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use slate::protocol::{ENV_OUT, ENV_STEPS};
use slate::steps::{self, Drive};

use crate::DriveArgs;
use crate::shoot::{App, launch, to_points};

pub fn run(args: &DriveArgs) -> Result<(), String> {
    // Read the steps here too, so a bad file is reported before the app starts.
    let steps = steps::read(&args.steps)?;
    let steps_file =
        std::fs::canonicalize(&args.steps).map_err(|error| format!("reading {}: {error}", args.steps.display()))?;
    let dir = tempfile::Builder::new()
        .prefix("slate-drive-")
        .tempdir()
        .map_err(|error| format!("making a folder for the shots: {error}"))?;
    let mut app = launch(&args.app, args.size, args.wait, &[(ENV_STEPS, &steps_file), (ENV_OUT, dir.path())])?;
    let timeout = Duration::from_millis(args.wait + args.timeout) + steps::duration(&steps);
    let drive = wait_for_drive(&mut app, dir.path(), timeout)?;
    drop(app);
    std::fs::create_dir_all(&args.out).map_err(|error| format!("making {}: {error}", args.out.display()))?;
    for shot in &drive.shots {
        let Some(image) = &shot.shot.image else {
            continue;
        };
        let image = image::open(image).map_err(|error| format!("reading {}: {error}", image.display()))?;
        let image = to_points(image.into_rgba8(), shot.shot.points());
        let path = args.out.join(format!("{}.png", shot.name));
        image
            .save_with_format(&path, image::ImageFormat::Png)
            .map_err(|error| format!("writing {}: {error}", path.display()))?;
        println!("{} ({}x{} points)", path.display(), image.width(), image.height());
    }
    for path in copy_menus(&drive, dir.path(), &args.out)? {
        println!("{}", path.display());
    }
    match drive.error {
        Some(problem) => Err(format!("played {} of {} steps, then {problem}", drive.played, steps.len())),
        None => {
            println!("played {} steps", drive.played);
            Ok(())
        }
    }
}

/// Copies each menu bar record the app saved into `out`.
fn copy_menus(drive: &Drive, from: &Path, out: &Path) -> Result<Vec<PathBuf>, String> {
    let mut copied = Vec::new();
    for name in &drive.menus {
        let file = format!("{name}.json");
        let path = out.join(&file);
        std::fs::copy(from.join(&file), &path).map_err(|error| format!("copying {file}: {error}"))?;
        copied.push(path);
    }
    Ok(copied)
}

/// Waits for the app to write its drive and quit.
fn wait_for_drive(app: &mut App, dir: &Path, timeout: Duration) -> Result<Drive, String> {
    let start = Instant::now();
    loop {
        if let Some(drive) = Drive::read(dir).map_err(|error| format!("reading the drive: {error}"))? {
            return Ok(drive);
        }
        if let Some(status) = app.child.try_wait().map_err(|error| error.to_string())? {
            return Err(format!(
                "{} quit ({status}) without playing the steps; does it call slate::attach on its main window?",
                app.name
            ));
        }
        if start.elapsed() > timeout {
            return Err(format!(
                "{} didn't finish the steps in {:.1}s; does it call slate::attach on its main window?",
                app.name,
                timeout.as_secs_f32()
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn an_app_that_quits_without_driving_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App { child: Command::new("true").spawn().unwrap(), name: "true".into() };
        let problem = wait_for_drive(&mut app, dir.path(), Duration::from_secs(10)).unwrap_err();
        assert!(problem.contains("without playing"), "{problem}");
    }

    #[test]
    fn the_drive_is_read_once_the_app_writes_it() {
        let dir = tempfile::tempdir().unwrap();
        Drive { played: 3, shots: vec![], menus: vec![], error: None }.write(dir.path()).unwrap();
        let mut app = App { child: Command::new("sleep").arg("30").spawn().unwrap(), name: "sleep".into() };
        let drive = wait_for_drive(&mut app, dir.path(), Duration::from_secs(10)).unwrap();
        assert_eq!(drive.played, 3);
    }

    #[test]
    fn menu_bar_records_are_copied_out_by_name() {
        let from = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        std::fs::write(from.path().join("menu-bar.json"), "[]").unwrap();
        let drive = Drive { played: 1, shots: vec![], menus: vec!["menu-bar".into()], error: None };
        let copied = copy_menus(&drive, from.path(), out.path()).unwrap();
        assert_eq!(copied, vec![out.path().join("menu-bar.json")]);
        assert_eq!(std::fs::read_to_string(&copied[0]).unwrap(), "[]");
        let missing = Drive { menus: vec!["gone".into()], ..drive };
        assert!(copy_menus(&missing, from.path(), out.path()).unwrap_err().contains("gone.json"));
    }

    #[test]
    fn a_bad_steps_file_stops_before_the_app_starts() {
        let dir = tempfile::tempdir().unwrap();
        let steps = dir.path().join("steps.json");
        std::fs::write(&steps, r#"[{"tap": {"x": 1, "y": 2}}]"#).unwrap();
        let args = DriveArgs {
            steps,
            out: dir.path().join("out"),
            size: None,
            wait: 0,
            timeout: 100,
            app: vec!["/nonexistent/app".into()],
        };
        let problem = run(&args).unwrap_err();
        assert!(!problem.contains("launching"), "{problem}");
    }
}
