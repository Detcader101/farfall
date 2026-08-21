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

use crate::cockpit::Instrument;
use crate::input::{is_reserved, key_name, Action};
use crate::settings::{Settings, MSAA_CHOICES};
use farfall_render::text::TextBitmap;
use winit::keyboard::KeyCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Graphics,
    Controls,
    Cockpit,
}

impl Page {
    const ALL: [Page; 3] = [Page::Graphics, Page::Controls, Page::Cockpit];

    fn name(self) -> &'static str {
        match self {
            Page::Graphics => "GRAPHICS",
            Page::Controls => "CONTROLS",
            Page::Cockpit => "COCKPIT",
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
    Slot(Instrument),
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
            Item::Slot(i) => i.name(),
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
            Item::Slot(i) => s.layout.get(i).name().to_string(),
        }
    }

    fn rebindable(self) -> bool {
        matches!(self, Item::Bind(_) | Item::BindBoost | Item::BindBrake)
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
    page: Page,
    cursor: usize,
    scroll: usize,
    /// Waiting for a key to bind to the item under the cursor.
    rebinding: bool,
}

impl Default for Menu {
    fn default() -> Self {
        Self {
            open: false,
            page: Page::Graphics,
            cursor: 0,
            scroll: 0,
            rebinding: false,
        }
    }
}

impl Menu {
    pub fn new() -> Self {
        Self::default()
    }

    fn items(&self) -> Vec<Item> {
        match self.page {
            Page::Graphics => vec![Item::Msaa, Item::Scale, Item::Vsync, Item::Quit],
            Page::Controls => {
                let mut v: Vec<Item> = Action::ALL.iter().map(|&a| Item::Bind(a)).collect();
                v.push(Item::BindBoost);
                v.push(Item::BindBrake);
                v
            }
            Page::Cockpit => Instrument::ALL.iter().map(|&i| Item::Slot(i)).collect(),
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
            KeyCode::Tab => {
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
                let i = MSAA_CHOICES.iter().position(|&m| m == s.msaa).unwrap_or(2);
                let n = MSAA_CHOICES.len();
                s.msaa = MSAA_CHOICES[if forward {
                    (i + 1) % n
                } else {
                    (i + n - 1) % n
                }];
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
            Item::Quit | Item::Bind(_) | Item::BindBoost | Item::BindBrake => MenuEvent::Nothing,
        }
    }

    fn keep_cursor_visible(&mut self) {
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor >= self.scroll + VISIBLE_ITEMS {
            self.scroll = self.cursor + 1 - VISIBLE_ITEMS;
        }
    }

    /// Draw the menu into the text bitmap.
    pub fn render(&self, text: &mut TextBitmap, s: &Settings) {
        text.clear();
        // Header: the pages, the current one bracketed.
        let mut header = String::new();
        for p in Page::ALL {
            if p == self.page {
                header.push_str(&format!("[{}]", p.name()));
            } else {
                header.push_str(&format!(" {} ", p.name()));
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
                item.value(s)
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
        m.key(KeyCode::Tab, &mut s); // cockpit
        assert_eq!(
            m.key(KeyCode::ArrowRight, &mut s),
            MenuEvent::Changed(Change::Layout)
        );
        assert_ne!(s.layout.get(Instrument::Speed), Slot::BottomRight);
        m.key(KeyCode::Tab, &mut s); // back to graphics
        for _ in 0..3 {
            m.key(KeyCode::ArrowDown, &mut s);
        }
        assert_eq!(m.key(KeyCode::Enter, &mut s), MenuEvent::Quit);
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
