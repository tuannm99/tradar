//! Shared vim-style navigation for any component that renders a selectable
//! list: `j`/`k`/arrows, `gg`/`G`, `Ctrl-d`/`Ctrl-u`. Used by
//! `ConnectionPickerComponent`, `SchemaSidebarComponent`,
//! `ResultsComponent`, and `HistoryPickerComponent` -- this is the one
//! module all four actually share (`tradar-core` is the only crate every
//! other crate in the workspace depends on), so the movement math and the
//! `gg` double-tap detection live in exactly one place instead of being
//! copy-pasted per component.
//!
//! Two-step API by design: `recognize` turns a key event into an intent
//! (or `None`, including while a lone `g` is pending a second one),
//! `apply` turns an intent into a mutation on the caller's own `selected`/
//! `visible_height` state. Splitting them keeps every component free to
//! keep its own field names (`selected`, `schema_selected`, ...) instead of
//! being forced onto one shared struct type.

use crossterm::event::{KeyCode, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VimMove {
    Down,
    Up,
    Top,
    Bottom,
    HalfPageDown,
    HalfPageUp,
}

/// Recognizes a navigation key, given `pending_g` -- the caller's own
/// per-list "a lone `g` is waiting for a second one" flag. Consumed via
/// `std::mem::take` on every call regardless of which key comes in, so any
/// key other than a second `g` cancels a dangling one, matching vim.
pub fn recognize(code: KeyCode, modifiers: KeyModifiers, pending_g: &mut bool) -> Option<VimMove> {
    let had_pending_g = std::mem::take(pending_g);
    match code {
        KeyCode::Down | KeyCode::Char('j') => Some(VimMove::Down),
        KeyCode::Up | KeyCode::Char('k') => Some(VimMove::Up),
        KeyCode::Char('g') if had_pending_g => Some(VimMove::Top),
        KeyCode::Char('g') => {
            *pending_g = true;
            None
        }
        KeyCode::Char('G') => Some(VimMove::Bottom),
        KeyCode::Char('d') if modifiers.contains(KeyModifiers::CONTROL) => {
            Some(VimMove::HalfPageDown)
        }
        KeyCode::Char('u') if modifiers.contains(KeyModifiers::CONTROL) => {
            Some(VimMove::HalfPageUp)
        }
        _ => None,
    }
}

fn move_down_by(selected: &mut usize, len: usize, delta: usize) {
    if len == 0 {
        return;
    }
    *selected = (*selected + delta).min(len - 1);
}

fn move_up_by(selected: &mut usize, delta: usize) {
    *selected = selected.saturating_sub(delta);
}

/// Half the last-rendered visible row count, minimum 1 -- matches vim's
/// `Ctrl-d`/`Ctrl-u`.
fn half_page(visible_height: usize) -> usize {
    (visible_height / 2).max(1)
}

/// Applies a recognized move to `selected`, clamped to `[0, len)` (or left
/// at 0 when `len` is 0).
pub fn apply(mv: VimMove, selected: &mut usize, len: usize, visible_height: usize) {
    match mv {
        VimMove::Down => move_down_by(selected, len, 1),
        VimMove::Up => move_up_by(selected, 1),
        VimMove::Top => *selected = 0,
        VimMove::Bottom => *selected = len.saturating_sub(1),
        VimMove::HalfPageDown => move_down_by(selected, len, half_page(visible_height)),
        VimMove::HalfPageUp => move_up_by(selected, half_page(visible_height)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn j_and_down_arrow_both_recognize_as_down() {
        let mut pending_g = false;
        assert_eq!(
            recognize(KeyCode::Char('j'), KeyModifiers::NONE, &mut pending_g),
            Some(VimMove::Down)
        );
        assert_eq!(
            recognize(KeyCode::Down, KeyModifiers::NONE, &mut pending_g),
            Some(VimMove::Down)
        );
    }

    #[test]
    fn a_lone_g_is_not_recognized_yet_but_sets_pending() {
        let mut pending_g = false;

        let result = recognize(KeyCode::Char('g'), KeyModifiers::NONE, &mut pending_g);

        assert_eq!(result, None);
        assert!(pending_g);
    }

    #[test]
    fn gg_recognizes_as_top() {
        let mut pending_g = false;
        recognize(KeyCode::Char('g'), KeyModifiers::NONE, &mut pending_g);

        let result = recognize(KeyCode::Char('g'), KeyModifiers::NONE, &mut pending_g);

        assert_eq!(result, Some(VimMove::Top));
        assert!(!pending_g, "pending_g must be consumed after completing gg");
    }

    #[test]
    fn any_other_key_cancels_a_pending_g() {
        let mut pending_g = false;
        recognize(KeyCode::Char('g'), KeyModifiers::NONE, &mut pending_g);

        let result = recognize(KeyCode::Char('k'), KeyModifiers::NONE, &mut pending_g);

        assert_eq!(result, Some(VimMove::Up));
        assert!(!pending_g);

        // A fresh 'g' afterwards starts a new pair, not a leftover one.
        let stale_g = recognize(KeyCode::Char('g'), KeyModifiers::NONE, &mut pending_g);
        assert_eq!(stale_g, None);
        assert!(pending_g);
    }

    #[test]
    fn shift_g_recognizes_as_bottom() {
        let mut pending_g = false;
        assert_eq!(
            recognize(KeyCode::Char('G'), KeyModifiers::NONE, &mut pending_g),
            Some(VimMove::Bottom)
        );
    }

    #[test]
    fn ctrl_d_and_ctrl_u_recognize_as_half_page_moves() {
        let mut pending_g = false;
        assert_eq!(
            recognize(KeyCode::Char('d'), KeyModifiers::CONTROL, &mut pending_g),
            Some(VimMove::HalfPageDown)
        );
        assert_eq!(
            recognize(KeyCode::Char('u'), KeyModifiers::CONTROL, &mut pending_g),
            Some(VimMove::HalfPageUp)
        );
    }

    #[test]
    fn plain_d_and_u_without_control_are_not_recognized() {
        let mut pending_g = false;
        assert_eq!(
            recognize(KeyCode::Char('d'), KeyModifiers::NONE, &mut pending_g),
            None
        );
        assert_eq!(
            recognize(KeyCode::Char('u'), KeyModifiers::NONE, &mut pending_g),
            None
        );
    }

    #[test]
    fn move_down_stops_at_the_last_index() {
        let mut selected = 0;
        apply(VimMove::Down, &mut selected, 3, 0);
        assert_eq!(selected, 1);
        apply(VimMove::Down, &mut selected, 3, 0);
        assert_eq!(selected, 2);
        apply(VimMove::Down, &mut selected, 3, 0);
        assert_eq!(selected, 2, "must stop at the last index, not wrap");
    }

    #[test]
    fn move_down_on_an_empty_list_is_a_no_op() {
        let mut selected = 0;
        apply(VimMove::Down, &mut selected, 0, 0);
        assert_eq!(selected, 0);
    }

    #[test]
    fn move_up_stops_at_zero() {
        let mut selected = 1;
        apply(VimMove::Up, &mut selected, 3, 0);
        assert_eq!(selected, 0);
        apply(VimMove::Up, &mut selected, 3, 0);
        assert_eq!(selected, 0, "must stop at zero, not go negative");
    }

    #[test]
    fn top_and_bottom_jump_straight_there() {
        let mut selected = 1;
        apply(VimMove::Bottom, &mut selected, 5, 0);
        assert_eq!(selected, 4);
        apply(VimMove::Top, &mut selected, 5, 0);
        assert_eq!(selected, 0);
    }

    #[test]
    fn bottom_on_an_empty_list_stays_at_zero() {
        let mut selected = 0;
        apply(VimMove::Bottom, &mut selected, 0, 0);
        assert_eq!(selected, 0);
    }

    #[test]
    fn half_page_moves_use_half_the_visible_height_minimum_one() {
        let mut selected = 0;
        apply(VimMove::HalfPageDown, &mut selected, 10, 12);
        assert_eq!(selected, 6, "half of 12 visible rows");

        apply(VimMove::HalfPageUp, &mut selected, 10, 12);
        assert_eq!(selected, 0);

        // visible_height 0 (before the first draw()) must still move by at
        // least 1, not 0.
        let mut selected = 0;
        apply(VimMove::HalfPageDown, &mut selected, 10, 0);
        assert_eq!(selected, 1);
    }
}
