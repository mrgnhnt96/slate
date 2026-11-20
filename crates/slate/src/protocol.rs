//! What the `slate` command and the app say to each other: the environment
//! the command launches the app with, and the shot file the app writes back.
//!
//! This part has no GPUI in it, so the command uses it with
//! `default-features = false`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// `SLATE=1` turns Slate on: the window opens hidden and without focus.
pub const ENV_ENABLED: &str = "SLATE";
/// `SLATE_SIZE=1280x820`: the window's size in points.
pub const ENV_SIZE: &str = "SLATE_SIZE";
/// `SLATE_WAIT=1500`: milliseconds from opening the window to the shot.
pub const ENV_WAIT: &str = "SLATE_WAIT";
/// `SLATE_DIR=<folder>`: where the app writes [`SHOT_FILE`] (and
/// [`IMAGE_FILE`] when it can render the window itself). No folder, no shot.
pub const ENV_DIR: &str = "SLATE_DIR";
/// `SLATE_STEPS=<file.json>`: steps the app plays on its window, then quits.
/// See [`crate::steps`].
pub const ENV_STEPS: &str = "SLATE_STEPS";
/// `SLATE_OUT=<folder>`: where a drive's shots and its
/// [`DRIVE_FILE`](crate::steps::DRIVE_FILE) go. `SLATE_STEPS` needs it.
pub const ENV_OUT: &str = "SLATE_OUT";

/// The shot description the app writes into `SLATE_DIR` once the wait is up.
pub const SHOT_FILE: &str = "shot.json";
/// The window rendered by the app itself, in device pixels.
pub const IMAGE_FILE: &str = "shot.png";

/// How long the app waits after opening the window before the shot when
/// `SLATE_WAIT` isn't set.
pub const DEFAULT_WAIT: Duration = Duration::from_millis(1000);

/// A window size in points, written `WIDTHxHEIGHT`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

impl Size {
    /// Reads `1280x820` (an `X` works too). Both sides must be positive.
    pub fn parse(text: &str) -> Result<Self, String> {
        let bad = || format!("size {text:?} should look like 1280x820");
        let (width, height) = text.trim().split_once(['x', 'X']).ok_or_else(bad)?;
        let side = |value: &str| -> Result<f32, String> {
            let value: f32 = value.trim().parse().map_err(|_| bad())?;
            if value.is_finite() && value > 0. { Ok(value) } else { Err(bad()) }
        };
        Ok(Self { width: side(width)?, height: side(height)? })
    }
}

impl TryFrom<String> for Size {
    type Error = String;

    fn try_from(text: String) -> Result<Self, String> {
        Self::parse(&text)
    }
}

impl From<Size> for String {
    fn from(size: Size) -> Self {
        size.to_string()
    }
}

impl std::fmt::Display for Size {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}x{}", self.width, self.height)
    }
}

/// Slate's settings for this run of the app, read from the environment.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// `SLATE=1`.
    pub enabled: bool,
    /// The window size the command asked for, if any.
    pub size: Option<Size>,
    /// Time from opening the window to the shot.
    pub wait: Duration,
    /// Where the shot goes. None means the app runs hidden and takes no shot.
    pub dir: Option<PathBuf>,
    /// The steps file to play on the window. While there is one, the app
    /// plays it instead of taking the single shot `dir` asks for.
    pub steps: Option<PathBuf>,
    /// Where the drive's shots go.
    pub out: Option<PathBuf>,
}

impl Config {
    /// Reads this process's environment. A malformed `SLATE_SIZE` or
    /// `SLATE_WAIT` is reported on stderr and ignored, so the app still opens.
    pub fn from_env() -> Self {
        let (config, problems) = Self::from_vars(|key| std::env::var(key).ok());
        for problem in problems {
            eprintln!("slate: {problem}");
        }
        config
    }

    /// Reads the settings through `var`, returning what couldn't be read
    /// alongside. Slate is off unless `SLATE` is exactly `1`, and then the
    /// other variables are ignored.
    pub fn from_vars(var: impl Fn(&str) -> Option<String>) -> (Self, Vec<String>) {
        let mut problems = Vec::new();
        let enabled = var(ENV_ENABLED).is_some_and(|value| value.trim() == "1");
        let mut config = Self { enabled, size: None, wait: DEFAULT_WAIT, dir: None, steps: None, out: None };
        if !enabled {
            return (config, problems);
        }
        if let Some(text) = var(ENV_SIZE).filter(|text| !text.trim().is_empty()) {
            match Size::parse(&text) {
                Ok(size) => config.size = Some(size),
                Err(problem) => problems.push(format!("{ENV_SIZE}: {problem}")),
            }
        }
        if let Some(text) = var(ENV_WAIT).filter(|text| !text.trim().is_empty()) {
            match text.trim().parse::<u64>() {
                Ok(ms) => config.wait = Duration::from_millis(ms),
                Err(_) => problems.push(format!("{ENV_WAIT}: {text:?} should be milliseconds, like 1500")),
            }
        }
        let path = |key| var(key).filter(|text| !text.trim().is_empty()).map(PathBuf::from);
        config.dir = path(ENV_DIR);
        config.steps = path(ENV_STEPS);
        config.out = path(ENV_OUT);
        if config.steps.is_some() && config.out.is_none() {
            problems.push(format!("{ENV_STEPS} needs {ENV_OUT}, the folder for its shots; not playing the steps"));
            config.steps = None;
        }
        (config, problems)
    }
}

/// What the app tells the command about its window once the wait is up.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Shot {
    /// The CGWindowID (`NSWindow.windowNumber`), for `screencapture -l`.
    pub window_id: u32,
    /// Device pixels per point.
    pub scale: f32,
    /// The window's content size in points.
    pub width: f32,
    pub height: f32,
    /// The window rendered by the app itself, in device pixels, if it could.
    pub image: Option<PathBuf>,
    /// Why the app couldn't render the window itself, if it tried.
    pub render_error: Option<String>,
}

impl Shot {
    /// Writes the shot into `dir` all at once (a temporary file renamed into
    /// place), so the command never reads half of it.
    pub fn write(&self, dir: &Path) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        let partial = dir.join(format!("{SHOT_FILE}.partial"));
        std::fs::write(&partial, text)?;
        std::fs::rename(partial, dir.join(SHOT_FILE))
    }

    /// Reads the shot from `dir`: `Ok(None)` while the app hasn't written it.
    pub fn read(dir: &Path) -> std::io::Result<Option<Self>> {
        match std::fs::read_to_string(dir.join(SHOT_FILE)) {
            Ok(text) => serde_json::from_str(&text).map(Some).map_err(std::io::Error::other),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// The image size in whole points: what the shot is scaled back to.
    pub fn points(&self) -> (u32, u32) {
        (self.width.round().max(1.) as u32, self.height.round().max(1.) as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn config(vars: &[(&str, &str)]) -> (Config, Vec<String>) {
        let vars: HashMap<String, String> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        Config::from_vars(|key| vars.get(key).cloned())
    }

    #[test]
    fn size_reads_width_by_height() {
        assert_eq!(Size::parse("1280x820"), Ok(Size { width: 1280., height: 820. }));
        assert_eq!(Size::parse(" 390X844 "), Ok(Size { width: 390., height: 844. }));
        assert_eq!(Size::parse("1440.5x900"), Ok(Size { width: 1440.5, height: 900. }));
    }

    #[test]
    fn size_refuses_what_is_not_a_size() {
        for text in ["", "1280", "1280x", "x820", "0x820", "1280x-1", "wide x tall", "NaNx3", "infx3"] {
            assert!(Size::parse(text).is_err(), "{text:?} should not parse");
        }
    }

    #[test]
    fn size_prints_the_way_it_reads() {
        let size = Size { width: 1280., height: 820. };
        assert_eq!(Size::parse(&size.to_string()), Ok(size));
    }

    #[test]
    fn off_unless_slate_is_one() {
        for value in ["", "0", "true", "yes"] {
            let (config, _) = config(&[(ENV_ENABLED, value), (ENV_SIZE, "10x10"), (ENV_DIR, "/tmp/x")]);
            assert!(!config.enabled, "SLATE={value:?}");
            assert_eq!(config.size, None, "other variables don't count while off");
            assert_eq!(config.dir, None);
        }
        assert!(!config(&[]).0.enabled);
    }

    #[test]
    fn reads_size_wait_and_dir_when_on() {
        let (config, problems) =
            config(&[(ENV_ENABLED, "1"), (ENV_SIZE, "1440x900"), (ENV_WAIT, "2500"), (ENV_DIR, "/tmp/shot")]);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(
            config,
            Config {
                enabled: true,
                size: Some(Size { width: 1440., height: 900. }),
                wait: Duration::from_millis(2500),
                dir: Some("/tmp/shot".into()),
                steps: None,
                out: None,
            }
        );
    }

    #[test]
    fn defaults_when_only_switched_on() {
        let (config, problems) = config(&[(ENV_ENABLED, "1")]);
        assert!(problems.is_empty());
        assert_eq!(config, Config { enabled: true, size: None, wait: DEFAULT_WAIT, dir: None, steps: None, out: None });
    }

    #[test]
    fn reports_bad_values_and_keeps_the_defaults() {
        let (config, problems) = config(&[(ENV_ENABLED, "1"), (ENV_SIZE, "big"), (ENV_WAIT, "soon")]);
        assert_eq!(config.size, None);
        assert_eq!(config.wait, DEFAULT_WAIT);
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems[0].starts_with(ENV_SIZE));
        assert!(problems[1].starts_with(ENV_WAIT));
    }

    #[test]
    fn reads_steps_and_their_out_folder() {
        let (config, problems) = config(&[(ENV_ENABLED, "1"), (ENV_STEPS, "/tmp/steps.json"), (ENV_OUT, "/tmp/out")]);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(config.steps, Some("/tmp/steps.json".into()));
        assert_eq!(config.out, Some("/tmp/out".into()));
    }

    #[test]
    fn steps_without_an_out_folder_are_reported_and_dropped() {
        let (config, problems) = config(&[(ENV_ENABLED, "1"), (ENV_STEPS, "/tmp/steps.json")]);
        assert_eq!(config.steps, None);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains(ENV_OUT), "{problems:?}");
    }

    #[test]
    fn size_reads_from_json_as_a_string() {
        let size: Size = serde_json::from_str("\"390x844\"").unwrap();
        assert_eq!(size, Size { width: 390., height: 844. });
        assert_eq!(serde_json::to_string(&size).unwrap(), "\"390x844\"");
        assert!(serde_json::from_str::<Size>("\"tall\"").is_err());
    }

    #[test]
    fn shot_round_trips_through_its_folder() {
        let dir = std::env::temp_dir().join(format!("slate-shot-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(Shot::read(&dir).unwrap(), None, "nothing written yet");
        let shot = Shot {
            window_id: 4242,
            scale: 2.,
            width: 1280.,
            height: 820.,
            image: Some(dir.join(IMAGE_FILE)),
            render_error: None,
        };
        shot.write(&dir).unwrap();
        assert_eq!(Shot::read(&dir).unwrap(), Some(shot));
        assert!(!dir.join(format!("{SHOT_FILE}.partial")).exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn points_round_to_whole_pixels() {
        let shot = Shot { window_id: 1, scale: 2., width: 1279.6, height: 0.2, image: None, render_error: None };
        assert_eq!(shot.points(), (1280, 1));
    }
}
