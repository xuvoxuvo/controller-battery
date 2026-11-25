//! Auto-hide when no controller is active.
//!
//! The card hides itself once every controller has been idle (or gone) for a
//! while and comes back as soon as a controller reports again. The logic is a
//! small state machine so it can be unit tested without windows, timers or a
//! real controller; `tray.rs` feeds it one tick per second.
//!
//! Manual actions win: showing the card by hand keeps it visible until a
//! controller becomes active again, and hiding it by hand never pops it back.

use std::time::{Duration, Instant};

/// How long the card stays visible after the last controller activity.
pub const IDLE_BEFORE_HIDE: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    /// Hide the card (we are allowed to bring it back).
    Hide,
    /// Show the card again.
    Show,
}

#[derive(Debug)]
pub struct AutoHide {
    enabled: bool,
    /// Hidden by this state machine, i.e. it may show the card again.
    hidden: bool,
    /// Allowed to hide on the next idle period. Cleared by a manual show so the
    /// card does not vanish from under the user's cursor.
    armed: bool,
    was_active: bool,
    idle_since: Option<Instant>,
}

impl AutoHide {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            hidden: false,
            armed: enabled,
            was_active: false,
            idle_since: None,
        }
    }

    /// Turn the feature on or off. Disabling brings back a card we hid.
    pub fn set_enabled(&mut self, enabled: bool) -> Action {
        self.enabled = enabled;
        if enabled {
            self.armed = true;
            return Action::None;
        }

        self.armed = false;
        self.idle_since = None;
        if self.hidden {
            self.hidden = false;
            Action::Show
        } else {
            Action::None
        }
    }

    /// The user asked to see the card (tray click, tray menu). Auto-hide stays
    /// out of the way until a controller becomes active again.
    pub fn note_manual_show(&mut self) {
        self.hidden = false;
        self.armed = false;
        self.idle_since = None;
    }

    /// The user hid the card; never pop it back on the next report.
    pub fn note_manual_hide(&mut self) {
        self.hidden = false;
        self.armed = false;
        self.idle_since = None;
    }

    /// One tick of the timer.
    ///
    /// `any_active` is true while at least one controller is reporting,
    /// `visible` is the real window visibility and `ready` says whether the
    /// tray icon exists (without it a hidden card cannot be recovered).
    pub fn tick(&mut self, now: Instant, any_active: bool, visible: bool, ready: bool) -> Action {
        // A controller showing up re-arms hiding after a manual show.
        if any_active && !self.was_active {
            self.armed = true;
            self.idle_since = None;
        }
        self.was_active = any_active;

        if !self.enabled {
            self.idle_since = None;
            return Action::None;
        }

        if self.hidden {
            if any_active {
                self.hidden = false;
                return Action::Show;
            }
            return Action::None;
        }

        if any_active {
            self.idle_since = None;
            return Action::None;
        }

        // A hidden-by-hand card is not ours to hide again, and hiding without a
        // tray icon would make the card unreachable.
        if !self.armed || !ready || !visible {
            self.idle_since = None;
            return Action::None;
        }

        let since = *self.idle_since.get_or_insert(now);
        if now.duration_since(since) >= IDLE_BEFORE_HIDE {
            self.idle_since = None;
            self.hidden = true;
            return Action::Hide;
        }
        Action::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(base: Instant, seconds: u64) -> Instant {
        base + Duration::from_secs(seconds)
    }

    #[test]
    fn hides_after_the_grace_period() {
        let base = Instant::now();
        let mut auto = AutoHide::new(true);

        assert_eq!(auto.tick(base, false, true, true), Action::None);
        assert_eq!(auto.tick(at(base, 4), false, true, true), Action::None);
        assert_eq!(auto.tick(at(base, 8), false, true, true), Action::Hide);
    }

    #[test]
    fn stays_visible_while_a_controller_is_active() {
        let base = Instant::now();
        let mut auto = AutoHide::new(true);

        for second in 0..30 {
            assert_eq!(auto.tick(at(base, second), true, true, true), Action::None);
        }
    }

    #[test]
    fn shows_again_when_a_controller_wakes_up() {
        let base = Instant::now();
        let mut auto = AutoHide::new(true);

        assert_eq!(auto.tick(base, false, true, true), Action::None);
        assert_eq!(auto.tick(at(base, 8), false, true, true), Action::Hide);
        // Hidden now; activity brings it back exactly once.
        assert_eq!(auto.tick(at(base, 9), true, false, true), Action::Show);
        assert_eq!(auto.tick(at(base, 10), true, true, true), Action::None);
    }

    #[test]
    fn manual_show_keeps_the_card_visible_until_the_next_activity() {
        let base = Instant::now();
        let mut auto = AutoHide::new(true);

        assert_eq!(auto.tick(base, false, true, true), Action::None);
        assert_eq!(auto.tick(at(base, 8), false, true, true), Action::Hide);

        // The user clicks the tray icon while nothing is connected.
        auto.note_manual_show();
        for second in 9..40 {
            assert_eq!(auto.tick(at(base, second), false, true, true), Action::None);
        }

        // A controller reports, then goes idle again: hiding re-arms.
        assert_eq!(auto.tick(at(base, 40), true, true, true), Action::None);
        assert_eq!(auto.tick(at(base, 41), false, true, true), Action::None);
        assert_eq!(auto.tick(at(base, 49), false, true, true), Action::Hide);
    }

    #[test]
    fn manual_hide_is_not_undone_by_activity() {
        let base = Instant::now();
        let mut auto = AutoHide::new(true);
        auto.note_manual_hide();

        assert_eq!(auto.tick(at(base, 0), false, false, true), Action::None);
        // The controller wakes while the user has the card hidden by hand.
        assert_eq!(auto.tick(at(base, 1), true, false, true), Action::None);
        assert_eq!(auto.tick(at(base, 2), true, false, true), Action::None);
    }

    #[test]
    fn never_hides_without_a_tray_icon() {
        let base = Instant::now();
        let mut auto = AutoHide::new(true);

        for second in 0..30 {
            assert_eq!(
                auto.tick(at(base, second), false, true, false),
                Action::None
            );
        }
    }

    #[test]
    fn does_nothing_while_disabled() {
        let base = Instant::now();
        let mut auto = AutoHide::new(false);

        for second in 0..30 {
            assert_eq!(auto.tick(at(base, second), false, true, true), Action::None);
        }
        assert_eq!(auto.set_enabled(false), Action::None);
    }

    #[test]
    fn disabling_brings_back_a_card_we_hid() {
        let base = Instant::now();
        let mut auto = AutoHide::new(true);

        assert_eq!(auto.tick(base, false, true, true), Action::None);
        assert_eq!(auto.tick(at(base, 8), false, true, true), Action::Hide);
        assert_eq!(auto.set_enabled(false), Action::Show);
        assert_eq!(auto.tick(at(base, 9), false, true, true), Action::None);
    }

    #[test]
    fn enabling_hides_an_idle_card() {
        let base = Instant::now();
        let mut auto = AutoHide::new(false);

        assert_eq!(auto.set_enabled(true), Action::None);
        assert_eq!(auto.tick(base, false, true, true), Action::None);
        assert_eq!(auto.tick(at(base, 8), false, true, true), Action::Hide);
    }
}
