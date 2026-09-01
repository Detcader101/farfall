//! A cockpit layout as a small file a player can send to another player:
//! every `ui.*` key (slots, dragged anchors, per-dial size / style /
//! fade / tilt / lean / rotate, the safe edge, the panel places), the
//! `holo.*` knobs and the mini map's look — in the settings file's own
//! `key = value` form, with a header naming the game and a format
//! version. Saved under `~/.farfall/huds/hud-<n>.fhud`. Sharing IS the
//! file: send it, the other player drops it in that folder and LOAD HUD
//! on the DIALS page wears it (or `FARFALL_HUD=path` wears one for a
//! run). Nothing of the machine (graphics), the hands (controls) or the
//! world (arms, mimics, the plan) rides along.

use crate::settings::{home_dir, Settings};
use std::path::{Path, PathBuf};

/// The .fhud format's version, written as `hud.version`.
pub const HUD_VERSION: u32 = 1;

/// The keys a HUD file owns: the whole look of the glass.
fn is_hud_key(k: &str) -> bool {
    k.starts_with("ui.") || k.starts_with("holo.") || k == "map.rings" || k == "map.grid"
}

fn is_hud_line(line: &str) -> bool {
    line.split_once('=')
        .is_some_and(|(k, _)| is_hud_key(k.trim()))
}

/// Where the HUD files live.
pub fn dir() -> Option<PathBuf> {
    home_dir().map(|h| h.join(".farfall").join("huds"))
}

/// The saved HUD files, hud-1 before hud-10, strangers' names after.
pub fn list() -> Vec<PathBuf> {
    dir().map(|d| list_in(&d)).unwrap_or_default()
}

fn list_in(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|it| {
            it.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "fhud"))
                .collect()
        })
        .unwrap_or_default();
    v.sort_by_key(|p| {
        (
            slot_of(p).unwrap_or(u32::MAX),
            p.file_name().map(|n| n.to_os_string()),
        )
    });
    v
}

/// The number in a `hud-<n>.fhud` name, if it has one.
pub fn slot_of(p: &Path) -> Option<u32> {
    p.file_stem()?.to_str()?.strip_prefix("hud-")?.parse().ok()
}

/// The file's text for this cockpit.
pub fn render(s: &Settings) -> String {
    let mut out = String::from(
        "# FARFALL HUD layout — send this file; it goes in ~/.farfall/huds/ \
         and LOAD HUD (DIALS page) wears it\n",
    );
    out.push_str(&format!("hud.version = {HUD_VERSION}\n"));
    for line in s.render().lines() {
        if is_hud_line(line) {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Wear a HUD file over these settings: every HUD-owned key starts from
/// stock and takes the file's word — a key the file omits means stock,
/// not "whatever you had" — and everything else (graphics, controls,
/// the world) is untouched. An empty text is the stock cockpit.
pub fn apply(base: &Settings, text: &str) -> Settings {
    let mut merged = String::new();
    for line in base.render().lines() {
        if !is_hud_line(line) {
            merged.push_str(line);
            merged.push('\n');
        }
    }
    for line in text.lines() {
        if is_hud_line(line.trim()) {
            merged.push_str(line.trim());
            merged.push('\n');
        }
    }
    Settings::parse(&merged)
}

/// Save the cockpit: over `hud-<slot>` when given (the one worn last),
/// else into the first free number. Returns the slot and the path.
pub fn save(s: &Settings, slot: Option<u32>) -> Option<(u32, PathBuf)> {
    let d = dir()?;
    let _ = std::fs::create_dir_all(&d);
    let n = slot.unwrap_or_else(|| {
        let taken: Vec<u32> = list_in(&d).iter().filter_map(|p| slot_of(p)).collect();
        (1..).find(|n| !taken.contains(n)).unwrap_or(1)
    });
    let path = d.join(format!("hud-{n}.fhud"));
    std::fs::write(&path, render(s)).ok()?;
    Some((n, path))
}

/// Wear the pick'th saved file (1-based, in [`list`]'s order) over
/// these settings. Returns its slot number (0 for a stranger's name).
pub fn load(base: &Settings, pick: usize) -> Option<(u32, Settings)> {
    let path = list().into_iter().nth(pick.checked_sub(1)?)?;
    let text = std::fs::read_to_string(&path).ok()?;
    Some((slot_of(&path).unwrap_or(0), apply(base, &text)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cockpit::{Instrument, Slot};
    use crate::input::Action;
    use winit::keyboard::KeyCode;

    /// A cockpit somebody actually laid out.
    fn worn() -> Settings {
        let mut s = Settings::default();
        s.dials[Instrument::Speed as usize].lean_deg = 25.0;
        s.dials[Instrument::Speed as usize].rotate_deg = -90.0;
        s.dials[Instrument::Map as usize].size = 1.5;
        s.layout.set(Instrument::Gyro, Slot::TopCentre);
        s.layout.set_free(Instrument::Map, [0.1, -0.4]);
        s.holo_size = 0.3;
        s.holo_anchor = [-0.5, 0.2];
        s.readout_anchor = [0.4, 0.4];
        s.layout.set_safe_edge(0.05);
        s.map_rings = 2;
        s
    }

    /// Export → wipe → import on a stranger's settings: the cockpit
    /// comes back identical, and nothing outside the HUD's keys moves.
    #[test]
    fn a_hud_file_round_trips_the_whole_cockpit_and_nothing_else() {
        let src = worn();
        let file = render(&src);
        assert!(file.starts_with("# FARFALL"), "self-describing: {file}");
        assert!(file.contains(&format!("hud.version = {HUD_VERSION}\n")));
        assert!(file.contains("ui.speed.lean = 25\n"));
        assert!(file.contains("ui.map = on at 0.100,-0.400\n"));
        assert!(!file.contains("graphics."), "a HUD carries no graphics");
        assert!(!file.contains("control."), "a HUD carries no binds");
        assert!(!file.contains("arms."), "a HUD carries no world");
        // The stranger: their own binds and graphics, a stock cockpit.
        let mut base = Settings::default();
        base.bindings.bind(Action::PitchUp, KeyCode::KeyI);
        base.msaa = 8;
        let worn_now = apply(&base, &file);
        assert_eq!(render(&worn_now), file, "the layout came across whole");
        assert_eq!(worn_now.msaa, 8, "their graphics stay theirs");
        assert_eq!(
            worn_now.bindings.key_for(Action::PitchUp),
            KeyCode::KeyI,
            "their binds stay theirs"
        );
        // Wipe: DEFAULT is the stock cockpit, still their machine.
        let wiped = apply(&worn_now, "");
        assert_eq!(render(&wiped), render(&Settings::default()));
        assert_eq!(wiped.msaa, 8);
        // A key the file omits means stock: a one-line file leans the
        // speed dial and resets everything else the HUD owns.
        let sparse = apply(&worn_now, "ui.speed.lean = 10\n");
        assert_eq!(sparse.dials[Instrument::Speed as usize].lean_deg, 10.0);
        assert_eq!(sparse.holo_size, Settings::default().holo_size);
        // Junk in a shared file is ignored, like the settings file's.
        let junk = apply(&base, "nonsense\nui.speed.lean = nan\ngraphics.msaa = 1\n");
        assert_eq!(render(&junk), render(&Settings::default()));
        assert_eq!(junk.msaa, 8, "a HUD file cannot touch graphics");
    }

    /// The bench's one-line fixture, worn over a fully laid-out cockpit:
    /// every HUD key the file omits comes back stock — the whole
    /// settings, not just a spot check — and only the map has moved.
    #[test]
    fn a_one_key_file_over_a_worn_cockpit_is_stock_but_for_that_key() {
        let sparse = apply(&worn(), "ui.map = on at -0.30,0.20\n");
        let mut want = Settings::default();
        want.layout.set(Instrument::Map, Slot::On);
        want.layout.set_free(Instrument::Map, [-0.30, 0.20]);
        assert_eq!(sparse.render(), want.render());
    }

    #[test]
    fn hud_files_are_numbered_and_sorted_naturally() {
        assert_eq!(slot_of(Path::new("hud-12.fhud")), Some(12));
        assert_eq!(slot_of(Path::new("hud-1.fhud")), Some(1));
        assert_eq!(slot_of(Path::new("jayjay-cluster.fhud")), None);
        assert!(is_hud_key("ui.gauge-style"));
        assert!(is_hud_key("holo.range"));
        assert!(is_hud_key("map.grid"));
        assert!(!is_hud_key("graphics.msaa"));
        assert!(!is_hud_key("control.boost"));
        assert!(!is_hud_key("settings.version"));
    }
}
