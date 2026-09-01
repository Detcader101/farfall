//! The CONTROLS card: the essential keys on one screen, shown on the
//! first run (no settings file yet), on F1 any time, and at every start
//! if the pilot asks (ui.controls-card). Any key puts it away.
//!
//! A flat card in the menu's language — the header rule, the ivory
//! footnote — laid out here in font pixels under test: two columns, the
//! flight keys on the left, the view, the panels and the arms on the
//! right, every key read from the live bindings so a rebound key shows
//! as it is. The full list with what each control does is the menu's
//! HELP page; this is the card in the glovebox.

use crate::input::{key_name, Action, Bindings, Named};
use farfall_render::text::{block_height, block_width, TextBitmap, MENU_COLS};

/// The card's width in characters, and each column's.
pub const COLS: usize = MENU_COLS;
const LEFT_COLS: usize = 24;
/// The key column's width in each half.
const LEFT_KEY_COLS: usize = 10;
const RIGHT_KEY_COLS: usize = 7;

/// A line's left or right half: a heading, or a key and what it does.
enum Cell {
    Head(&'static str),
    Key(String, &'static str),
}

/// The card's key names are key caps: where the font has the symbol,
/// the key reads as the cap itself — the arrows as chevrons, the comma
/// and period as themselves. Spelled out they are wider than the key
/// column (the KEYS page, with room, spells them).
fn cap(name: &str) -> &str {
    match name {
        "LEFT" => "<",
        "RIGHT" => ">",
        "COMMA" => ",",
        "PERIOD" => ".",
        n => n,
    }
}

fn pair(b: &Bindings, a: Action, c: Action) -> String {
    format!(
        "{} {}",
        cap(key_name(b.key_for(a))),
        cap(key_name(b.key_for(c)))
    )
}

fn one(b: &Bindings, n: Named) -> String {
    cap(key_name(b.named(n))).to_string()
}

fn left(b: &Bindings) -> Vec<Cell> {
    vec![
        Cell::Head("FLIGHT"),
        Cell::Key(pair(b, Action::ThrustForward, Action::ThrustBack), "THRUST"),
        Cell::Key(pair(b, Action::StrafeLeft, Action::StrafeRight), "STRAFE"),
        Cell::Key(pair(b, Action::ThrustUp, Action::ThrustDown), "UP / DOWN"),
        Cell::Key(pair(b, Action::PitchUp, Action::PitchDown), "PITCH"),
        Cell::Key(pair(b, Action::YawLeft, Action::YawRight), "YAW"),
        Cell::Key(pair(b, Action::RollLeft, Action::RollRight), "ROLL"),
        Cell::Key(one(b, Named::Boost), "BOOST (HOLD)"),
        Cell::Key(one(b, Named::Brake), "AIR BRAKE"),
        Cell::Key(one(b, Named::Despin), "KILL SPIN"),
        Cell::Key(one(b, Named::Assist), "FLIGHT ASSIST"),
        Cell::Key(one(b, Named::Hold), "HOLD TARGET"),
        Cell::Head("DRIVES"),
        Cell::Key(one(b, Named::Hyper), "CHAOS DRIVE"),
        Cell::Key(one(b, Named::WarpStop), "WARP STOP"),
        Cell::Key(one(b, Named::Engage), "WORMHOLE"),
    ]
}

fn right(b: &Bindings) -> Vec<Cell> {
    vec![
        Cell::Head("VIEW"),
        Cell::Key(
            format!("RMB {}", one(b, Named::LookLock)),
            "LOOK HOLD / LOCK",
        ),
        Cell::Key(
            format!("{} {}", one(b, Named::Chase), one(b, Named::Holo)),
            "CHASE / HOLO3PP",
        ),
        Cell::Key(
            format!("{} {}", one(b, Named::HoloOut), one(b, Named::HoloIn)),
            "HOLOGRAM RANGE",
        ),
        Cell::Key(one(b, Named::Design), "DESIGN THE DASH"),
        Cell::Head("PANELS"),
        Cell::Key(
            format!("{} {}", one(b, Named::Map), one(b, Named::Bay)),
            "MAP / SHIP BAY",
        ),
        Cell::Key(one(b, Named::Landing), "LANDING MODE"),
        Cell::Key("ESC".to_string(), "SETTINGS MENU"),
        Cell::Head("ARMS"),
        Cell::Key("LMB".to_string(), "FIRE"),
        Cell::Key(
            format!(
                "{} {} {}",
                one(b, Named::Weapon1),
                one(b, Named::Weapon2),
                one(b, Named::NextWeapon)
            ),
            "CANNON RAIL NEXT",
        ),
        // The stick pilots the menus (crates/app/src/stick.rs has the
        // whole convention): the hat is the arrows, BASE L held is the
        // shift layer for combos, BASE R is ESC.
        Cell::Head("STICK"),
        Cell::Key("HAT".to_string(), "MENU ARROWS"),
        Cell::Key("TRIG".to_string(), "ENTER. BASE R ESC"),
        Cell::Key("BASE L".to_string(), "HOLD FOR COMBOS"),
    ]
}

/// A key cut a character short of its column: a pair of long names must
/// never run into the label — the gap survives the cut (the KEYS page
/// always shows the whole name).
fn cut(key: &str, cols: usize) -> String {
    key.chars().take(cols.saturating_sub(1)).collect()
}

fn cell_text(c: &Cell, key_cols: usize) -> String {
    match c {
        Cell::Head(h) => h.to_string(),
        Cell::Key(k, what) => format!("{:<w$}{what}", cut(k, key_cols), w = key_cols),
    }
}

/// The card's lines, top to bottom.
pub fn lines(b: &Bindings) -> Vec<String> {
    let mut out = vec!["FARFALL   CONTROLS".to_string(), String::new()];
    let (l, r) = (left(b), right(b));
    for i in 0..l.len().max(r.len()) {
        let lt = l
            .get(i)
            .map_or(String::new(), |c| cell_text(c, LEFT_KEY_COLS));
        let rt = r
            .get(i)
            .map_or(String::new(), |c| cell_text(c, RIGHT_KEY_COLS));
        let mut line = format!("{lt:<w$}{rt}", w = LEFT_COLS);
        line.truncate(line.trim_end().len());
        out.push(line);
    }
    out.push(String::new());
    out.push(format!(
        "{:<w$}F1 SHOWS THIS AGAIN",
        "PRESS ANY KEY TO FLY",
        w = LEFT_COLS
    ));
    out
}

/// Which of the card's lines are headings (for a brighter tint: none
/// yet; the columns read by their layout).
pub fn extent() -> (usize, usize) {
    (
        block_width(COLS),
        block_height(lines(&Bindings::default()).len()),
    )
}

/// The rules: under the title, over the footnote.
pub fn rules(b: &Bindings) -> [Option<f32>; 2] {
    let n = lines(b).len();
    let pitch = farfall_render::text::LINE as f32;
    [Some(pitch - 1.5), Some((n as f32 - 1.0) * pitch - 1.5)]
}

/// Draw the card into the bitmap.
pub fn render(text: &mut TextBitmap, b: &Bindings) {
    text.clear();
    for (i, line) in lines(b).iter().enumerate() {
        text.draw_line(0, i, line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use farfall_render::text::{has_glyph, ROWS};
    use winit::keyboard::KeyCode;

    /// Every line fits the card and the card fits the bitmap, with the
    /// stock keys and with the longest names bound everywhere.
    #[test]
    fn the_card_fits_with_any_binding() {
        let mut long = Bindings::default();
        for a in Action::ALL {
            long.bind(a, KeyCode::Backquote);
        }
        for n in Named::ALL {
            long.bind_named(n, KeyCode::NumpadEnter);
        }
        for b in [Bindings::default(), long] {
            let lines = lines(&b);
            for line in &lines {
                assert!(line.chars().count() <= COLS, "{line:?}");
                for c in line.chars() {
                    assert!(c == ' ' || has_glyph(c), "{c:?} in {line:?}");
                }
            }
            assert!(block_height(lines.len()) <= ROWS);
            let mut t = TextBitmap::new();
            render(&mut t, &b);
            let (w, h) = t.used_extent();
            let (ew, eh) = extent();
            assert!(w <= ew && h <= eh, "{w}x{h} in {ew}x{eh}");
        }
    }

    /// The card names the essentials — the thrust, attitude, the
    /// look, the map, the menu — and reads the live keys.
    #[test]
    fn the_card_shows_the_essentials_with_their_live_keys() {
        let text = lines(&Bindings::default()).join("\n");
        for word in [
            "THRUST",
            "PITCH",
            "ROLL",
            "BOOST",
            "LOOK HOLD",
            "MAP / SHIP BAY",
            "SETTINGS MENU",
            "FIRE",
            "CHAOS DRIVE",
            "PRESS ANY KEY",
            "F1",
            // The stick's menu convention, on the card in the glovebox.
            "STICK",
            "MENU ARROWS",
            "HOLD FOR COMBOS",
        ] {
            assert!(text.contains(word), "no {word} on the card");
        }
        assert!(text.contains("W S       THRUST"));
        let mut b = Bindings::default();
        b.bind(Action::ThrustForward, KeyCode::KeyI);
        assert!(lines(&b).join("\n").contains("I S       THRUST"));
        assert!(text.contains("M B    MAP / SHIP BAY"));
        let [top, bottom] = rules(&Bindings::default());
        assert!(top.unwrap() < bottom.unwrap());
    }

    /// A key never runs into its label. The stock wide names read as
    /// their key caps — the arrows as chevrons, the comma and period as
    /// themselves — and even a key as wide as its column keeps the gap
    /// (it once printed LEFT RIGHTYAW and COMMA PHOLOGRAM RANGE).
    #[test]
    fn a_key_never_runs_into_its_label() {
        let text = lines(&Bindings::default()).join("\n");
        assert!(text.contains("< >       YAW"), "{text}");
        assert!(text.contains(", .    HOLOGRAM RANGE"), "{text}");
        assert!(!text.contains("RIGHTYAW"), "{text}");
        assert!(!text.contains("PHOLOGRAM"), "{text}");
        // The net under every binding: a column-filling key is cut a
        // character short of the label, never flush against it.
        let c = cell_text(&Cell::Key("LEFT RIGHT".into(), "YAW"), 10);
        assert_eq!(c, "LEFT RIGH YAW");
    }
}
