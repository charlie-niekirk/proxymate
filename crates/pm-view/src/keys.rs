//! The flow table's keys, ported from `FlowTable/keys.ts`: a pure function
//! of the key and where the table stands, so they are tested without a
//! window (the table in `keys.test.ts`, below).

/// The keys the table and its resize handles look at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    Home,
    End,
    Enter,
    Escape,
    Backspace,
    /// Any other key, which the table leaves alone.
    Other,
}

impl Key {
    /// A gpui keystroke's `key` (`"down"`, `"pagedown"`, `"enter"`, …).
    pub fn from_name(name: &str) -> Key {
        match name {
            "up" => Key::Up,
            "down" => Key::Down,
            "left" => Key::Left,
            "right" => Key::Right,
            "pageup" => Key::PageUp,
            "pagedown" => Key::PageDown,
            "home" => Key::Home,
            "end" => Key::End,
            "enter" => Key::Enter,
            "escape" => Key::Escape,
            "backspace" => Key::Backspace,
            _ => Key::Other,
        }
    }
}

/// A key with its modifiers (`cmd` is ⌘ on macOS, the platform key).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TableKey {
    pub key: Option<Key>,
    pub shift: bool,
    pub cmd: bool,
    pub ctrl: bool,
    pub alt: bool,
}

impl TableKey {
    pub fn new(key: Key) -> Self {
        Self {
            key: Some(key),
            ..Default::default()
        }
    }

    pub fn shift(mut self) -> Self {
        self.shift = true;
        self
    }

    pub fn cmd(mut self) -> Self {
        self.cmd = true;
        self
    }

    pub fn ctrl(mut self) -> Self {
        self.ctrl = true;
        self
    }

    pub fn alt(mut self) -> Self {
        self.alt = true;
        self
    }
}

/// Where the table stands when a key arrives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TableState {
    /// The focused flow's row index, if it is known.
    pub from: Option<usize>,
    /// The first and last rendered rows.
    pub first: usize,
    pub last: usize,
    /// Rows in the view.
    pub len: usize,
    /// Rows per page (PgUp/PgDn).
    pub page: usize,
    /// More than one flow is selected.
    pub multi: bool,
    /// A flow is focused.
    pub selected: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableAction {
    /// Select row `to` (clamped to the view); with ⇧, extend the selection to it.
    Move { to: usize, extend: bool },
    /// Enter: into the inspector.
    Inspect,
    /// Escape in a multi-selection: back to the focused flow alone.
    Collapse,
    /// Handled, nothing to do (Enter with nothing selected).
    None,
}

/// What `e` does to the table, or `None` when the table leaves it alone (any
/// key with ⌘, Ctrl or ⌥ belongs to the app's commands).
pub fn table_key(e: TableKey, s: TableState) -> Option<TableAction> {
    if e.cmd || e.ctrl || e.alt {
        return None;
    }
    let key = e.key?;
    match key {
        Key::Escape => return s.multi.then_some(TableAction::Collapse),
        Key::Enter => {
            return Some(if s.selected {
                TableAction::Inspect
            } else {
                TableAction::None
            });
        }
        _ => {}
    }
    let target = move_target(key, s)?;
    if s.len == 0 {
        return Some(TableAction::None);
    }
    Some(TableAction::Move {
        to: target.clamp(0, s.len as i64 - 1) as usize,
        extend: e.shift,
    })
}

fn move_target(key: Key, s: TableState) -> Option<i64> {
    let from = s.from.map(|f| f as i64);
    let (first, last, len, page) = (s.first as i64, s.last as i64, s.len as i64, s.page as i64);
    Some(match key {
        Key::Down => from.map_or(first, |f| f + 1),
        Key::Up => from.map_or(last, |f| f - 1),
        Key::PageDown => from.unwrap_or(first) + page,
        Key::PageUp => from.unwrap_or(first) - page,
        Key::Home => 0,
        Key::End => len - 1,
        _ => return None,
    })
}

/// A column's width limits for [`resize_key`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResizeColumn {
    /// Its current width.
    pub px: f32,
    pub min_width: f32,
    /// Its default width.
    pub width: f32,
}

/// A column resize handle's keys: ←/→ by 8 px (⇧: 32), Home to the minimum,
/// End to `max`, ↩ to the default.
pub fn resize_key(e: TableKey, col: ResizeColumn, max: f32) -> Option<f32> {
    let step = if e.shift { 32.0 } else { 8.0 };
    let clamp = |w: f32| w.min(max).max(col.min_width);
    Some(match e.key? {
        Key::Left => clamp(col.px - step),
        Key::Right => clamp(col.px + step),
        Key::Home => col.min_width,
        Key::End => max,
        Key::Enter => clamp(col.width),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> TableState {
        TableState {
            from: Some(10),
            first: 5,
            last: 40,
            len: 100,
            page: 20,
            multi: false,
            selected: true,
        }
    }

    fn mv(to: usize, extend: bool) -> Option<TableAction> {
        Some(TableAction::Move { to, extend })
    }

    #[test]
    fn keys_move_to_their_rows() {
        for (key, to) in [
            (Key::Down, 11),
            (Key::Up, 9),
            (Key::PageDown, 30),
            (Key::PageUp, 0),
            (Key::Home, 0),
            (Key::End, 99),
        ] {
            assert_eq!(table_key(TableKey::new(key), state()), mv(to, false), "{key:?}");
        }
    }

    #[test]
    fn starts_from_the_rendered_rows_when_nothing_is_focused() {
        let s = TableState { from: None, ..state() };
        assert_eq!(table_key(TableKey::new(Key::Down), s), mv(5, false));
        assert_eq!(table_key(TableKey::new(Key::Up), s), mv(40, false));
        assert_eq!(table_key(TableKey::new(Key::PageDown), s), mv(25, false));
    }

    #[test]
    fn clamps_to_the_view() {
        let at = |from| TableState {
            from: Some(from),
            ..state()
        };
        assert_eq!(table_key(TableKey::new(Key::Down), at(99)), mv(99, false));
        assert_eq!(table_key(TableKey::new(Key::Up), at(0)), mv(0, false));
        assert_eq!(table_key(TableKey::new(Key::PageDown), at(95)), mv(99, false));
    }

    #[test]
    fn extends_the_selection_with_shift() {
        assert_eq!(table_key(TableKey::new(Key::Down).shift(), state()), mv(11, true));
        assert_eq!(table_key(TableKey::new(Key::End).shift(), state()), mv(99, true));
    }

    #[test]
    fn does_nothing_to_an_empty_view_but_still_takes_the_key() {
        let s = TableState {
            len: 0,
            from: None,
            ..state()
        };
        assert_eq!(table_key(TableKey::new(Key::Down), s), Some(TableAction::None));
    }

    #[test]
    fn enter_goes_to_the_inspector_only_with_a_flow_focused() {
        assert_eq!(
            table_key(TableKey::new(Key::Enter), state()),
            Some(TableAction::Inspect)
        );
        let none = TableState {
            selected: false,
            ..state()
        };
        assert_eq!(table_key(TableKey::new(Key::Enter), none), Some(TableAction::None));
    }

    #[test]
    fn escape_collapses_a_multi_selection_and_is_left_alone_otherwise() {
        let multi = TableState { multi: true, ..state() };
        assert_eq!(
            table_key(TableKey::new(Key::Escape), multi),
            Some(TableAction::Collapse)
        );
        assert_eq!(table_key(TableKey::new(Key::Escape), state()), None);
    }

    #[test]
    fn leaves_cmd_ctrl_and_alt_keys_and_other_keys_to_the_app() {
        assert_eq!(table_key(TableKey::new(Key::Down).cmd(), state()), None);
        assert_eq!(table_key(TableKey::new(Key::Down).ctrl(), state()), None);
        assert_eq!(table_key(TableKey::new(Key::Down).alt(), state()), None);
        assert_eq!(table_key(TableKey::new(Key::Other), state()), None);
        assert_eq!(table_key(TableKey::new(Key::Left), state()), None);
        assert_eq!(table_key(TableKey::default(), state()), None);
    }

    const COL: ResizeColumn = ResizeColumn {
        px: 100.0,
        min_width: 40.0,
        width: 120.0,
    };

    #[test]
    fn resize_steps_by_8_px_32_with_shift_within_the_limits() {
        assert_eq!(resize_key(TableKey::new(Key::Right), COL, 600.0), Some(108.0));
        assert_eq!(resize_key(TableKey::new(Key::Left), COL, 600.0), Some(92.0));
        assert_eq!(resize_key(TableKey::new(Key::Left).shift(), COL, 600.0), Some(68.0));
        let narrow = ResizeColumn { px: 44.0, ..COL };
        assert_eq!(resize_key(TableKey::new(Key::Left), narrow, 600.0), Some(40.0));
        let wide = ResizeColumn { px: 598.0, ..COL };
        assert_eq!(resize_key(TableKey::new(Key::Right), wide, 600.0), Some(600.0));
    }

    #[test]
    fn resize_home_end_and_enter_go_to_the_minimum_maximum_and_default() {
        assert_eq!(resize_key(TableKey::new(Key::Home), COL, 600.0), Some(40.0));
        assert_eq!(resize_key(TableKey::new(Key::End), COL, 600.0), Some(600.0));
        assert_eq!(resize_key(TableKey::new(Key::Enter), COL, 600.0), Some(120.0));
    }

    #[test]
    fn resize_ignores_other_keys() {
        assert_eq!(resize_key(TableKey::new(Key::Up), COL, 600.0), None);
    }

    #[test]
    fn gpui_key_names_map_to_keys() {
        assert_eq!(Key::from_name("pagedown"), Key::PageDown);
        assert_eq!(Key::from_name("backspace"), Key::Backspace);
        assert_eq!(Key::from_name("a"), Key::Other);
    }
}
