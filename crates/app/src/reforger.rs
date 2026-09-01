//! The Reforger export: EXPORT REFORGER CFG on the STICK page writes a
//! real `Joystick_TFlightHotas4_0.conf` — the helicopter-pilot map the
//! hotas-reforger wizard ships (its `pilot` profile, byte for byte the
//! same grammar) — into `~/.farfall/reforger/`. Copy it to Reforger's
//! `Documents\My Games\ArmaReforger\profile\.save\settings\
//! customInputConfigs\` (and name it in InputUserSettings.conf's
//! CustomConfigs block) while the game is closed; Reforger rewrites the
//! file on exit, so a copy made while it runs is discarded.
//!
//! The format is unforgiving in three ways this module honours exactly:
//! UTF-8 with NO byte-order mark (a BOM makes the engine read the whole
//! file as empty, silently), CRLF line endings with a trailing CRLF
//! after the final brace, and one SPACE per indent level. The `{id}`
//! fields are 16 hex digits unique within the file; ours carry the same
//! `7CB1F0A54D` prefix the wizard uses, counted up in emission order.

use std::path::PathBuf;

use crate::settings::home_dir;

/// Reforger names the file for the device; the `_0` is its instance.
pub const FILE_NAME: &str = "Joystick_TFlightHotas4_0.conf";

/// The id prefix + a six-digit counter makes each 16-hex-digit id.
const ID_PREFIX: &str = "7CB1F0A54D";

/// One action block: the Reforger action, its filter preset, the
/// physical input token, and whether the block carries the one filter
/// the grammar knows (`InputFilterSingleClick`, FreelookReset alone).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding {
    pub action: &'static str,
    pub preset: &'static str,
    pub input: &'static str,
    pub single_click: bool,
}

const fn bind(action: &'static str, preset: &'static str, input: &'static str) -> Binding {
    Binding {
        action,
        preset,
        input,
        single_click: false,
    }
}

/// The helicopter-pilot map, in the wizard's emission order (its control
/// catalogue: buttons and hat first, then the axes). The throttle
/// rocker is deliberately unbound — on the measured unit it yawed under
/// command and drifted on release — and the trigger fires the turret,
/// never CharacterFire, so it stays safe on foot. Axis signs are the
/// measured unit's: the lever forward drives axis2 NEGATIVE, so
/// collective-up is `axis2-` (Reforger's stock preset has it backwards
/// on this hardware).
pub const PILOT: [Binding; 34] = [
    bind("TurretFire", "hold", "joystick0:button0"),
    bind("VONChannel", "hold", "joystick0:button1"),
    bind("VONDirectToggle", "click", "joystick0:button1"),
    bind("SwitchCameraType", "click", "joystick0:button3"),
    bind("HelicopterSightDeploy", "click", "joystick0:button2"),
    bind("FreelookUp", "up", "joystick0:pov_up"),
    bind("FreelookDown", "down", "joystick0:pov_down"),
    bind("FreelookLeft", "left", "joystick0:pov_left"),
    bind("FreelookRight", "right", "joystick0:pov_right"),
    bind("HelicopterAutohoverToggle", "click", "joystick0:button4"),
    bind("HelicopterWheelBrake", "pressed", "joystick0:button5"),
    bind(
        "HelicopterWheelBrakePersistent",
        "pressed",
        "joystick0:button5",
    ),
    bind("Freelook", "hold", "joystick0:button6"),
    Binding {
        action: "FreelookReset",
        preset: "click",
        input: "joystick0:button6",
        single_click: true,
    },
    bind("SelectAction", "next", "joystick0:button7"),
    bind("TurretReload", "click", "joystick0:button9"),
    bind("TurretNextWeapon", "click", "joystick0:button8"),
    bind("GadgetMap", "select", "joystick0:button10"),
    bind("HelicopterEngineStart", "click", "joystick0:button11"),
    bind("VehicleEngineStart", "click", "joystick0:button11"),
    bind("HelicopterCyclicRight", "right", "joystick0:axis0+"),
    bind("TurretAimRight", "right", "joystick0:axis0+"),
    bind("HelicopterCyclicLeft", "left", "joystick0:axis0-"),
    bind("TurretAimLeft", "left", "joystick0:axis0-"),
    bind("HelicopterCyclicForward", "forward", "joystick0:axis1-"),
    bind("TurretAimDown", "down", "joystick0:axis1-"),
    bind("HelicopterCyclicBack", "back", "joystick0:axis1+"),
    bind("TurretAimUp", "up", "joystick0:axis1+"),
    bind("HelicopterAntiTorqueRight", "right", "joystick0:axis5+"),
    bind("TurretRotateRight", "right", "joystick0:axis5+"),
    bind("HelicopterAntiTorqueLeft", "left", "joystick0:axis5-"),
    bind("TurretRotateLeft", "left", "joystick0:axis5-"),
    bind("HelicopterCollectiveIncrease", "up", "joystick0:axis2-"),
    bind("HelicopterCollectiveDecrease", "down", "joystick0:axis2+"),
];

/// The file's text: every binding as its own Action block under one
/// ActionManager, ids counted up in emission order (a sum, its value,
/// then a filter id where one is carried).
pub fn render(bindings: &[Binding]) -> String {
    let mut lines: Vec<String> = vec!["ActionManager {".into(), " Actions {".into()];
    let mut next_id = 1u32;
    let mut id = move || {
        let s = format!("{ID_PREFIX}{next_id:06X}");
        next_id += 1;
        s
    };
    for b in bindings {
        lines.push(format!("  Action {} {{", b.action));
        lines.push(format!("   InputSource InputSourceSum \"{{{}}}\" {{", id()));
        lines.push("    Sources {".into());
        lines.push(format!("     InputSourceValue \"{{{}}}\" {{", id()));
        lines.push(format!("      FilterPreset \"{}\"", b.preset));
        lines.push(format!("      Input \"{}\"", b.input));
        if b.single_click {
            lines.push(format!(
                "      Filter InputFilterSingleClick \"{{{}}}\" {{",
                id()
            ));
            lines.push("      }".into());
        }
        lines.push("     }".into());
        lines.push("    }".into());
        lines.push("   }".into());
        lines.push("  }".into());
    }
    lines.push(" }".into());
    lines.push("}".into());
    lines.join("\r\n") + "\r\n"
}

/// Read a rendered file back into bindings, so a test can prove the
/// round trip. Returns None on anything the grammar does not allow.
#[cfg(test)]
pub fn parse(text: &str) -> Option<Vec<Binding>> {
    let mut out: Vec<Binding> = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("Action ") {
            let name = rest.strip_suffix(" {")?;
            let action = PILOT.iter().find(|b| b.action == name)?.action;
            out.push(bind(action, "", ""));
        } else if let Some(rest) = t.strip_prefix("FilterPreset ") {
            let preset = rest.strip_prefix('"')?.strip_suffix('"')?;
            let b = out.last_mut()?;
            b.preset = PILOT.iter().map(|p| p.preset).find(|p| *p == preset)?;
        } else if let Some(rest) = t.strip_prefix("Input ") {
            let input = rest.strip_prefix('"')?.strip_suffix('"')?;
            let b = out.last_mut()?;
            b.input = PILOT.iter().map(|p| p.input).find(|i| *i == input)?;
        } else if t.starts_with("Filter InputFilterSingleClick ") {
            out.last_mut()?.single_click = true;
        }
    }
    Some(out)
}

/// Where the exported file goes.
pub fn dir() -> Option<PathBuf> {
    home_dir().map(|h| h.join(".farfall").join("reforger"))
}

/// Write the pilot map. Returns the path written.
pub fn save() -> Option<PathBuf> {
    let d = dir()?;
    let _ = std::fs::create_dir_all(&d);
    let path = d.join(FILE_NAME);
    std::fs::write(&path, render(&PILOT)).ok()?;
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The head of the file, byte for byte as the hotas-reforger
    /// wizard's own accepted output begins (its 20260817-141811 backup):
    /// no BOM, CRLF, one space per level, ids from {PREFIX}000001.
    const HEAD: &str = "ActionManager {\r\n Actions {\r\n  Action TurretFire {\r\n   InputSource InputSourceSum \"{7CB1F0A54D000001}\" {\r\n    Sources {\r\n     InputSourceValue \"{7CB1F0A54D000002}\" {\r\n      FilterPreset \"hold\"\r\n      Input \"joystick0:button0\"\r\n     }\r\n    }\r\n   }\r\n  }\r\n  Action VONChannel {\r\n   InputSource InputSourceSum \"{7CB1F0A54D000003}\" {\r\n";

    #[test]
    fn the_conf_round_trips_and_matches_the_wizards_bytes() {
        let file = render(&PILOT);
        assert!(file.starts_with(HEAD), "the head is the wizard's, verbatim");
        assert!(
            file.ends_with("  }\r\n }\r\n}\r\n"),
            "trailing CRLF after the final brace"
        );
        assert!(!file.contains('\t'), "spaces, never tabs");
        assert!(
            !file.starts_with('\u{feff}'),
            "no BOM: a BOM reads as an empty file"
        );
        assert_eq!(parse(&file).expect("our own file parses"), PILOT.to_vec());
    }

    /// Not a test: writes the real export on demand (the sim's
    /// print_golden idiom), so the file can be checked byte for byte
    /// against the hotas-reforger wizard's accepted output.
    /// `cargo test -p farfall-app write_the_conf -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn write_the_conf() {
        let path = save().expect("a home directory to write under");
        println!("wrote {}", path.display());
    }

    #[test]
    fn every_id_is_unique_and_counted_in_order() {
        let file = render(&PILOT);
        let ids: Vec<&str> = file
            .match_indices(ID_PREFIX)
            .map(|(i, _)| &file[i..i + 16])
            .collect();
        // 34 sums + 34 values + FreelookReset's single-click filter.
        assert_eq!(ids.len(), 69);
        for (n, id) in ids.iter().enumerate() {
            assert_eq!(**id, format!("{ID_PREFIX}{:06X}", n + 1), "emission order");
        }
    }

    #[test]
    fn braces_balance_and_never_go_negative() {
        let file = render(&PILOT);
        let mut depth = 0i32;
        for line in file.lines() {
            depth += line.matches('{').count() as i32;
            depth -= line.matches('}').count() as i32;
            assert!(depth >= 0, "a close before its open");
        }
        assert_eq!(depth, 0);
    }

    #[test]
    fn the_grammar_only_speaks_reforgers_tokens() {
        for b in PILOT {
            let ok = [
                "left", "right", "up", "down", "forward", "back", "hold", "click", "pressed",
                "select", "next",
            ];
            assert!(ok.contains(&b.preset), "{}: preset {}", b.action, b.preset);
            let token = b.input.strip_prefix("joystick0:").expect(b.input);
            let axis = token.starts_with("axis")
                && token.len() == 6
                && token.as_bytes()[4].is_ascii_digit()
                && matches!(token.as_bytes()[5], b'+' | b'-');
            let button = token
                .strip_prefix("button")
                .is_some_and(|n| n.parse::<u8>().is_ok());
            let pov = matches!(token, "pov_up" | "pov_down" | "pov_left" | "pov_right");
            assert!(axis || button || pov, "{}: input {}", b.action, b.input);
        }
    }
}
