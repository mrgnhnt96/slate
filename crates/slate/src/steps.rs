//! The steps an app plays on its own hidden window when launched with
//! `SLATE_STEPS=<file.json>`, and the record it writes back once they're
//! played. No GPUI in here, so the `slate` command can check a steps file
//! before it launches anything.
//!
//! A steps file is a JSON list. Positions are in window points, from the
//! window's top left corner (the title bar counts when the app draws under it):
//!
//! ```json
//! [
//!   { "wait": 300 },
//!   { "click": { "x": 120, "y": 160 } },
//!   { "type": "hello" },
//!   { "key": "cmd-k" },
//!   { "key": "down down enter" },
//!   { "move": { "x": 400, "y": 200 } },
//!   { "scroll": { "x": 400, "y": 300, "dy": 240 } },
//!   { "drag": { "from": { "x": 320, "y": 400 }, "to": { "x": 480, "y": 400 } } },
//!   { "menu": "View > Hide Sidebar" },
//!   { "menus": "menu-bar" },
//!   { "resize": "1280x820" },
//!   { "shot": "after-search" }
//! ]
//! ```
//!
//! Clicks, drags and scrolls over a native view the app laid over its GPUI
//! view (a `wry` WKWebView) go to that view as AppKit events, and keys and
//! typing go to it while it has the keyboard, the way they would for a person.

use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::protocol::{Shot, Size};

/// The record of a drive the app writes into `SLATE_OUT` once it has played
/// the steps (or stopped at one that failed), just before it quits.
pub const DRIVE_FILE: &str = "drive.json";

/// The pause after every step, for the app to handle it and draw.
pub const SETTLE: Duration = Duration::from_millis(50);

/// One thing to do to the window.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
pub enum Step {
    /// Keystrokes in GPUI's keybinding syntax, separated by spaces:
    /// `"cmd-k"`, `"j"`, `"shift-tab enter"`. Each goes through the app's
    /// keymap first, the way a key press does, and is typed into the focused
    /// text field when nothing handles it.
    Key(String),
    /// Text typed into the focused element one character at a time, as if from
    /// the keyboard. Characters bound to actions (a `j` in a list) run them.
    Type(String),
    /// A mouse click (press and release) at a point in the window.
    Click(Click),
    /// The mouse moving to a point in the window, for hover states.
    Move(Point),
    /// A scroll wheel turn over a point in the window.
    Scroll(Scroll),
    /// The left button pressed at one point, the pointer moved to another
    /// with it held, then released there.
    Drag(Drag),
    /// Chooses an item in the app's menu bar, by its menus' titles and its
    /// own: `"View > Hide Sidebar"`. Fails when it's disabled.
    Menu(String),
    /// Saves the app's menu bar as it would show now (titles, shortcuts,
    /// checkmarks, disabled items) as `<name>.json`.
    Menus(String),
    /// A pause in milliseconds, for the app to load or animate.
    Wait(u64),
    /// The window's new content size in points, `"WIDTHxHEIGHT"`.
    Resize(Size),
    /// A PNG of the window as it is now, saved as `<name>.png`.
    Shot(String),
}

/// A point in the window, in points from its top left corner.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Click {
    pub x: f32,
    pub y: f32,
    /// `left` (the default), `right` or `middle`.
    #[serde(default)]
    pub button: Button,
    /// 2 for a double click, 3 for a triple click.
    #[serde(default = "one")]
    pub count: u32,
    /// Modifier keys held during the click, like `"cmd"` or `"cmd-shift"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modifiers: Option<String>,
}

fn one() -> u32 {
    1
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Button {
    #[default]
    Left,
    Right,
    Middle,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scroll {
    /// Where the mouse is while it scrolls.
    pub x: f32,
    pub y: f32,
    /// Points to scroll right (negative: left).
    #[serde(default)]
    pub dx: f32,
    /// Points to scroll down the content (negative: up).
    #[serde(default)]
    pub dy: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Drag {
    pub from: Point,
    pub to: Point,
    /// How many pointer moves it takes from `from` to `to`, each drawn
    /// before the next, so drag handlers see it go.
    #[serde(default = "default_moves")]
    pub moves: u32,
    /// Modifier keys held through the drag, like `"alt"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modifiers: Option<String>,
}

/// The pointer moves in a drag unless the step says how many.
pub const DRAG_MOVES: u32 = 10;

/// The pause between a drag's moves, for the app to draw each one.
pub const DRAG_FRAME: Duration = Duration::from_millis(16);

fn default_moves() -> u32 {
    DRAG_MOVES
}

impl Drag {
    /// Where the pointer is after move `index` of `moves` (1 to `moves`).
    pub fn at(&self, index: u32) -> Point {
        let t = index as f32 / self.moves as f32;
        Point { x: self.from.x + (self.to.x - self.from.x) * t, y: self.from.y + (self.to.y - self.from.y) * t }
    }
}

/// A menu step's path: `"View > Hide Sidebar"` is `["View", "Hide Sidebar"]`.
/// A menu and at least one item in it.
pub fn menu_path(text: &str) -> Result<Vec<&str>, String> {
    let path: Vec<&str> = text.split('>').map(str::trim).collect();
    if path.len() < 2 || path.iter().any(|title| title.is_empty()) {
        return Err(format!("{text:?} should name a menu and an item in it, like \"View > Hide Sidebar\""));
    }
    Ok(path)
}

/// Modifier keys, read from `"cmd-shift"`. `cmd` is also `platform` or
/// `super`, `alt` is also `option`, and `ctrl` is also `control`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub control: bool,
    pub alt: bool,
    pub shift: bool,
    pub platform: bool,
    pub function: bool,
}

impl Modifiers {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut modifiers = Self::default();
        for name in text.split(['-', '+']).map(str::trim).filter(|name| !name.is_empty()) {
            let flag = match name.to_ascii_lowercase().as_str() {
                "cmd" | "command" | "platform" | "super" => &mut modifiers.platform,
                "ctrl" | "control" => &mut modifiers.control,
                "alt" | "option" | "opt" => &mut modifiers.alt,
                "shift" => &mut modifiers.shift,
                "fn" | "function" => &mut modifiers.function,
                _ => return Err(format!("{name:?} in {text:?} isn't a modifier (cmd, ctrl, alt, shift, fn)")),
            };
            *flag = true;
        }
        Ok(modifiers)
    }
}

impl Step {
    /// How the step reads in messages: `step 3 (key "cmd-k")`.
    pub fn describe(&self) -> String {
        match self {
            Step::Key(keys) => format!("key {keys:?}"),
            Step::Type(text) => format!("type {text:?}"),
            Step::Click(click) => format!("click at {},{}", click.x, click.y),
            Step::Move(point) => format!("move to {},{}", point.x, point.y),
            Step::Scroll(scroll) => format!("scroll {},{} at {},{}", scroll.dx, scroll.dy, scroll.x, scroll.y),
            Step::Drag(drag) => format!("drag {},{} to {},{}", drag.from.x, drag.from.y, drag.to.x, drag.to.y),
            Step::Menu(path) => format!("menu {path:?}"),
            Step::Menus(name) => format!("menus {name:?}"),
            Step::Wait(ms) => format!("wait {ms}ms"),
            Step::Resize(size) => format!("resize to {size}"),
            Step::Shot(name) => format!("shot {name:?}"),
        }
    }

    /// Time the step takes on its own, before the settle every step gets.
    pub fn duration(&self) -> Duration {
        match self {
            Step::Wait(ms) => Duration::from_millis(*ms),
            Step::Drag(drag) => DRAG_FRAME * (drag.moves + 1),
            _ => Duration::ZERO,
        }
    }

    fn check(&self) -> Result<(), String> {
        let finite = |values: &[f32]| values.iter().all(|value| value.is_finite());
        match self {
            Step::Key(keys) if keys.split_whitespace().next().is_none() => Err("no keys to press".into()),
            Step::Type(text) if text.is_empty() => Err("no text to type".into()),
            Step::Click(click) => {
                if !finite(&[click.x, click.y]) {
                    return Err("the point isn't a number".into());
                }
                if !(1..=3).contains(&click.count) {
                    return Err(format!("count {} should be 1, 2 or 3", click.count));
                }
                click.modifiers.as_deref().map(Modifiers::parse).transpose().map(drop)
            }
            Step::Move(point) if !finite(&[point.x, point.y]) => Err("the point isn't a number".into()),
            Step::Scroll(scroll) if !finite(&[scroll.x, scroll.y, scroll.dx, scroll.dy]) => {
                Err("the point or distance isn't a number".into())
            }
            Step::Drag(drag) => {
                if !(1..=100).contains(&drag.moves) {
                    return Err(format!("moves {} should be 1 to 100", drag.moves));
                }
                drag.modifiers.as_deref().map(Modifiers::parse).transpose().map(drop)
            }
            Step::Menu(path) => menu_path(path).map(drop),
            Step::Shot(name) | Step::Menus(name) => check_shot_name(name),
            _ => Ok(()),
        }
    }
}

/// A shot's name becomes a file name, so it can't climb out of the folder.
fn check_shot_name(name: &str) -> Result<(), String> {
    let fine = |c: char| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | ' ');
    if name.is_empty() || name.starts_with('.') || !name.chars().all(fine) {
        return Err(format!(
            "shot name {name:?} should be letters, digits, '-', '_', '.' or spaces, not starting with '.'"
        ));
    }
    Ok(())
}

/// The steps from a steps file, checked: every shot and menu bar record has
/// a name of its own.
pub fn parse(text: &str) -> Result<Vec<Step>, String> {
    let steps: Vec<Step> = serde_json::from_str(text).map_err(|error| format!("not a list of steps: {error}"))?;
    let mut names = HashSet::new();
    for (index, step) in steps.iter().enumerate() {
        step.check().map_err(|problem| format!("step {} ({}): {problem}", index + 1, step.describe()))?;
        if let Step::Shot(name) | Step::Menus(name) = step
            && !names.insert(name.as_str())
        {
            return Err(format!("step {}: there's already a shot or menus called {name:?}", index + 1));
        }
    }
    Ok(steps)
}

/// Reads and checks a steps file.
pub fn read(path: &Path) -> Result<Vec<Step>, String> {
    let text = std::fs::read_to_string(path).map_err(|error| format!("reading {}: {error}", path.display()))?;
    parse(&text).map_err(|problem| format!("{}: {problem}", path.display()))
}

/// How long the steps take to play, waits and settles included.
pub fn duration(steps: &[Step]) -> Duration {
    steps.iter().map(|step| step.duration() + SETTLE).sum()
}

/// One shot the app took during the drive.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DriveShot {
    /// The name from the steps file.
    pub name: String,
    #[serde(flatten)]
    pub shot: Shot,
}

/// What the app writes into `SLATE_OUT` when the drive is over.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Drive {
    /// How many steps were played to the end.
    pub played: usize,
    /// The shots taken, in order.
    pub shots: Vec<DriveShot>,
    /// The names of the menu bar records saved (`<name>.json`), in order.
    #[serde(default)]
    pub menus: Vec<String>,
    /// Why the drive stopped early, if it did.
    pub error: Option<String>,
}

impl Drive {
    /// Writes the record all at once, so the command never reads half of it.
    pub fn write(&self, dir: &Path) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        let partial = dir.join(format!("{DRIVE_FILE}.partial"));
        std::fs::write(&partial, text)?;
        std::fs::rename(partial, dir.join(DRIVE_FILE))
    }

    /// Reads the record from `dir`: `Ok(None)` while the drive is still going.
    pub fn read(dir: &Path) -> std::io::Result<Option<Self>> {
        match std::fs::read_to_string(dir.join(DRIVE_FILE)) {
            Ok(text) => serde_json::from_str(&text).map(Some).map_err(std::io::Error::other),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_every_kind_of_step() {
        let steps = parse(
            r#"[
                {"key": "cmd-k"},
                {"type": "hello"},
                {"click": {"x": 10, "y": 20.5}},
                {"click": {"x": 1, "y": 2, "button": "right", "count": 2, "modifiers": "cmd-shift"}},
                {"move": {"x": 3, "y": 4}},
                {"scroll": {"x": 5, "y": 6, "dy": 120}},
                {"wait": 250},
                {"resize": "1280x820"},
                {"shot": "after typing"},
                {"drag": {"from": {"x": 300, "y": 400}, "to": {"x": 420, "y": 400}}},
                {"drag": {"from": {"x": 1, "y": 2}, "to": {"x": 3, "y": 4}, "moves": 4, "modifiers": "alt"}},
                {"menu": "View > Hide Sidebar"},
                {"menus": "menu bar"}
            ]"#,
        )
        .unwrap();
        assert_eq!(
            steps,
            [
                Step::Key("cmd-k".into()),
                Step::Type("hello".into()),
                Step::Click(Click { x: 10., y: 20.5, button: Button::Left, count: 1, modifiers: None }),
                Step::Click(Click {
                    x: 1.,
                    y: 2.,
                    button: Button::Right,
                    count: 2,
                    modifiers: Some("cmd-shift".into())
                }),
                Step::Move(Point { x: 3., y: 4. }),
                Step::Scroll(Scroll { x: 5., y: 6., dx: 0., dy: 120. }),
                Step::Wait(250),
                Step::Resize(Size { width: 1280., height: 820. }),
                Step::Shot("after typing".into()),
                Step::Drag(Drag {
                    from: Point { x: 300., y: 400. },
                    to: Point { x: 420., y: 400. },
                    moves: DRAG_MOVES,
                    modifiers: None
                }),
                Step::Drag(Drag {
                    from: Point { x: 1., y: 2. },
                    to: Point { x: 3., y: 4. },
                    moves: 4,
                    modifiers: Some("alt".into())
                }),
                Step::Menu("View > Hide Sidebar".into()),
                Step::Menus("menu bar".into()),
            ]
        );
    }

    #[test]
    fn a_drag_moves_evenly_and_ends_where_it_says() {
        let drag = Drag { from: Point { x: 100., y: 50. }, to: Point { x: 200., y: 30. }, moves: 4, modifiers: None };
        assert_eq!(drag.at(1), Point { x: 125., y: 45. });
        assert_eq!(drag.at(4), drag.to);
        assert_eq!(Step::Drag(drag).duration(), DRAG_FRAME * 5);
    }

    #[test]
    fn a_menu_path_names_a_menu_and_an_item() {
        assert_eq!(menu_path("View > Hide Sidebar").unwrap(), ["View", "Hide Sidebar"]);
        assert_eq!(menu_path("File>Open Recent > notes.md").unwrap(), ["File", "Open Recent", "notes.md"]);
        assert!(menu_path("View").is_err());
        assert!(menu_path("View > ").is_err());
    }

    #[test]
    fn refuses_steps_it_cannot_play() {
        for (text, wanted) in [
            (r#"{"key": "j"}"#, "not a list"),
            (r#"[{"press": "j"}]"#, "unknown variant"),
            (r#"[{"click": {"x": 1}}]"#, "missing field"),
            (r#"[{"click": {"x": 1, "y": 2, "z": 3}}]"#, "unknown field"),
            (r#"[{"resize": "big"}]"#, "1280x820"),
            (r#"[{"key": "  "}]"#, "step 1 (key"),
            (r#"[{"type": ""}]"#, "no text"),
            (r#"[{"click": {"x": 1, "y": 2, "count": 4}}]"#, "count 4"),
            (r#"[{"click": {"x": 1, "y": 2, "modifiers": "hyper"}}]"#, "isn't a modifier"),
            (r#"[{"shot": "../escape"}]"#, "shot name"),
            (r#"[{"shot": ""}]"#, "shot name"),
            (r#"[{"shot": "a"}, {"wait": 1}, {"shot": "a"}]"#, "step 3: there's already"),
            (r#"[{"shot": "a"}, {"menus": "a"}]"#, "step 2: there's already"),
            (r#"[{"menus": "../bar"}]"#, "shot name"),
            (r#"[{"menu": "Quit"}]"#, "should name a menu and an item"),
            (r#"[{"drag": {"from": {"x": 1, "y": 2}}}]"#, "missing field"),
            (r#"[{"drag": {"from": {"x": 1, "y": 2}, "to": {"x": 3, "y": 4}, "moves": 0}}]"#, "moves 0"),
            (r#"[{"drag": {"from": {"x": 1, "y": 2}, "to": {"x": 3, "y": 4}, "modifiers": "meta"}}]"#, "isn't a modifier"),
        ] {
            let problem = parse(text).unwrap_err();
            assert!(problem.contains(wanted), "{text}: {problem}");
        }
    }

    #[test]
    fn modifiers_read_their_usual_names() {
        assert_eq!(
            Modifiers::parse("cmd-shift").unwrap(),
            Modifiers { platform: true, shift: true, ..Default::default() }
        );
        assert_eq!(
            Modifiers::parse("ctrl+option+fn").unwrap(),
            Modifiers { control: true, alt: true, function: true, ..Default::default() }
        );
        assert_eq!(Modifiers::parse("").unwrap(), Modifiers::default());
    }

    #[test]
    fn duration_counts_waits_and_a_settle_per_step() {
        let steps = [Step::Wait(200), Step::Key("j".into()), Step::Wait(100)];
        assert_eq!(duration(&steps), Duration::from_millis(300) + SETTLE * 3);
    }

    #[test]
    fn drive_round_trips_through_its_folder() {
        let dir = std::env::temp_dir().join(format!("slate-drive-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(Drive::read(&dir).unwrap(), None);
        let drive = Drive {
            played: 3,
            shots: vec![DriveShot {
                name: "typed".into(),
                shot: Shot {
                    window_id: 9,
                    scale: 2.,
                    width: 800.,
                    height: 500.,
                    image: Some(dir.join("typed.png")),
                    render_error: None,
                },
            }],
            menus: vec!["menu-bar".into()],
            error: Some("step 4 (key \"cmd-?\"): bad".into()),
        };
        drive.write(&dir).unwrap();
        assert_eq!(Drive::read(&dir).unwrap(), Some(drive));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
