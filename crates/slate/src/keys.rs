//! Keys the way AppKit hands them to a native view (a web view the app hosts):
//! the characters typed, the characters without modifiers, the key code of a
//! US keyboard and the modifier flags. No GPUI or AppKit in here.

use crate::menus::{COMMAND, CONTROL, OPTION, SHIFT};
use crate::steps::Modifiers;

/// `NSEventModifierFlagFunction`.
const FUNCTION: usize = 1 << 23;

#[derive(Clone, Debug, PartialEq)]
pub struct AppKitKey {
    /// `NSEvent.characters`.
    pub characters: String,
    /// `NSEvent.charactersIgnoringModifiers`.
    pub ignoring: String,
    /// `NSEvent.keyCode`, the key on a US keyboard.
    pub code: u16,
    /// `NSEvent.modifierFlags`.
    pub flags: usize,
}

impl AppKitKey {
    /// Whether AppKit offers it as a key equivalent (to the window's views
    /// and the menu bar) before the view with the keyboard gets it.
    pub fn is_equivalent(&self) -> bool {
        self.flags & (COMMAND | CONTROL) != 0
    }
}

/// The key a key step names, by its GPUI name (`enter`, `left`, `a`) and the
/// modifiers held.
pub fn named(key: &str, modifiers: Modifiers) -> Result<AppKitKey, String> {
    let (base, code) = match special(key) {
        Some(special) => special,
        None => {
            let mut chars = key.chars();
            let (Some(base), None) = (chars.next(), chars.next()) else {
                return Err(format!("{key:?} isn't a key Slate can send to a web view"));
            };
            let base = base.to_ascii_lowercase();
            (base, code(base).ok_or_else(|| format!("{key:?} isn't on a US keyboard"))?)
        }
    };
    let ignoring = if modifiers.shift { shifted(base) } else { base };
    let characters = if modifiers.control && base.is_ascii_alphabetic() {
        char::from(base as u8 & 0x1f)
    } else if modifiers.platform {
        base
    } else {
        ignoring
    };
    Ok(AppKitKey { characters: characters.into(), ignoring: ignoring.into(), code, flags: flags(modifiers, base) })
}

/// The key that types `character`, with Shift when the keyboard needs it.
pub fn typed(character: char) -> AppKitKey {
    match character {
        '\n' => return named("enter", Modifiers::default()).expect("enter is a key"),
        '\t' => return named("tab", Modifiers::default()).expect("tab is a key"),
        _ => {}
    }
    let base = unshifted(character);
    let shift = base != character;
    let modifiers = Modifiers { shift, ..Modifiers::default() };
    AppKitKey {
        characters: character.into(),
        ignoring: character.into(),
        code: code(base).unwrap_or(0),
        flags: flags(modifiers, base),
    }
}

fn flags(modifiers: Modifiers, base: char) -> usize {
    let mut flags = 0;
    for (on, flag) in [
        (modifiers.shift, SHIFT),
        (modifiers.control, CONTROL),
        (modifiers.alt, OPTION),
        (modifiers.platform, COMMAND),
        (modifiers.function || ('\u{f700}'..='\u{f8ff}').contains(&base), FUNCTION),
    ] {
        if on {
            flags |= flag;
        }
    }
    flags
}

/// The keys GPUI names by word, as AppKit's characters and key codes.
fn special(key: &str) -> Option<(char, u16)> {
    Some(match key {
        "enter" => ('\r', 36),
        "tab" => ('\t', 48),
        "space" => (' ', 49),
        "backspace" => ('\u{7f}', 51),
        "escape" => ('\u{1b}', 53),
        "delete" => ('\u{f728}', 117),
        "left" => ('\u{f702}', 123),
        "right" => ('\u{f703}', 124),
        "down" => ('\u{f701}', 125),
        "up" => ('\u{f700}', 126),
        "home" => ('\u{f729}', 115),
        "end" => ('\u{f72b}', 119),
        "pageup" => ('\u{f72c}', 116),
        "pagedown" => ('\u{f72d}', 121),
        _ => {
            let number: usize = key.strip_prefix('f')?.parse().ok()?;
            let code = *FUNCTION_CODES.get(number.checked_sub(1)?)?;
            (char::from_u32(0xf704 + number as u32 - 1)?, code)
        }
    })
}

const FUNCTION_CODES: [u16; 12] = [122, 120, 99, 118, 96, 97, 98, 100, 101, 109, 103, 111];

/// The key code of an unshifted character on a US keyboard.
fn code(base: char) -> Option<u16> {
    const KEYS: &str = "asdfhgzxcv\0bqweryt123465=97-80]ou[ip\0lj'k;\\,/nm.\0 \0";
    if base == '`' {
        return Some(50);
    }
    KEYS.chars().position(|key| key == base && key != '\0').map(|index| index as u16)
}

const SHIFTED: [(char, char); 21] = [
    ('1', '!'),
    ('2', '@'),
    ('3', '#'),
    ('4', '$'),
    ('5', '%'),
    ('6', '^'),
    ('7', '&'),
    ('8', '*'),
    ('9', '('),
    ('0', ')'),
    ('-', '_'),
    ('=', '+'),
    ('[', '{'),
    (']', '}'),
    ('\\', '|'),
    (';', ':'),
    ('\'', '"'),
    (',', '<'),
    ('.', '>'),
    ('/', '?'),
    ('`', '~'),
];

fn shifted(base: char) -> char {
    if base.is_ascii_lowercase() {
        return base.to_ascii_uppercase();
    }
    SHIFTED.iter().find(|(plain, _)| *plain == base).map_or(base, |(_, shifted)| *shifted)
}

fn unshifted(character: char) -> char {
    if character.is_ascii_uppercase() {
        return character.to_ascii_lowercase();
    }
    SHIFTED.iter().find(|(_, shifted)| *shifted == character).map_or(character, |(plain, _)| *plain)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn held(text: &str) -> Modifiers {
        Modifiers::parse(text).unwrap()
    }

    #[test]
    fn named_keys_carry_appkit_characters_and_us_key_codes() {
        let a = named("a", Modifiers::default()).unwrap();
        assert_eq!((a.characters.as_str(), a.ignoring.as_str(), a.code, a.flags), ("a", "a", 0, 0));
        let enter = named("enter", Modifiers::default()).unwrap();
        assert_eq!((enter.characters.as_str(), enter.code), ("\r", 36));
        let left = named("left", held("shift")).unwrap();
        assert_eq!((left.characters.as_str(), left.code), ("\u{f702}", 123));
        assert_eq!(left.flags, SHIFT | FUNCTION, "arrows are function keys to AppKit");
        assert_eq!(named("f2", Modifiers::default()).unwrap().characters, "\u{f705}");
        assert_eq!(named("k", Modifiers::default()).unwrap().code, 40);
        assert_eq!(named("m", Modifiers::default()).unwrap().code, 46);
        assert_eq!(named("space", Modifiers::default()).unwrap().code, 49);
        assert!(named("hyper", Modifiers::default()).is_err());
    }

    #[test]
    fn modifiers_change_the_characters_the_way_appkit_does() {
        let select_all = named("a", held("cmd")).unwrap();
        assert_eq!((select_all.characters.as_str(), select_all.flags), ("a", COMMAND));
        assert!(select_all.is_equivalent());
        let capital = named("a", held("shift")).unwrap();
        assert_eq!((capital.characters.as_str(), capital.ignoring.as_str()), ("A", "A"));
        assert!(!capital.is_equivalent());
        let line_start = named("a", held("ctrl")).unwrap();
        assert_eq!((line_start.characters.as_str(), line_start.ignoring.as_str()), ("\u{1}", "a"));
        assert_eq!(named("1", held("shift")).unwrap().characters, "!");
    }

    #[test]
    fn typing_holds_shift_for_what_needs_it() {
        let q = typed('Q');
        assert_eq!((q.characters.as_str(), q.code, q.flags), ("Q", 12, SHIFT));
        let bang = typed('!');
        assert_eq!((bang.code, bang.flags), (18, SHIFT));
        assert_eq!(typed('é').characters, "é");
        assert_eq!(typed('\n').code, 36);
        assert_eq!(typed(' ').code, 49);
        assert_eq!(typed('.').flags, 0);
    }
}
