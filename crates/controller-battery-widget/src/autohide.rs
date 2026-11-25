//! Auto-hide when no controller is active.
//!
//! The card hides itself once every controller has been idle (or gone) for a
//! while and comes back as soon as a controller reports again. The logic is a
//! small state machine so it can be unit tested without windows, timers or a
//! real controller; `tray.rs` feeds it one tick per second.
//!
//! Visibility is always taken from the real window, never cached: the tray
//! thread shows the card when a controller wakes up, and a cached "hidden"
//! flag would drift out of sync after a fade or a manual toggle.

use std::time::{Duration, Instant};

/// How long the card stays visible after the last controller activity.
///
/// The device itself is already marked idle after `collector::STALE_AFTER`
/// (1 s), so this is the countdown before the fade: 3-2-1, then the fade.
pub const IDLE_BEFORE_HIDE: Duration = Duration::from_secs(3);

/// After a manual show the card is left alone for this long, so clicking the
/// tray icon never immediately fades back out.
pub const MANUAL_SHOW_HOLD: Duration = Duration::from_secs(5);

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
    /// Whether auto-show may bring the card back. Cleared by a manual hide.
    auto_show_allowed: bool,
    /// No hiding before this instant (set by a manual show).
    hold_until: Option<Instant>,
    idle_since: Option<Instant>,
}

impl AutoHide {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            auto_show_allowed: true,
            hold_until: None,
            idle_since: None,
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.idle_since = None;
            self.hold_until = None;
        }
    }

    /// The user asked to see the card (tray click, tray menu). Start a hold so
    /// the card does not fade out from under the click.
    pub fn note_manual_show(&mut self, now: Instant) {
        self.auto_show_allowed = true;
        self.hold_until = Some(now + MANUAL_SHOW_HOLD);
        self.idle_since = None;
    }

    /// The user hid the card; do not bring it back on the next report.
    pub fn note_manual_hide(&mut self) {
        self.auto_show_allowed = false;
        self.hold_until = None;
        self.idle_since = None;
    }

    /// The user clicked or dragged the card. Restart the hold so a running
    /// countdown is cancelled immediately, not only at the next timer tick.
    pub fn note_interaction(&mut self, now: Instant) {
        self.hold_until = Some(now + MANUAL_SHOW_HOLD);
        self.idle_since = None;
    }

    /// When the fade should start, if a countdown is running.
    pub fn deadline(&self) -> Option<Instant> {
        self.idle_since.map(|since| since + IDLE_BEFORE_HIDE)
    }

    /// Seconds left before the fade starts (`Some(0)` = about to hide).
    ///
    /// The UI derives the same values from [`Self::deadline`] so it can count
    /// on its own clock; this is the equivalent for tests and callers that
    /// only have an `Instant`.
    #[allow(dead_code)]
    pub fn countdown(&self, now: Instant) -> Option<u8> {
        if !self.enabled {
            return None;
        }
        let since = self.idle_since?;
        let elapsed = now.saturating_duration_since(since);
        if elapsed >= IDLE_BEFORE_HIDE {
            return Some(0);
        }
        let left = IDLE_BEFORE_HIDE - elapsed;
        Some(left.as_millis().div_ceil(1000).min(9) as u8)
    }

    /// One tick of the timer.
    ///
    /// `active` is true while at least one controller is reporting, `visible`
    /// is the real window visibility, `focused` says whether the user is
    /// interacting with the card and `ready` says whether the tray icon exists
    /// (without it a hidden card could not be recovered).
    pub fn tick(
        &mut self,
        now: Instant,
        active: bool,
        visible: bool,
        focused: bool,
        ready: bool,
    ) -> Action {
        if !self.enabled {
            self.idle_since = None;
            return Action::None;
        }

        if !visible {
            self.idle_since = None;
            if active && self.auto_show_allowed {
                return Action::Show;
            }
            return Action::None;
        }

        if active {
            self.idle_since = None;
            return Action::None;
        }

        // Never pull the card away from someone who is using it.
        if focused {
            self.idle_since = None;
            return Action::None;
        }

        // A just-shown card gets a short grace period instead of fading out
        // immediately when no controller is around.
        if let Some(until) = self.hold_until {
            if now < until {
                self.idle_since = None;
                return Action::None;
            }
            self.hold_until = None;
        }

        // Hiding without a tray icon would make the card unreachable.
        if !ready {
            self.idle_since = None;
            return Action::None;
        }

        let since = *self.idle_since.get_or_insert(now);
        if now.saturating_duration_since(since) >= IDLE_BEFORE_HIDE {
            self.idle_since = None;
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
    fn hides_after_the_countdown() {
        let base = Instant::now();
        let mut auto = AutoHide::new(true);

        assert_eq!(auto.tick(base, false, true, false, true), Action::None);
        assert_eq!(
            auto.tick(at(base, 2), false, true, false, true),
            Action::None
        );
        assert_eq!(
            auto.tick(at(base, 3), false, true, false, true),
            Action::Hide
        );
    }

    #[test]
    fn is_not_hidden_while_a_controller_is_active() {
        let base = Instant::now();
        let mut auto = AutoHide::new(true);

        for second in 0..30 {
            assert_eq!(
                auto.tick(at(base, second), true, true, false, true),
                Action::None
            );
        }
    }

    #[test]
    fn shows_again_when_a_controller_wakes_up_while_hidden() {
        let base = Instant::now();
        let mut auto = AutoHide::new(true);

        // Hidden window + activity = show, even if no tick happened before.
        assert_eq!(auto.tick(base, true, false, false, true), Action::Show);
    }

    #[test]
    fn a_manual_hide_is_not_undone_by_activity() {
        let base = Instant::now();
        let mut auto = AutoHide::new(true);
        auto.note_manual_hide();

        assert_eq!(auto.tick(base, true, false, false, true), Action::None);
        assert_eq!(
            auto.tick(at(base, 1), true, false, false, true),
            Action::None
        );
    }

    #[test]
    fn manual_show_starts_a_hold() {
        let base = Instant::now();
        let mut auto = AutoHide::new(true);
        auto.note_manual_show(base);

        // Still visible, nothing connected: the hold keeps it on screen.
        for second in 0..MANUAL_SHOW_HOLD.as_secs() {
            assert_eq!(
                auto.tick(at(base, second), false, true, false, true),
                Action::None
            );
            assert_eq!(auto.countdown(at(base, second)), None);
        }

        // After the hold the normal countdown resumes.
        assert_eq!(
            auto.tick(at(base, 5), false, true, false, true),
            Action::None
        );
        assert_eq!(auto.countdown(at(base, 5)), Some(3));
        assert_eq!(
            auto.tick(at(base, 8), false, true, false, true),
            Action::Hide
        );
    }

    #[test]
    fn interaction_cancels_the_countdown() {
        let base = Instant::now();
        let mut auto = AutoHide::new(true);

        assert_eq!(auto.tick(base, false, true, false, true), Action::None);
        assert_eq!(auto.countdown(base), Some(3));

        auto.note_interaction(at(base, 1));
        assert_eq!(auto.countdown(at(base, 1)), None);
        // The hold keeps it visible for a while.
        assert_eq!(
            auto.tick(at(base, 2), false, true, false, true),
            Action::None
        );
        assert_eq!(auto.countdown(at(base, 2)), None);
    }

    #[test]
    fn focus_keeps_the_card_visible() {
        let base = Instant::now();
        let mut auto = AutoHide::new(true);

        for second in 0..30 {
            assert_eq!(
                auto.tick(at(base, second), false, true, true, true),
                Action::None
            );
        }

        assert_eq!(
            auto.tick(at(base, 30), false, true, false, true),
            Action::None
        );
        assert_eq!(
            auto.tick(at(base, 33), false, true, false, true),
            Action::Hide
        );
    }

    #[test]
    fn never_hides_without_a_tray_icon() {
        let base = Instant::now();
        let mut auto = AutoHide::new(true);

        for second in 0..30 {
            assert_eq!(
                auto.tick(at(base, second), false, true, false, false),
                Action::None
            );
        }
    }

    #[test]
    fn disabled_does_nothing() {
        let base = Instant::now();
        let mut auto = AutoHide::new(false);

        for second in 0..30 {
            assert_eq!(
                auto.tick(at(base, second), false, true, false, true),
                Action::None
            );
        }
    }

    #[test]
    fn countdown_counts_down_and_stops_when_hidden() {
        let base = Instant::now();
        let mut auto = AutoHide::new(true);

        assert_eq!(auto.countdown(base), None);
        assert_eq!(auto.tick(base, false, true, false, true), Action::None);
        assert_eq!(auto.countdown(base), Some(3));
        assert_eq!(auto.countdown(at(base, 1)), Some(2));
        assert_eq!(
            auto.countdown(base + IDLE_BEFORE_HIDE - Duration::from_millis(1)),
            Some(1)
        );
        assert_eq!(auto.countdown(base + IDLE_BEFORE_HIDE), Some(0));

        assert_eq!(
            auto.tick(at(base, 3), false, true, false, true),
            Action::Hide
        );
        assert_eq!(auto.countdown(at(base, 3)), None);
    }
}
