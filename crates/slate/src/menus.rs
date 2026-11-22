//! The app's menu bar as a `menus` step saves it, and the shortcuts written
//! the way macOS shows them. No GPUI or AppKit in here.
//!
//! An app under Slate is an accessory app, which macOS never gives the menu
//! bar, so its menus are never seen. They're still built: a `menus` step
//! reads them as AppKit holds them and a `menu` step chooses an item, as
//! though from the menu bar.

use serde::{Deserialize, Serialize};

/// One menu in the bar, or one item in a menu.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MenuItem {
    /// The title as shown. Empty for a separator.
    pub title: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub separator: bool,
    /// The keyboard shortcut as macOS shows it beside the title: `⇧⌘A`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shortcut: Option<String>,
    /// Whether it can be chosen now, as macOS would decide when the menu
    /// opens with the hidden window as the active one.
    pub enabled: bool,
    /// Whether it has a checkmark.
    #[serde(default, skip_serializing_if = "is_false")]
    pub checked: bool,
    /// The items of a menu or submenu.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<MenuItem>,
}

fn is_false(value: &bool) -> bool {
    !value
}

/// `NSEventModifierFlags` bits a key equivalent's mask uses.
pub const SHIFT: usize = 1 << 17;
pub const CONTROL: usize = 1 << 18;
pub const OPTION: usize = 1 << 19;
pub const COMMAND: usize = 1 << 20;

/// A key equivalent and its modifier mask as the menu shows them, modifiers
/// in macOS's order (⌃⌥⇧⌘) and then the key. None for an item with no key.
/// An upper-case letter implies Shift, as it does in AppKit.
pub fn shortcut(key: &str, mask: usize) -> Option<String> {
    let mut chars = key.chars();
    let first = chars.next()?;
    let single = chars.next().is_none();
    let shift = mask & SHIFT != 0 || (single && first.is_uppercase());
    let mut text = String::new();
    for (on, glyph) in [(mask & CONTROL != 0, '⌃'), (mask & OPTION != 0, '⌥'), (shift, '⇧'), (mask & COMMAND != 0, '⌘')] {
        if on {
            text.push(glyph);
        }
    }
    match single.then(|| key_glyph(first)).flatten() {
        Some(glyph) => text.push_str(glyph),
        None => text.extend(key.chars().flat_map(char::to_uppercase)),
    }
    Some(text)
}

/// How macOS writes the keys that aren't printed as themselves, from the
/// characters AppKit's key equivalents use for them.
fn key_glyph(key: char) -> Option<&'static str> {
    Some(match key {
        '\r' | '\n' | '\u{3}' => "↩",
        '\t' => "⇥",
        '\u{19}' => "⇤",
        '\u{1b}' => "⎋",
        '\u{8}' | '\u{7f}' => "⌫",
        ' ' => "Space",
        '\u{f700}' => "↑",
        '\u{f701}' => "↓",
        '\u{f702}' => "←",
        '\u{f703}' => "→",
        '\u{f728}' => "⌦",
        '\u{f729}' => "↖",
        '\u{f72b}' => "↘",
        '\u{f72c}' => "⇞",
        '\u{f72d}' => "⇟",
        '\u{f704}'..='\u{f70f}' => FUNCTION_KEYS[key as usize - 0xf704],
        _ => return None,
    })
}

const FUNCTION_KEYS: [&str; 12] = ["F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_read_the_way_the_menu_bar_shows_them() {
        assert_eq!(shortcut("", COMMAND), None);
        assert_eq!(shortcut("n", COMMAND).as_deref(), Some("⌘N"));
        assert_eq!(shortcut("a", COMMAND | SHIFT).as_deref(), Some("⇧⌘A"));
        assert_eq!(shortcut("s", CONTROL | COMMAND).as_deref(), Some("⌃⌘S"));
        assert_eq!(shortcut("\u{f703}", OPTION | COMMAND).as_deref(), Some("⌥⌘→"));
        assert_eq!(shortcut("\u{8}", COMMAND).as_deref(), Some("⌘⌫"));
        assert_eq!(shortcut("\r", COMMAND).as_deref(), Some("⌘↩"));
        assert_eq!(shortcut("r", 0).as_deref(), Some("R"));
        assert_eq!(shortcut("D", COMMAND).as_deref(), Some("⇧⌘D"), "an upper-case key means Shift");
        assert_eq!(shortcut(",", COMMAND).as_deref(), Some("⌘,"));
        assert_eq!(shortcut("\u{f705}", 0).as_deref(), Some("F2"));
    }

    #[test]
    fn a_menu_bar_writes_only_what_each_item_has() {
        let bar = vec![MenuItem {
            title: "View".into(),
            enabled: true,
            items: vec![
                MenuItem { title: "Hide Sidebar".into(), shortcut: Some("⌃⌘S".into()), enabled: true, ..MenuItem::default() },
                MenuItem { separator: true, ..MenuItem::default() },
                MenuItem { title: "Aurora Theme".into(), enabled: true, checked: true, ..MenuItem::default() },
            ],
            ..MenuItem::default()
        }];
        let text = serde_json::to_string(&bar).unwrap();
        assert_eq!(
            text,
            r#"[{"title":"View","enabled":true,"items":[{"title":"Hide Sidebar","shortcut":"⌃⌘S","enabled":true},{"title":"","separator":true,"enabled":false},{"title":"Aurora Theme","enabled":true,"checked":true}]}]"#
        );
        assert_eq!(serde_json::from_str::<Vec<MenuItem>>(&text).unwrap(), bar);
    }
}
