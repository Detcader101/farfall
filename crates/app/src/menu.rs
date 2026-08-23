//! The in-game menu: graphics, controls, cockpit — on the same glass as
//! everything else.
//!
//! Rendered through the HUD text pass (a bit mask the GPU draws on the
//! canopy), driven by the keyboard, editing [`Settings`] in place. Esc
//! opens it and closes it; while it is open the sim is paused and the
//! flight keys are released, because a pilot reading a menu is not flying.
//!
//! Three pages, Tab between them. Up/Down moves, Left/Right changes a
//! value, Enter starts a key rebind (the next key pressed takes it; Esc
//! cancels). Every change is applied at once and written to the settings
//! file; there is no "save" — the file is the state.

use crate::cockpit::{Instrument, SAFE_EDGE_MAX};
use crate::input::{is_reserved, key_name, Action};
use crate::settings::{
    Settings, COCKPIT_RES_CHOICES, FOV_MAX, FOV_MIN, HOOP_SIZE_MAX, HOOP_SIZE_MIN,
    LANDING_SPACINGS, MSAA_CHOICES,
};
use farfall_render::text::TextBitmap;
use winit::keyboard::KeyCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Graphics,
    Controls,
    Cockpit,
    Gauges,
    Map,
}

impl Page {
    /// The settings menu's pages. The MAP is its own panel (M), not a page.
    const ALL: [Page; 4] = [Page::Graphics, Page::Controls, Page::Cockpit, Page::Gauges];

    fn short(self) -> &'static str {
        match self {
            Page::Graphics => "GFX",
            Page::Controls => "KEYS",
            Page::Cockpit => "CABIN",
            Page::Gauges => "GAUGES",
            Page::Map => "MAP",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Page::Graphics => "GRAPHICS",
            Page::Controls => "CONTROLS",
            Page::Cockpit => "COCKPIT",
            Page::Gauges => "GAUGES",
            Page::Map => "MAP",
        }
    }
}

/// What a key press in the menu asks the app to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuEvent {
    Nothing,
    /// Settings changed: apply and save.
    Changed(Change),
    Closed,
    Quit,
    /// Close the menu and fire the wormhole drive at the plan.
    Engage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Graphics,
    Bindings,
    Layout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Item {
    Msaa,
    Scale,
    Vsync,
    Quit,
    Bind(Action),
    BindBoost,
    BindBrake,
    BindDespin,
    Slot(Instrument),
    SafeEdge,
    HoopSize,
    LandingHoops,
    CockpitFrame,
    CockpitGlow,
    CockpitHull,
    CockpitRes,
    Fov,
    GaugeStyle,
    GaugesStay,
    Guide,
    /// The GAUGES page's per-dial block: which dial, and its own numbers.
    DialSelect,
    DialSize,
    DialStyle,
    DialFade,
    DialRot,
    MapRings,
    MapGrid,
    LookSens,
    Destination,
    SafeDist,
    Engage,
}

impl Item {
    fn label(self) -> &'static str {
        match self {
            Item::Msaa => "MSAA",
            Item::Scale => "RENDER SCALE",
            Item::Vsync => "VSYNC",
            Item::Quit => "QUIT GAME",
            Item::Bind(a) => a.name(),
            Item::BindBoost => "BOOST",
            Item::BindBrake => "AIR BRAKE",
            Item::BindDespin => "DESPIN",
            Item::Slot(i) => i.name(),
            Item::SafeEdge => "SAFE EDGE",
            Item::HoopSize => "HOOP SIZE",
            Item::LandingHoops => "LANDING HOOPS",
            Item::CockpitFrame => "CABIN FRAME",
            Item::CockpitGlow => "CABIN GLOW",
            Item::CockpitHull => "CABIN METAL",
            Item::CockpitRes => "CABIN DETAIL",
            Item::Fov => "FOV",
            Item::GaugeStyle => "GAUGE STYLE",
            Item::GaugesStay => "GAUGES",
            Item::Guide => "GUIDE",
            Item::DialSelect => "DIAL",
            Item::DialSize => "  SIZE",
            Item::DialStyle => "  STYLE",
            Item::DialFade => "  FADE",
            Item::DialRot => "  ROTATE",
            Item::MapRings => "BODY RINGS",
            Item::MapGrid => "GRID",
            Item::LookSens => "LOOK SENS",
            Item::Destination => "DESTINATION",
            Item::SafeDist => "SAFE DISTANCE",
            Item::Engage => "ENGAGE DRIVE",
        }
    }

    fn value(self, s: &Settings) -> String {
        match self {
            Item::Msaa => format!("{}X", s.msaa),
            Item::Scale => format!("{:.0}%", s.scale * 100.0),
            Item::Vsync => (if s.vsync { "ON" } else { "OFF" }).to_string(),
            Item::Quit => String::new(),
            Item::Bind(a) => key_name(s.bindings.key_for(a)).to_string(),
            Item::BindBoost => key_name(s.bindings.boost).to_string(),
            Item::BindBrake => key_name(s.bindings.brake).to_string(),
            Item::BindDespin => key_name(s.bindings.despin).to_string(),
            Item::Slot(i) => match s.layout.free(i) {
                Some(_) => "DRAGGED".to_string(),
                None => s.layout.get(i).name().to_string(),
            },
            Item::SafeEdge => format!("{:.0}%", s.layout.safe_edge * 100.0),
            Item::HoopSize => format!("{:.2}x", s.hoop_size),
            Item::LandingHoops => format!("{:.0}M", s.landing_spacing_m),
            Item::CockpitFrame => if s.cockpit_frame { "ON" } else { "OFF" }.to_string(),
            Item::CockpitGlow => format!("{:.2}x", s.cockpit_glow),
            Item::CockpitHull => format!("{:.0}%", s.cockpit_hull * 100.0),
            Item::CockpitRes => format!("{:.0}%", s.cockpit_res * 100.0),
            Item::Fov => format!("{:.0} DEG", s.fov),
            Item::GaugeStyle => s.gauge_style.name().to_string(),
            Item::GaugesStay => if s.gauges_stay { "STAY" } else { "FADE" }.to_string(),
            Item::Guide => if s.guide { "ON" } else { "OFF" }.to_string(),
            Item::DialSelect => String::new(),
            Item::DialSize => String::new(),
            Item::DialStyle => String::new(),
            Item::DialFade => String::new(),
            Item::DialRot => String::new(),
            Item::MapRings => s.map_rings.to_string(),
            Item::MapGrid => if s.map_grid { "ON" } else { "OFF" }.to_string(),
            Item::LookSens => format!("{:.2}", s.look_sensitivity),
            Item::Destination => s.plan.dest.name().to_string(),
            Item::SafeDist => format!("{:.2} R", s.plan.safe_radii),
            Item::Engage => String::new(),
        }
    }

    fn rebindable(self) -> bool {
        matches!(
            self,
            Item::Bind(_) | Item::BindBoost | Item::BindBrake | Item::BindDespin
        )
    }
}

/// Rows of text the bitmap can hold (64 px / 6 px pitch), minus the
/// header and the footer.
const VISIBLE_ITEMS: usize = 8;
const ROW_PX: usize = 6;
/// Characters per row (128 px / 4 px advance).
const COLS: usize = 32;

#[derive(Debug, Clone, Copy)]
pub struct Menu {
    pub open: bool,
    /// A one-page panel (the DRIVE panel beside the map): no paging, its
    /// own header.
    standalone: bool,
    page: Page,
    cursor: usize,
    scroll: usize,
    /// Waiting for a key to bind to the item under the cursor.
    rebinding: bool,
    /// Which of MSAA_CHOICES this GPU can render at (set at start).
    msaa_ok: [bool; 4],
    /// The dial the GAUGES page's per-dial block edits.
    dial: Instrument,
}

impl Default for Menu {
    fn default() -> Self {
        Self {
            open: false,
            standalone: false,
            page: Page::Graphics,
            cursor: 0,
            scroll: 0,
            rebinding: false,
            msaa_ok: [true; 4],
            dial: Instrument::Speed,
        }
    }
}

impl Menu {
    pub fn new() -> Self {
        Self::default()
    }

    fn items(&self) -> Vec<Item> {
        match self.page {
            Page::Graphics => vec![
                Item::Msaa,
                Item::Scale,
                Item::Vsync,
                Item::Fov,
                Item::CockpitRes,
                Item::Quit,
            ],
            Page::Controls => {
                let mut v: Vec<Item> = Action::ALL.iter().map(|&a| Item::Bind(a)).collect();
                v.push(Item::BindBoost);
                v.push(Item::BindBrake);
                v.push(Item::BindDespin);
                v.push(Item::LookSens);
                v
            }
            Page::Cockpit => vec![
                Item::CockpitFrame,
                Item::CockpitGlow,
                Item::CockpitHull,
                Item::SafeEdge,
                Item::HoopSize,
                Item::LandingHoops,
                Item::Guide,
            ],
            Page::Gauges => {
                let mut v: Vec<Item> = vec![Item::GaugeStyle, Item::GaugesStay];
                v.extend(Instrument::ALL.iter().map(|&i| Item::Slot(i)));
                v.push(Item::DialSelect);
                v.push(Item::DialSize);
                v.push(Item::DialStyle);
                v.push(Item::DialFade);
                v.push(Item::DialRot);
                v
            }
            Page::Map => vec![
                Item::Destination,
                Item::SafeDist,
                Item::Engage,
                Item::MapRings,
                Item::MapGrid,
            ],
        }
    }

    /// Restrict the MSAA choices to what the GPU supports.
    pub fn set_msaa_supported(&mut self, supported: &[u32]) {
        for (i, n) in MSAA_CHOICES.iter().enumerate() {
            self.msaa_ok[i] = supported.contains(n);
        }
    }

    /// The MAP page is showing: draw the system map under the text.
    pub fn map_open(&self) -> bool {
        self.open && self.page == Page::Map
    }

    /// The DRIVE panel: the map's own controls, a one-page menu of its own
    /// that opens with the map (M).
    pub fn map_panel() -> Self {
        Self {
            standalone: true,
            page: Page::Map,
            ..Self::default()
        }
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.rebinding = false;
    }

    fn set_page(&mut self, page: Page) {
        self.page = page;
        self.cursor = 0;
        self.scroll = 0;
        self.rebinding = false;
    }

    /// A key press while the menu is open.
    pub fn key(&mut self, key: KeyCode, settings: &mut Settings) -> MenuEvent {
        let items = self.items();
        let item = items[self.cursor.min(items.len() - 1)];

        if self.rebinding {
            if key == KeyCode::Escape {
                self.rebinding = false;
                return MenuEvent::Nothing;
            }
            if is_reserved(key) {
                return MenuEvent::Nothing;
            }
            let bound = match item {
                Item::Bind(a) => settings.bindings.bind(a, key),
                Item::BindBoost => settings.bindings.bind_boost(key),
                Item::BindDespin => settings.bindings.bind_despin(key),
                Item::BindBrake => settings.bindings.bind_brake(key),
                _ => false,
            };
            self.rebinding = false;
            return if bound {
                MenuEvent::Changed(Change::Bindings)
            } else {
                MenuEvent::Nothing
            };
        }

        match key {
            KeyCode::Escape => {
                self.open = false;
                MenuEvent::Closed
            }
            KeyCode::Tab if !self.standalone => {
                let i = Page::ALL.iter().position(|&p| p == self.page).unwrap_or(0);
                self.set_page(Page::ALL[(i + 1) % Page::ALL.len()]);
                MenuEvent::Nothing
            }
            KeyCode::ArrowUp => {
                self.cursor = (self.cursor + items.len() - 1) % items.len();
                self.keep_cursor_visible();
                MenuEvent::Nothing
            }
            KeyCode::ArrowDown => {
                self.cursor = (self.cursor + 1) % items.len();
                self.keep_cursor_visible();
                MenuEvent::Nothing
            }
            KeyCode::ArrowLeft | KeyCode::ArrowRight => {
                let forward = key == KeyCode::ArrowRight;
                self.adjust(item, forward, settings)
            }
            KeyCode::Enter | KeyCode::Space => match item {
                Item::Quit => MenuEvent::Quit,
                Item::Engage => {
                    self.open = false;
                    MenuEvent::Engage
                }
                i if i.rebindable() => {
                    self.rebinding = true;
                    MenuEvent::Nothing
                }
                i => self.adjust(i, true, settings),
            },
            _ => MenuEvent::Nothing,
        }
    }

    fn adjust(&mut self, item: Item, forward: bool, s: &mut Settings) -> MenuEvent {
        match item {
            Item::Msaa => {
                let n = MSAA_CHOICES.len();
                let mut i = MSAA_CHOICES.iter().position(|&m| m == s.msaa).unwrap_or(2);
                // Step to the next count this GPU can do; none other: stay.
                for _ in 0..n {
                    i = if forward {
                        (i + 1) % n
                    } else {
                        (i + n - 1) % n
                    };
                    if self.msaa_ok[i] {
                        break;
                    }
                }
                if !self.msaa_ok[i] || MSAA_CHOICES[i] == s.msaa {
                    return MenuEvent::Nothing;
                }
                s.msaa = MSAA_CHOICES[i];
                MenuEvent::Changed(Change::Graphics)
            }
            Item::Scale => {
                let step = if forward { 0.05 } else { -0.05 };
                s.scale = ((s.scale + step) * 100.0).round() / 100.0;
                s.scale = s.scale.clamp(0.25, 1.0);
                MenuEvent::Changed(Change::Graphics)
            }
            Item::Vsync => {
                s.vsync = !s.vsync;
                MenuEvent::Changed(Change::Graphics)
            }
            Item::Slot(i) => {
                s.layout.cycle(i, forward);
                MenuEvent::Changed(Change::Layout)
            }
            Item::LookSens => {
                let step = if forward { 0.25 } else { -0.25 };
                let next = (s.look_sensitivity + step).clamp(0.25, 5.0);
                if (next - s.look_sensitivity).abs() < 1e-6 {
                    return MenuEvent::Nothing;
                }
                s.look_sensitivity = next;
                MenuEvent::Changed(Change::Bindings)
            }
            Item::Destination => {
                s.plan.cycle_destination(forward);
                MenuEvent::Changed(Change::Layout)
            }
            Item::SafeDist => {
                let before = s.plan.safe_radii;
                s.plan.adjust_safe(forward);
                if (s.plan.safe_radii - before).abs() < 1e-9 {
                    return MenuEvent::Nothing;
                }
                MenuEvent::Changed(Change::Layout)
            }
            Item::Engage => MenuEvent::Nothing,
            Item::LandingHoops => {
                let n = LANDING_SPACINGS.len();
                let i = LANDING_SPACINGS
                    .iter()
                    .position(|&x| x == s.landing_spacing_m)
                    .unwrap_or(1);
                let j = if forward {
                    (i + 1) % n
                } else {
                    (i + n - 1) % n
                };
                s.landing_spacing_m = LANDING_SPACINGS[j];
                MenuEvent::Changed(Change::Layout)
            }
            Item::CockpitFrame => {
                s.cockpit_frame = !s.cockpit_frame;
                MenuEvent::Changed(Change::Layout)
            }
            Item::CockpitGlow => {
                let step = if forward { 0.25 } else { -0.25 };
                let next = (s.cockpit_glow + step).clamp(0.25, 2.0);
                if (next - s.cockpit_glow).abs() < 1e-6 {
                    return MenuEvent::Nothing;
                }
                s.cockpit_glow = next;
                MenuEvent::Changed(Change::Layout)
            }
            Item::CockpitHull => {
                let step = if forward { 0.1 } else { -0.1 };
                let next = (s.cockpit_hull + step).clamp(0.0, 1.0);
                if (next - s.cockpit_hull).abs() < 1e-6 {
                    return MenuEvent::Nothing;
                }
                s.cockpit_hull = next;
                MenuEvent::Changed(Change::Layout)
            }
            Item::Fov => {
                let step = if forward { 5.0 } else { -5.0 };
                let next = (s.fov + step).clamp(FOV_MIN, FOV_MAX);
                if (next - s.fov).abs() < 1e-6 {
                    return MenuEvent::Nothing;
                }
                s.fov = next;
                MenuEvent::Changed(Change::Layout)
            }
            Item::GaugeStyle => {
                s.gauge_style = s.gauge_style.next(forward);
                MenuEvent::Changed(Change::Layout)
            }
            Item::GaugesStay => {
                s.gauges_stay = !s.gauges_stay;
                MenuEvent::Changed(Change::Layout)
            }
            Item::Guide => {
                s.guide = !s.guide;
                MenuEvent::Changed(Change::Layout)
            }
            Item::DialSelect => {
                let dials: Vec<Instrument> = Instrument::ALL
                    .iter()
                    .copied()
                    .filter(|i| i.slotted())
                    .collect();
                let i = dials.iter().position(|&d| d == self.dial).unwrap_or(0);
                let n = dials.len();
                self.dial = dials[if forward {
                    (i + 1) % n
                } else {
                    (i + n - 1) % n
                }];
                MenuEvent::Nothing
            }
            Item::DialSize => {
                let d = &mut s.dials[self.dial as usize];
                let step = if forward { 0.125 } else { -0.125 };
                let next = (d.size + step).clamp(
                    crate::settings::DIAL_SIZE_MIN,
                    crate::settings::DIAL_SIZE_MAX,
                );
                if (next - d.size).abs() < 1e-6 {
                    return MenuEvent::Nothing;
                }
                d.size = next;
                MenuEvent::Changed(Change::Layout)
            }
            Item::DialStyle => {
                let d = &mut s.dials[self.dial as usize];
                d.style = match (d.style, forward) {
                    (None, true) => Some(crate::settings::GaugeStyle::Tron),
                    (Some(crate::settings::GaugeStyle::Tron), true) => {
                        Some(crate::settings::GaugeStyle::Jet)
                    }
                    (Some(crate::settings::GaugeStyle::Jet), true) => {
                        Some(crate::settings::GaugeStyle::Dial)
                    }
                    (Some(crate::settings::GaugeStyle::Dial), true) => None,
                    (None, false) => Some(crate::settings::GaugeStyle::Dial),
                    (Some(crate::settings::GaugeStyle::Dial), false) => {
                        Some(crate::settings::GaugeStyle::Jet)
                    }
                    (Some(crate::settings::GaugeStyle::Jet), false) => {
                        Some(crate::settings::GaugeStyle::Tron)
                    }
                    (Some(crate::settings::GaugeStyle::Tron), false) => None,
                };
                MenuEvent::Changed(Change::Layout)
            }
            Item::DialRot => {
                let d = &mut s.dials[self.dial as usize];
                d.rot_deg = (d.rot_deg + if forward { 15.0 } else { -15.0 }).rem_euclid(360.0);
                MenuEvent::Changed(Change::Layout)
            }
            Item::DialFade => {
                let d = &mut s.dials[self.dial as usize];
                d.stay = match (d.stay, forward) {
                    (None, true) | (Some(false), false) => Some(true),
                    (Some(true), true) | (None, false) => Some(false),
                    (Some(false), true) | (Some(true), false) => None,
                };
                MenuEvent::Changed(Change::Layout)
            }
            Item::CockpitRes => {
                let n = COCKPIT_RES_CHOICES.len();
                let i = COCKPIT_RES_CHOICES
                    .iter()
                    .position(|&x| (x - s.cockpit_res).abs() < 1e-6)
                    .unwrap_or(0);
                let j = if forward {
                    (i + 1) % n
                } else {
                    (i + n - 1) % n
                };
                s.cockpit_res = COCKPIT_RES_CHOICES[j];
                MenuEvent::Changed(Change::Graphics)
            }
            Item::MapRings => {
                let n = crate::map::RINGS_MAX + 1;
                s.map_rings = if forward {
                    (s.map_rings + 1) % n
                } else {
                    (s.map_rings + n - 1) % n
                };
                MenuEvent::Changed(Change::Layout)
            }
            Item::MapGrid => {
                s.map_grid = !s.map_grid;
                MenuEvent::Changed(Change::Layout)
            }
            Item::HoopSize => {
                let step = if forward { 0.25 } else { -0.25 };
                let next = (s.hoop_size + step).clamp(HOOP_SIZE_MIN, HOOP_SIZE_MAX);
                if (next - s.hoop_size).abs() < 1e-6 {
                    return MenuEvent::Nothing;
                }
                s.hoop_size = next;
                MenuEvent::Changed(Change::Layout)
            }
            Item::SafeEdge => {
                let step = if forward { 0.01 } else { -0.01 };
                let next = (s.layout.safe_edge + step).clamp(0.0, SAFE_EDGE_MAX);
                if (next - s.layout.safe_edge).abs() < 1e-6 {
                    return MenuEvent::Nothing;
                }
                s.layout.set_safe_edge(next);
                MenuEvent::Changed(Change::Layout)
            }
            Item::Quit | Item::Bind(_) | Item::BindBoost | Item::BindBrake | Item::BindDespin => {
                MenuEvent::Nothing
            }
        }
    }

    fn keep_cursor_visible(&mut self) {
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor >= self.scroll + VISIBLE_ITEMS {
            self.scroll = self.cursor + 1 - VISIBLE_ITEMS;
        }
    }

    /// An item's shown value; the per-dial block reads the selected dial.
    fn value_of(&self, item: Item, s: &Settings) -> String {
        let d = s.dials[self.dial as usize];
        match item {
            Item::DialSelect => self.dial.name().to_string(),
            Item::DialSize => format!("{:.2}X", d.size),
            Item::DialStyle => d
                .style
                .map_or("AUTO".to_string(), |st| st.name().to_string()),
            Item::DialFade => match d.stay {
                None => "AUTO",
                Some(true) => "STAY",
                Some(false) => "FADE",
            }
            .to_string(),
            Item::DialRot => format!("{:.0} DEG", d.rot_deg),
            other => other.value(s),
        }
    }

    /// Draw the menu into the text bitmap.
    pub fn render(&self, text: &mut TextBitmap, s: &Settings) {
        text.clear();
        // Header: the pages, the current one bracketed — or, for a panel
        // of its own, just its name.
        let mut header = String::new();
        if self.standalone {
            header.push_str(&format!("[{}]  WORMHOLE DRIVE", self.page.name()));
        } else {
            // Short names, so four pages fit the row.
            for p in Page::ALL {
                if p == self.page {
                    header.push_str(&format!("[{}]", p.short()));
                } else {
                    header.push_str(&format!(" {} ", p.short()));
                }
            }
        }
        text.draw(0, 0, &header);

        let items = self.items();
        let end = (self.scroll + VISIBLE_ITEMS).min(items.len());
        for (row, idx) in (self.scroll..end).enumerate() {
            let item = items[idx];
            let y = (row + 1) * ROW_PX;
            let selected = idx == self.cursor;
            let value = if selected && self.rebinding {
                "PRESS KEY".to_string()
            } else {
                self.value_of(item, s)
            };
            let mark = if selected { ">" } else { " " };
            let label = item.label();
            // Value right-aligned to the row.
            let pad = COLS.saturating_sub(1 + label.len() + value.len());
            let line = format!("{mark}{label}{}{value}", " ".repeat(pad));
            text.draw(0, y, &line);
        }
        // Scroll marks.
        if self.scroll > 0 {
            text.draw(124, ROW_PX, "^");
        }
        if end < items.len() {
            text.draw(124, VISIBLE_ITEMS * ROW_PX, "V");
        }

        let footer = if self.rebinding {
            "ESC CANCEL"
        } else {
            match self.page {
                Page::Controls => "TAB PAGE  ENTER BIND  ESC BACK",
                Page::Map => "< > SET  ENTER ENGAGE  M CLOSE",
                _ => "TAB PAGE  < > ADJUST  ESC BACK",
            }
        };
        text.draw(0, (VISIBLE_ITEMS + 1) * ROW_PX, footer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cockpit::Slot;

    #[test]
    fn escape_closes_and_tab_pages() {
        let mut m = Menu::new();
        let mut s = Settings::default();
        m.toggle();
        assert!(m.open);
        m.key(KeyCode::Tab, &mut s);
        assert_eq!(m.page, Page::Controls);
        assert_eq!(m.key(KeyCode::Escape, &mut s), MenuEvent::Closed);
        assert!(!m.open);
    }

    #[test]
    fn msaa_only_offers_what_the_gpu_supports() {
        let mut m = Menu::new();
        let mut s = Settings::default();
        m.set_msaa_supported(&[1, 4]);
        m.toggle();
        // From 4, forward wraps past 8 (unsupported) to 1.
        assert_eq!(
            m.key(KeyCode::ArrowRight, &mut s),
            MenuEvent::Changed(Change::Graphics)
        );
        assert_eq!(s.msaa, 1);
        assert_eq!(
            m.key(KeyCode::ArrowRight, &mut s),
            MenuEvent::Changed(Change::Graphics)
        );
        assert_eq!(s.msaa, 4);
        // Only one choice: nothing to change.
        m.set_msaa_supported(&[4]);
        assert_eq!(m.key(KeyCode::ArrowRight, &mut s), MenuEvent::Nothing);
    }

    #[test]
    fn right_arrow_adjusts_graphics() {
        let mut m = Menu::new();
        let mut s = Settings::default();
        m.toggle();
        assert_eq!(
            m.key(KeyCode::ArrowRight, &mut s),
            MenuEvent::Changed(Change::Graphics)
        );
        assert_eq!(s.msaa, 8);
        m.key(KeyCode::ArrowDown, &mut s);
        m.key(KeyCode::ArrowLeft, &mut s);
        assert!((s.scale - 0.95).abs() < 1e-6);
    }

    #[test]
    fn rebinding_takes_the_next_key_and_refuses_reserved_ones() {
        let mut m = Menu::new();
        let mut s = Settings::default();
        m.toggle();
        m.key(KeyCode::Tab, &mut s); // controls, cursor on thrust-forward
        assert_eq!(m.key(KeyCode::Enter, &mut s), MenuEvent::Nothing);
        assert!(m.rebinding);
        assert_eq!(m.key(KeyCode::Tab, &mut s), MenuEvent::Nothing); // reserved: ignored
        assert!(m.rebinding);
        assert_eq!(
            m.key(KeyCode::KeyI, &mut s),
            MenuEvent::Changed(Change::Bindings)
        );
        assert_eq!(s.bindings.key_for(Action::ThrustForward), KeyCode::KeyI);
        assert!(!m.rebinding);
    }

    #[test]
    fn cockpit_page_cycles_slots_and_quit_quits() {
        let mut m = Menu::new();
        let mut s = Settings::default();
        m.toggle();
        m.key(KeyCode::Tab, &mut s);
        m.key(KeyCode::Tab, &mut s);
        m.key(KeyCode::Tab, &mut s); // gauges: style, stay, then the slots
        for _ in 0..2 {
            m.key(KeyCode::ArrowDown, &mut s);
        }
        assert_eq!(
            m.key(KeyCode::ArrowRight, &mut s),
            MenuEvent::Changed(Change::Layout)
        );
        assert_ne!(s.layout.get(Instrument::Speed), Slot::BottomRight);
        // The per-dial block at the page's end: select the gyro, size it.
        let items = m.items().len();
        for _ in 0..(items - 2 - 5) {
            m.key(KeyCode::ArrowDown, &mut s);
        }
        m.key(KeyCode::ArrowRight, &mut s); // DIAL: speed -> altitude
        assert_eq!(m.dial, Instrument::Altitude);
        m.key(KeyCode::ArrowDown, &mut s);
        m.key(KeyCode::ArrowRight, &mut s); // SIZE
        assert_eq!(s.dials[Instrument::Altitude as usize].size, 1.125);
        m.key(KeyCode::Tab, &mut s); // back to graphics
        for _ in 0..5 {
            m.key(KeyCode::ArrowDown, &mut s);
        }
        assert_eq!(m.key(KeyCode::Enter, &mut s), MenuEvent::Quit);
    }

    #[test]
    fn the_drive_panel_sets_the_plan_and_engages_and_never_pages() {
        // The settings menu has no MAP page any more: Tab cycles the three.
        let mut settings_menu = Menu::new();
        let mut s = Settings::default();
        settings_menu.toggle();
        for _ in 0..Page::ALL.len() {
            settings_menu.key(KeyCode::Tab, &mut s);
            assert!(!settings_menu.map_open());
        }
        let mut m = Menu::map_panel();
        m.toggle();
        assert!(m.map_open());
        m.key(KeyCode::Tab, &mut s);
        assert!(m.map_open(), "the drive panel has nowhere to page to");
        let d0 = s.plan.dest;
        assert_eq!(
            m.key(KeyCode::ArrowRight, &mut s),
            MenuEvent::Changed(Change::Layout)
        );
        assert_ne!(s.plan.dest, d0);
        m.key(KeyCode::ArrowDown, &mut s);
        let r0 = s.plan.safe_radii;
        m.key(KeyCode::ArrowRight, &mut s);
        assert!(s.plan.safe_radii > r0);
        m.key(KeyCode::ArrowDown, &mut s);
        assert_eq!(m.key(KeyCode::Enter, &mut s), MenuEvent::Engage);
        assert!(!m.open);
    }

    #[test]
    fn renders_within_the_bitmap() {
        let mut m = Menu::new();
        let s = Settings::default();
        m.toggle();
        m.key(KeyCode::Tab, &mut Settings::default());
        let mut t = TextBitmap::new();
        m.render(&mut t, &s);
        let (w, h) = t.used_extent();
        assert!(w <= 128 && h <= 64, "{w}x{h}");
        assert!(w > 60);
    }
}
