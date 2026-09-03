//! The world file: everything a quit should hand back on resume, in the
//! same idiom as `settings.rs` — plain `key = value` lines, no format
//! crate. Unlike settings, a bad line here is not a "fall back to
//! default" case: [`Save::parse`] refuses the whole file rather than
//! half-apply it, because a half-restored world (a ship at a hand-edited
//! position but the old velocity) is worse than none.
//!
//! The seal is `world.hash`, over the WHOLE file, not just the ship: see
//! [`whole_file_seal`]. `render` computes it from its own body; `parse`
//! builds a candidate `Save` from every field first, re-renders THAT
//! candidate's body, hashes it the same way, and compares. A legitimate
//! file's body re-renders byte for byte identical to what produced its
//! `world.hash` — that is the round trip's whole point — so a hand-edit
//! to anything under that line, `hull` or `arms.ammo` or `belt.dead` just
//! as much as `ship.pos`, moves the re-rendered bytes and fails the
//! comparison. There is no partial success: either every field came back
//! exactly as written, or nothing did.
//!
//! Floats are written with Rust's own shortest round-trip formatting
//! (`{:?}`), which is exact: `parse(render(x)) == x`, bit for bit. The
//! one field that is not written back exactly as stored is the ship's
//! orientation: `sealed_orient` renormalises it if (and only if) its
//! length is merely close to 1, which is why `render` writes and
//! hashes the SEALED form rather than whatever is literally on `self` —
//! otherwise a value already very near unit length (every orientation
//! the sim itself ever produces) would fail its own seal.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use farfall_sim as sim;
use glam::{DQuat, DVec3};

use crate::arms::Weapon;
use crate::belt::RockId;
use crate::mimic::{MimicSave, Mood, Phase};

/// The web build's localStorage key (`Settings` uses `"farfall.settings"`).
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
const WEB_KEY: &str = "farfall.world";

/// Everything the world file persists. Built by [`crate::Game::snapshot`],
/// consumed by [`crate::Game::restore`]; the two are each other's inverse
/// modulo the fields on [`crate::Game`] that are deliberately never
/// carried over a quit (see the module doc on `lib.rs`'s `restore`).
#[derive(Debug, Clone, PartialEq)]
pub struct Save {
    pub time_s: f64,
    pub ship_pos: DVec3,
    pub ship_vel: DVec3,
    pub ship_orient: DQuat,
    pub ship_spin: DVec3,
    pub assist: bool,
    pub landing: bool,
    pub appearance_index: usize,
    pub hyper_strain: f32,
    pub slip_at: f32,
    pub jumps: u32,
    pub arms_selected: Weapon,
    pub arms_ammo: [u32; 2],
    pub arms_jammed: [bool; 2],
    pub arms_heat: [f32; 2],
    pub arms_charge: f32,
    pub haul_tonnes: [f64; 4],
    pub hull: f32,
    pub strikes: u32,
    pub strike_rng: u32,
    pub odometer_m: f64,
    pub hoops_passed: u32,
    pub belt_dead: HashSet<RockId>,
    pub belt_wounds: HashMap<RockId, f64>,
    pub mimics_revealed: HashSet<RockId>,
    pub mimics: Vec<MimicSave>,
}

impl Save {
    /// The orientation as `world_state`/`render_body` actually use it:
    /// sealed via [`seal_orient`] rather than whatever is literally
    /// stored on `self`. Written form and hashed form must always be the
    /// SAME four numbers — see [`whole_file_seal`]'s doc for why — so
    /// both go through this, never `self.ship_orient` directly.
    fn sealed_orient(&self) -> DQuat {
        let o = self.ship_orient;
        seal_orient(o.x, o.y, o.z, o.w).unwrap_or(o)
    }

    fn world_state(&self) -> sim::WorldState {
        sim::WorldState {
            time_s: self.time_s,
            ship: sim::ShipState {
                pos_m: self.ship_pos,
                vel_mps: self.ship_vel,
                orient: self.sealed_orient(),
                ang_vel_radps: self.ship_spin,
                // Not in the file and not in the hash (state_hash eats
                // only time, pos, vel, spin, orient): a parked ship
                // resumed in Flight touches down clean again at once.
                ground: sim::Ground::Flight,
            },
        }
    }

    /// The ship-state hash alone (`sim::state_hash`) — one ingredient of
    /// [`Self::seal`], the whole-file value actually written and checked
    /// as `world.hash`.
    pub fn state_hash(&self) -> u64 {
        sim::state_hash(&self.world_state())
    }

    /// The value written (and checked) as `world.hash`: see
    /// [`whole_file_seal`].
    pub fn seal(&self) -> u64 {
        whole_file_seal(self.state_hash(), &self.render_body())
    }

    pub fn render(&self) -> String {
        let body = self.render_body();
        let mut out = String::from(
            "# farfall world — written by the game on quit and every 30 s \
             of sim time; delete it or use NEW GAME to start over\n",
        );
        out.push_str("world.version = 1\n");
        out.push_str(&format!(
            "world.hash = {:016x}\n",
            whole_file_seal(self.state_hash(), &body)
        ));
        out.push_str(&body);
        out
    }

    /// Every line `render` writes AFTER the `world.hash` line, exactly as
    /// `render` produces them. This is the text [`whole_file_seal`] hashes
    /// (alongside the ship-state hash) on both the write side (here) and
    /// the read side (`parse`, which builds a candidate `Save` and calls
    /// this on it) — the whole scheme rests on a legitimate file's body
    /// re-rendering byte for byte from what `parse` reads back out of it.
    fn render_body(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("sim.time = {}\n", f64s(self.time_s)));
        out.push_str(&format!("ship.pos = {}\n", vec3(self.ship_pos)));
        out.push_str(&format!("ship.vel = {}\n", vec3(self.ship_vel)));
        let so = self.sealed_orient();
        out.push_str(&format!(
            "ship.orient = {},{},{},{}\n",
            f64s(so.x),
            f64s(so.y),
            f64s(so.z),
            f64s(so.w)
        ));
        out.push_str(&format!("ship.spin = {}\n", vec3(self.ship_spin)));
        out.push_str(&format!("flight.assist = {}\n", bools(self.assist)));
        out.push_str(&format!("flight.landing = {}\n", bools(self.landing)));
        out.push_str(&format!("planet.appearance = {}\n", self.appearance_index));
        out.push_str(&format!("drive.entropy = {}\n", f32s(self.hyper_strain)));
        out.push_str(&format!("drive.slip-at = {}\n", f32s(self.slip_at)));
        out.push_str(&format!("drive.jumps = {}\n", self.jumps));
        out.push_str(&format!("arms.selected = {}\n", self.arms_selected.key()));
        out.push_str(&format!(
            "arms.ammo = {},{}\n",
            self.arms_ammo[0], self.arms_ammo[1]
        ));
        out.push_str(&format!(
            "arms.jammed = {},{}\n",
            bools(self.arms_jammed[0]),
            bools(self.arms_jammed[1])
        ));
        out.push_str(&format!(
            "arms.heat = {},{}\n",
            f32s(self.arms_heat[0]),
            f32s(self.arms_heat[1])
        ));
        out.push_str(&format!("arms.charge = {}\n", f32s(self.arms_charge)));
        out.push_str(&format!(
            "haul.tonnes = {},{},{},{}\n",
            f64s(self.haul_tonnes[0]),
            f64s(self.haul_tonnes[1]),
            f64s(self.haul_tonnes[2]),
            f64s(self.haul_tonnes[3])
        ));
        out.push_str(&format!("hull = {}\n", f32s(self.hull)));
        out.push_str(&format!("strikes = {}\n", self.strikes));
        out.push_str(&format!("strikes.rng = {:#010x}\n", self.strike_rng));
        out.push_str(&format!("odometer = {}\n", f64s(self.odometer_m)));
        out.push_str(&format!("hoops = {}\n", self.hoops_passed));
        out.push_str(&format!("belt.dead = {}\n", render_id_set(&self.belt_dead)));
        out.push_str(&format!(
            "belt.wounds = {}\n",
            render_wounds(&self.belt_wounds)
        ));
        out.push_str(&format!(
            "mimics.revealed = {}\n",
            render_id_set(&self.mimics_revealed)
        ));
        for (i, m) in self.mimics.iter().enumerate() {
            out.push_str(&format!("mimic.{i} = {}\n", render_mimic(m)));
        }
        out
    }

    /// Parse a world file. `None` on anything wrong at all — a missing or
    /// unsupported version, a non-finite number, an orientation that is
    /// not within `1e-6` of unit length (renormalised when it is), or a
    /// `world.hash` that does not match [`Self::seal`] of every field the
    /// rest of the file describes. There is no partial success: either
    /// every field came back exactly as written, or nothing did — and
    /// that now covers the WHOLE file, not just the ship (see
    /// [`whole_file_seal`]'s doc): a hand-edit to `hull`, `arms.ammo`,
    /// `haul.tonnes`, `belt.dead`, anything, is caught the same way a
    /// hand-edit to `ship.pos` always was.
    pub fn parse(text: &str) -> Option<Save> {
        let mut fields: HashMap<&str, &str> = HashMap::new();
        let mut mimic_lines: Vec<(usize, &str)> = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (k, v) = line.split_once('=')?;
            let (k, v) = (k.trim(), v.trim());
            if let Some(rest) = k.strip_prefix("mimic.") {
                mimic_lines.push((rest.parse().ok()?, v));
            } else {
                fields.insert(k, v);
            }
        }
        let get = |k: &str| fields.get(k).copied();

        if get("world.version")? != "1" {
            return None;
        }
        let saved_seal = parse_hex_u64(get("world.hash")?)?;

        let time_s = parse_f64(get("sim.time")?)?;
        let ship_pos = parse_vec3(get("ship.pos")?)?;
        let ship_vel = parse_vec3(get("ship.vel")?)?;
        let ship_orient = parse_quat(get("ship.orient")?)?;
        let ship_spin = parse_vec3(get("ship.spin")?)?;

        let assist = parse_bool(get("flight.assist")?)?;
        let landing = parse_bool(get("flight.landing")?)?;
        let appearance_index: usize = get("planet.appearance")?.parse().ok()?;
        let hyper_strain = parse_f32(get("drive.entropy")?)?;
        let slip_at = parse_f32(get("drive.slip-at")?)?;
        let jumps: u32 = get("drive.jumps")?.parse().ok()?;
        let arms_selected = Weapon::from_key(get("arms.selected")?)?;
        let arms_ammo = parse_u32x2(get("arms.ammo")?)?;
        let arms_jammed = parse_boolx2(get("arms.jammed")?)?;
        let arms_heat = parse_f32x2(get("arms.heat")?)?;
        let arms_charge = parse_f32(get("arms.charge")?)?;
        let haul_tonnes = parse_f64x4(get("haul.tonnes")?)?;
        let hull = parse_f32(get("hull")?)?;
        let strikes: u32 = get("strikes")?.parse().ok()?;
        let strike_rng = parse_hex_u32(get("strikes.rng")?)?;
        let odometer_m = parse_f64(get("odometer")?)?;
        let hoops_passed: u32 = get("hoops")?.parse().ok()?;
        let belt_dead = parse_id_set(get("belt.dead")?)?;
        let belt_wounds = parse_wounds(get("belt.wounds")?)?;
        let mimics_revealed = parse_id_set(get("mimics.revealed")?)?;

        mimic_lines.sort_by_key(|(i, _)| *i);
        let mut mimics = Vec::with_capacity(mimic_lines.len());
        for (_, v) in mimic_lines {
            mimics.push(parse_mimic(v)?);
        }

        // Every field is in hand: only now can the candidate's own body be
        // rendered, to check it seals to what the file claims. Any field
        // above missing or invalid already returned `None` via `?`.
        let candidate = Save {
            time_s,
            ship_pos,
            ship_vel,
            ship_orient,
            ship_spin,
            assist,
            landing,
            appearance_index,
            hyper_strain,
            slip_at,
            jumps,
            arms_selected,
            arms_ammo,
            arms_jammed,
            arms_heat,
            arms_charge,
            haul_tonnes,
            hull,
            strikes,
            strike_rng,
            odometer_m,
            hoops_passed,
            belt_dead,
            belt_wounds,
            mimics_revealed,
            mimics,
        };
        if candidate.seal() != saved_seal {
            return None;
        }
        Some(candidate)
    }

    /// Write the file (or, on the web, localStorage). Failure is logged,
    /// never fatal — the same policy as `Settings::save`.
    pub fn store(&self) {
        #[cfg(target_arch = "wasm32")]
        {
            crate::web::storage_set(WEB_KEY, &self.render());
            log::info!(
                "world: saved t={:.1}s hash={:016x}",
                self.time_s,
                self.seal()
            );
            return;
        }
        #[allow(unreachable_code)]
        {
            let Some(path) = path() else { return };
            self.store_to(&path);
        }
    }

    /// Write the file to an EXPLICIT path, bypassing the normal
    /// `~/.farfall/world.cfg` location — native only. Used by the
    /// `FARFALL_BENCH_SAVE` knob so a scripted bench run can produce a
    /// real sealed save of a real parked world without an interactive
    /// window. Same failure policy as `store`: logged, never fatal.
    pub fn store_to(&self, path: &std::path::Path) {
        let text = self.render();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(path, &text) {
            log::warn!("world: could not write {}: {e}", path.display());
            return;
        }
        log::info!(
            "world: saved t={:.1}s hash={:016x} -> {}",
            self.time_s,
            self.seal(),
            path.display()
        );
    }
}

/// FNV-1a 64 — the same constants `sim::state_hash` uses — over the
/// ship-state hash's own bytes, then every byte of the rendered body:
/// this is `world.hash`, the whole-file seal.
///
/// `render` computes this once, from its own body, to know what to write.
/// `parse` recomputes it from the candidate `Save` it just finished
/// building, by rendering THAT candidate's own body and hashing the
/// result the same way, then compares against the file's
/// `world.hash`. The two agree exactly when the candidate's body
/// re-renders byte for byte identical to what produced the file's own
/// hash — which is always true for a legitimate, untampered file (that
/// is the round trip's whole point) and false the moment any field under
/// `world.hash` has been hand-edited without also recomputing it: `hull`,
/// `arms.ammo`, `haul.tonnes`, `belt.dead`, anything at all, not only the
/// ship state a narrower seal would have covered.
fn whole_file_seal(ship_state_hash: u64, body: &str) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h = OFFSET;
    let mut eat = |b: u8| {
        h ^= u64::from(b);
        h = h.wrapping_mul(PRIME);
    };
    for b in ship_state_hash.to_le_bytes() {
        eat(b);
    }
    for &b in body.as_bytes() {
        eat(b);
    }
    h
}

/// Where the world file lives, native only (the web build keeps it in
/// localStorage under [`WEB_KEY`]).
pub fn path() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("world.cfg"))
}

/// Read and parse the world file, or `None` if there is none, it cannot
/// be read, or it does not parse.
pub fn load() -> Option<Save> {
    #[cfg(target_arch = "wasm32")]
    {
        return Save::parse(&crate::web::storage_get(WEB_KEY)?);
    }
    #[allow(unreachable_code)]
    {
        Save::parse(&std::fs::read_to_string(path()?).ok()?)
    }
}

/// Read and parse a world file from an EXPLICIT path, bypassing the
/// normal `~/.farfall/world.cfg` location — native only. Used by the
/// `FARFALL_BENCH_RESUME` knob; goes through the same `Save::parse` seal
/// check as a real resume, so a tampered file is refused the same way.
pub fn load_from(path: &std::path::Path) -> Option<Save> {
    Save::parse(&std::fs::read_to_string(path).ok()?)
}

/// NEW GAME: forget whatever was saved, so the next start is a fresh
/// spawn even with RESUME on.
pub fn forget() {
    #[cfg(target_arch = "wasm32")]
    {
        crate::web::storage_remove(WEB_KEY);
        return;
    }
    #[allow(unreachable_code)]
    if let Some(path) = path() {
        let _ = std::fs::remove_file(&path);
    }
}

// ---- rendering helpers -----------------------------------------------

/// Shortest round-trip text for an f64: `parse::<f64>(f64s(x)) == x`.
fn f64s(v: f64) -> String {
    format!("{v:?}")
}
/// Shortest round-trip text for an f32.
fn f32s(v: f32) -> String {
    format!("{v:?}")
}
fn bools(b: bool) -> &'static str {
    if b {
        "on"
    } else {
        "off"
    }
}
fn vec3(v: DVec3) -> String {
    format!("{},{},{}", f64s(v.x), f64s(v.y), f64s(v.z))
}
fn render_id(id: RockId) -> String {
    format!("{}:{}:{}:{}", id.0, id.1, id.2, id.3)
}
fn render_id_set(ids: &HashSet<RockId>) -> String {
    let mut v: Vec<RockId> = ids.iter().copied().collect();
    v.sort();
    v.iter()
        .map(|&id| render_id(id))
        .collect::<Vec<_>>()
        .join(";")
}
fn render_wounds(w: &HashMap<RockId, f64>) -> String {
    let mut v: Vec<(RockId, f64)> = w.iter().map(|(&k, &val)| (k, val)).collect();
    v.sort_by_key(|(k, _)| *k);
    v.into_iter()
        .map(|(id, val)| format!("{}={}", render_id(id), f64s(val)))
        .collect::<Vec<_>>()
        .join(";")
}
/// 21 comma-separated fields — see [`parse_mimic`] for the exact order,
/// which this must match field for field.
fn render_mimic(m: &MimicSave) -> String {
    format!(
        "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
        render_id(m.id),
        f64s(m.pos.x),
        f64s(m.pos.y),
        f64s(m.pos.z),
        f64s(m.vel.x),
        f64s(m.vel.y),
        f64s(m.vel.z),
        f64s(m.orient.x),
        f64s(m.orient.y),
        f64s(m.orient.z),
        f64s(m.orient.w),
        f64s(m.spin.x),
        f64s(m.spin.y),
        f64s(m.spin.z),
        f64s(m.born_s),
        m.phase.key(),
        f64s(m.phase_s),
        m.mood.key(),
        f64s(m.wound_j),
        f32s(m.effort),
        f32s(m.seed),
    )
}

// ---- parsing helpers ---------------------------------------------------

fn parse_f64(s: &str) -> Option<f64> {
    let v: f64 = s.parse().ok()?;
    v.is_finite().then_some(v)
}
fn parse_f32(s: &str) -> Option<f32> {
    let v: f32 = s.parse().ok()?;
    v.is_finite().then_some(v)
}
fn parse_bool(s: &str) -> Option<bool> {
    match s {
        "on" => Some(true),
        "off" => Some(false),
        _ => None,
    }
}
fn parse_vec3(s: &str) -> Option<DVec3> {
    let p: Vec<&str> = s.split(',').collect();
    if p.len() != 3 {
        return None;
    }
    Some(DVec3::new(
        parse_f64(p[0])?,
        parse_f64(p[1])?,
        parse_f64(p[2])?,
    ))
}
/// A unit quaternion from 4 comma-separated f64s: `None` unless the
/// parsed length is within `ORIENT_TOLERANCE` of 1 (sealed via
/// [`seal_orient`] otherwise).
fn parse_quat(s: &str) -> Option<DQuat> {
    let p: Vec<&str> = s.split(',').collect();
    if p.len() != 4 {
        return None;
    }
    seal_orient(
        parse_f64(p[0])?,
        parse_f64(p[1])?,
        parse_f64(p[2])?,
        parse_f64(p[3])?,
    )
}

/// How far a quaternion's length may sit from exactly 1 and still be
/// treated as already-unit: real sim output lands within a few ULPs of
/// 1 (`~1e-16`), many orders of magnitude inside this, so any orientation
/// the game itself produced is returned completely unchanged — which is
/// what keeps a legitimate save's round trip bit-exact rather than merely
/// numerically close.
const ORIENT_EXACT_EPS: f64 = 1e-12;
/// How far it may sit from 1 and still be accepted at all (renormalised
/// onto it, between here and `ORIENT_EXACT_EPS`).
const ORIENT_TOLERANCE: f64 = 1e-6;

/// Rejects (`None`) a length that is not finite or sits too far from 1
/// to trust at all (past `ORIENT_TOLERANCE`). Otherwise: `(x, y, z, w)`
/// exactly as given, when the length is already within
/// `ORIENT_EXACT_EPS` of 1, or renormalised onto unit length when it is
/// not. `render`'s hash and `parse`'s validation both go through this
/// same function on the same four numbers (the file's text round-trips
/// f64s exactly), so a hand-edited-but-within-tolerance orientation seals
/// to the same quaternion — and the same hash — on both sides.
fn seal_orient(x: f64, y: f64, z: f64, w: f64) -> Option<DQuat> {
    let len = (x * x + y * y + z * z + w * w).sqrt();
    if !len.is_finite() || (len - 1.0).abs() >= ORIENT_TOLERANCE {
        return None;
    }
    if (len - 1.0).abs() < ORIENT_EXACT_EPS {
        Some(DQuat::from_xyzw(x, y, z, w))
    } else {
        Some(DQuat::from_xyzw(x / len, y / len, z / len, w / len))
    }
}
fn parse_u32x2(s: &str) -> Option<[u32; 2]> {
    let p: Vec<&str> = s.split(',').collect();
    if p.len() != 2 {
        return None;
    }
    Some([p[0].parse().ok()?, p[1].parse().ok()?])
}
fn parse_f32x2(s: &str) -> Option<[f32; 2]> {
    let p: Vec<&str> = s.split(',').collect();
    if p.len() != 2 {
        return None;
    }
    Some([parse_f32(p[0])?, parse_f32(p[1])?])
}
fn parse_boolx2(s: &str) -> Option<[bool; 2]> {
    let p: Vec<&str> = s.split(',').collect();
    if p.len() != 2 {
        return None;
    }
    Some([parse_bool(p[0])?, parse_bool(p[1])?])
}
fn parse_f64x4(s: &str) -> Option<[f64; 4]> {
    let p: Vec<&str> = s.split(',').collect();
    if p.len() != 4 {
        return None;
    }
    Some([
        parse_f64(p[0])?,
        parse_f64(p[1])?,
        parse_f64(p[2])?,
        parse_f64(p[3])?,
    ])
}
fn parse_hex_u64(s: &str) -> Option<u64> {
    u64::from_str_radix(s.trim_start_matches("0x"), 16).ok()
}
fn parse_hex_u32(s: &str) -> Option<u32> {
    u32::from_str_radix(s.trim_start_matches("0x"), 16).ok()
}
fn parse_id(s: &str) -> Option<RockId> {
    let p: Vec<&str> = s.split(':').collect();
    if p.len() != 4 {
        return None;
    }
    Some((
        p[0].parse().ok()?,
        p[1].parse().ok()?,
        p[2].parse().ok()?,
        p[3].parse().ok()?,
    ))
}
fn parse_id_set(s: &str) -> Option<HashSet<RockId>> {
    if s.is_empty() {
        return Some(HashSet::new());
    }
    s.split(';').map(parse_id).collect()
}
fn parse_wounds(s: &str) -> Option<HashMap<RockId, f64>> {
    if s.is_empty() {
        return Some(HashMap::new());
    }
    let mut out = HashMap::new();
    for entry in s.split(';') {
        let (id_s, val_s) = entry.split_once('=')?;
        out.insert(parse_id(id_s)?, parse_f64(val_s)?);
    }
    Some(out)
}
/// `id,pos.x,y,z,vel.x,y,z,orient.x,y,z,w,spin.x,y,z,born,phase,phase_s,mood,wound,effort,seed`
/// — 21 comma-separated fields (the id's own `:`-separated parts count as
/// one).
fn parse_mimic(s: &str) -> Option<MimicSave> {
    let p: Vec<&str> = s.split(',').collect();
    if p.len() != 21 {
        return None;
    }
    let id = parse_id(p[0])?;
    let pos = DVec3::new(parse_f64(p[1])?, parse_f64(p[2])?, parse_f64(p[3])?);
    let vel = DVec3::new(parse_f64(p[4])?, parse_f64(p[5])?, parse_f64(p[6])?);
    let orient = seal_orient(
        parse_f64(p[7])?,
        parse_f64(p[8])?,
        parse_f64(p[9])?,
        parse_f64(p[10])?,
    )?;
    let spin = DVec3::new(parse_f64(p[11])?, parse_f64(p[12])?, parse_f64(p[13])?);
    let born_s = parse_f64(p[14])?;
    let phase = Phase::from_key(p[15])?;
    let phase_s = parse_f64(p[16])?;
    let mood = Mood::from_key(p[17])?;
    let wound_j = parse_f64(p[18])?;
    let effort = parse_f32(p[19])?;
    let seed = parse_f32(p[20])?;
    Some(MimicSave {
        id,
        pos,
        vel,
        orient,
        spin,
        born_s,
        phase,
        phase_s,
        mood,
        wound_j,
        effort,
        seed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Save {
        let mut belt_dead = HashSet::new();
        belt_dead.insert((1, -2, 3, 0));
        belt_dead.insert((4, 5, -6, 2));
        let mut belt_wounds = HashMap::new();
        belt_wounds.insert((7, 8, 9, 1), 12_345.5);
        let mut mimics_revealed = HashSet::new();
        mimics_revealed.insert((7, 8, 9, 1));
        Save {
            time_s: 1_234.5,
            ship_pos: DVec3::new(1.0, -2_000.25, 3.0e7),
            ship_vel: DVec3::new(-10.0, 0.0, 250.125),
            ship_orient: DQuat::from_xyzw(0.0, 0.0, 0.0, 1.0),
            ship_spin: DVec3::new(0.01, -0.02, 0.0),
            assist: true,
            landing: false,
            appearance_index: 2,
            hyper_strain: 0.4,
            slip_at: 0.83,
            jumps: 3,
            arms_selected: Weapon::Rail,
            arms_ammo: [600, 23],
            arms_jammed: [false, true],
            arms_heat: [0.1, 0.9],
            arms_charge: 0.5,
            haul_tonnes: [1.5, 0.0, 2.25, 3.0],
            hull: 0.8,
            strikes: 12,
            strike_rng: 0x9E37_79B9,
            odometer_m: 4_500.75,
            hoops_passed: 6,
            belt_dead,
            belt_wounds,
            mimics_revealed,
            mimics: vec![MimicSave {
                id: (7, 8, 9, 1),
                pos: DVec3::new(1.0, 2.0, 3.0),
                vel: DVec3::new(-1.0, -2.0, -3.0),
                orient: DQuat::from_xyzw(0.0, 0.0, 0.0, 1.0),
                spin: DVec3::new(0.1, 0.2, 0.3),
                born_s: 900.0,
                phase: Phase::Hailing,
                phase_s: 903.0,
                mood: Mood::Hail,
                wound_j: 500.0,
                effort: 0.25,
                seed: 0.75,
            }],
        }
    }

    #[test]
    fn a_saved_world_renders_and_parses_back_bit_for_bit() {
        let s = sample();
        let parsed = Save::parse(&s.render()).expect("a well-formed save parses");
        assert_eq!(parsed, s);
    }

    #[test]
    fn an_empty_world_with_no_mimics_or_wounds_round_trips() {
        let mut s = sample();
        s.belt_dead.clear();
        s.belt_wounds.clear();
        s.mimics_revealed.clear();
        s.mimics.clear();
        let parsed = Save::parse(&s.render()).expect("an empty-set save parses");
        assert_eq!(parsed, s);
    }

    #[test]
    fn floats_round_trip_bit_exact_at_the_edges() {
        for v in [
            0.0_f64,
            -0.0,
            1.0,
            -1.0,
            f64::MIN_POSITIVE,
            1.0e300,
            -1.0e300,
            123_456_789.987_654_3,
        ] {
            let rendered = f64s(v);
            let back: f64 = rendered.parse().expect("renders as valid f64 text");
            assert_eq!(back.to_bits(), v.to_bits(), "{v} via {rendered:?}");
        }
    }

    #[test]
    fn a_hand_edited_or_truncated_save_is_refused_whole() {
        let text = sample().render();
        assert!(Save::parse(&text).is_some(), "the control case parses");

        // One digit of ship.pos flipped: the rest of the file (and its
        // hash) no longer describes the same ship.
        let corrupted = text.replacen("ship.pos = 1.0", "ship.pos = 9.0", 1);
        assert!(corrupted.contains("ship.pos = 9.0"), "the edit landed");
        assert_eq!(Save::parse(&corrupted), None, "hash no longer matches");

        // Truncated halfway through (by chars, so a multibyte character in
        // the header comment can never land the cut mid-codepoint).
        let half: String = text.chars().take(text.chars().count() / 2).collect();
        assert_eq!(Save::parse(&half), None, "missing keys refuse the load");

        // A NaN smuggled into a numeric field.
        let nanned = text.replacen("arms.charge = 0.5", "arms.charge = NaN", 1);
        assert_eq!(Save::parse(&nanned), None, "non-finite numbers are refused");

        // An unsupported version.
        let future = text.replacen("world.version = 1", "world.version = 2", 1);
        assert_eq!(Save::parse(&future), None, "unknown version refuses");
    }

    /// The old seal (`sim::state_hash` of the ship alone) would have let
    /// every one of these through silently — SPEC §7.6 promises "refused
    /// whole", and that has to mean the whole file, not just the ship.
    #[test]
    fn a_hand_edit_to_the_hull_the_ammo_or_the_haul_is_refused_whole() {
        let text = sample().render();
        for (needle, edited) in [
            ("hull = 0.8", "hull = 0.5"),
            ("arms.ammo = 600,23", "arms.ammo = 999999,23"),
            (
                "haul.tonnes = 1.5,0.0,2.25,3.0",
                "haul.tonnes = 9.0,0.0,2.25,3.0",
            ),
            (
                "belt.dead = 1:-2:3:0;4:5:-6:2",
                "belt.dead = 1:-2:3:0;4:5:-6:9",
            ),
            ("mimics.revealed = 7:8:9:1", "mimics.revealed = 7:8:9:9"),
        ] {
            let tampered = text.replacen(needle, edited, 1);
            assert_ne!(tampered, text, "the edit for {needle:?} actually landed");
            assert_eq!(
                Save::parse(&tampered),
                None,
                "hand-edited {needle:?} without updating world.hash"
            );
        }
    }

    #[test]
    fn an_orientation_off_unit_length_is_renormalised_within_tolerance_and_refused_beyond_it() {
        // `render` always writes the SEALED orientation (see the module
        // doc), so `sample()`'s exactly-unit one comes out as "1.0" —
        // hand-edit that line to something merely close to unit length,
        // the way an actual hand edit would, without touching
        // `world.hash`: `seal` renormalises it back to the very same
        // quaternion on the read side, so the file's body re-renders
        // identically and the (unchanged) hash still matches — forgiven.
        let text = sample().render();
        let nudged = text.replacen(
            "ship.orient = 0.0,0.0,0.0,1.0",
            "ship.orient = 0.0,0.0,0.0,1.0000001",
            1,
        );
        assert_ne!(nudged, text, "the edit actually landed");
        let parsed = Save::parse(&nudged).expect("within tolerance, renormalised and accepted");
        assert_eq!(parsed.ship_orient.length(), 1.0);
        assert_eq!(
            parsed,
            Save::parse(&text).unwrap(),
            "the tiny edit renormalises onto the exact same orientation the original had"
        );

        // Well outside tolerance: refused outright, whatever the hash says.
        let way_off = text.replacen(
            "ship.orient = 0.0,0.0,0.0,1.0",
            "ship.orient = 0.0,0.0,0.0,2.0",
            1,
        );
        assert_eq!(Save::parse(&way_off), None);
    }

    #[test]
    fn resume_off_or_a_bench_run_never_touches_the_world_file() {
        assert!(
            crate::resume_allowed(true, false, None, false),
            "the ordinary case"
        );
        assert!(
            !crate::resume_allowed(false, false, None, false),
            "RESUME off"
        );
        assert!(
            !crate::resume_allowed(true, true, None, false),
            "frozen (a bench)"
        );
        assert!(
            !crate::resume_allowed(true, false, None, true),
            "a bench spawn override, even without FARFALL_BENCH itself"
        );
        for off in ["0", "off", "false"] {
            assert!(
                !crate::resume_allowed(true, false, Some(off), false),
                "FARFALL_RESUME={off} turns it off however the pilot set it"
            );
        }
        assert!(
            crate::resume_allowed(true, false, Some("1"), false),
            "an explicit non-off value does not itself force it on or off"
        );
        assert!(
            !crate::resume_allowed(false, false, Some("1"), false),
            "the environment can turn resume off but never force it on over the setting"
        );
    }
}
