//! The auto-show mode's bookkeeping: what niri's focus was last pointed at, and
//! what is waiting to be sent. Plain state and plain rules, so they can be unit
//! tested on their own.

/// What a focus points at: the selected window, or the selected workspace when it
/// holds no windows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target {
    Window(u64),
    Workspace(u64),
}

/// The target for a selection: its window if it has one, else its workspace.
pub fn target_for(win_id: Option<u64>, ws_id: Option<u64>) -> Option<Target> {
    match (win_id, ws_id) {
        (Some(id), _) => Some(Target::Window(id)),
        (None, Some(id)) => Some(Target::Workspace(id)),
        (None, None) => None,
    }
}

/// Tracks the mode and debounces the focus calls it makes.
///
/// `last_sent` is where niri's focus is believed to be, so a selection that
/// returns there costs nothing. `pending` is a target that differs from it and is
/// waiting for the debounce to elapse; `scheduled` says a timer is in flight, so
/// navigating repeatedly arms only one. `scheduled` tracks the timer itself and so
/// outlives a disable, which keeps a re-enable from arming a second one.
#[derive(Default)]
pub struct AutoShow {
    on: bool,
    last_sent: Option<Target>,
    pending: Option<Target>,
    scheduled: bool,
}

impl AutoShow {
    pub fn is_on(&self) -> bool {
        self.on
    }

    /// Arm the mode, seeded with niri's current focus. Seeding is what makes the
    /// mode's promise "niri's focus equals the selection": a target niri already
    /// holds is left alone, so navigating away and back is free.
    pub fn enable(&mut self, focused: Option<Target>) {
        self.on = true;
        self.last_sent = focused;
        self.pending = None;
    }

    /// Disarm the mode and drop anything waiting. A timer that is already running
    /// finds nothing to send.
    pub fn disable(&mut self) {
        self.on = false;
        self.last_sent = None;
        self.pending = None;
    }

    /// Offer the selection's target. Returns true when the caller should start a
    /// debounce timer; a target niri is already focused on is dropped, and so is
    /// a target that arrives while a timer is running (it replaces the pending
    /// one, so the latest selection wins).
    pub fn queue(&mut self, target: Target) -> bool {
        if !self.on {
            return false;
        }
        if Some(target) == self.last_sent {
            self.pending = None;
            return false;
        }
        self.pending = Some(target);
        if self.scheduled {
            return false;
        }
        self.scheduled = true;
        true
    }

    /// The target to focus now, recorded as sent. Recording happens whether or
    /// not the focus call then succeeds: a window that died during the debounce
    /// is not worth retrying.
    pub fn take_due(&mut self) -> Option<Target> {
        self.scheduled = false;
        let target = self.pending.take()?;
        if Some(target) == self.last_sent {
            return None;
        }
        self.last_sent = Some(target);
        Some(target)
    }

    /// Record a target that something else focused, so the guard keeps matching
    /// niri. Without this a target focused by hand looks unfocused, and the next
    /// navigation back to it would be dropped as redundant.
    pub fn record_sent(&mut self, target: Target) {
        if self.on {
            self.last_sent = Some(target);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_prefers_the_window_over_its_workspace() {
        assert_eq!(target_for(Some(7), Some(3)), Some(Target::Window(7)));
        assert_eq!(target_for(None, Some(3)), Some(Target::Workspace(3)));
        assert_eq!(target_for(None, None), None);
    }

    #[test]
    fn a_window_and_a_workspace_sharing_an_id_are_different_targets() {
        assert_ne!(Target::Window(1), Target::Workspace(1));
    }

    #[test]
    fn off_by_default_and_queues_nothing() {
        let mut a = AutoShow::default();
        assert!(!a.is_on());
        assert!(!a.queue(Target::Window(1)));
        assert_eq!(a.take_due(), None);
    }

    #[test]
    fn enabling_sends_nothing_by_itself() {
        let mut a = AutoShow::default();
        a.enable(Some(Target::Window(1)));
        assert!(a.is_on());
        assert_eq!(a.take_due(), None);
    }

    #[test]
    fn the_seeded_focus_is_not_re_sent() {
        let mut a = AutoShow::default();
        a.enable(Some(Target::Window(1)));
        assert!(!a.queue(Target::Window(1)));
        assert_eq!(a.take_due(), None);
    }

    #[test]
    fn a_new_target_arms_a_timer_and_is_sent_once() {
        let mut a = AutoShow::default();
        a.enable(Some(Target::Window(1)));
        assert!(a.queue(Target::Window(2)));
        assert_eq!(a.take_due(), Some(Target::Window(2)));
        assert_eq!(a.take_due(), None);
    }

    #[test]
    fn a_target_already_sent_is_not_sent_again() {
        let mut a = AutoShow::default();
        a.enable(None);
        a.queue(Target::Workspace(5));
        assert_eq!(a.take_due(), Some(Target::Workspace(5)));
        assert!(!a.queue(Target::Workspace(5)));
        assert_eq!(a.take_due(), None);
    }

    #[test]
    fn bursts_arm_one_timer_and_send_only_the_latest() {
        let mut a = AutoShow::default();
        a.enable(Some(Target::Window(1)));
        assert!(a.queue(Target::Window(2)));
        assert!(!a.queue(Target::Window(3)));
        assert!(!a.queue(Target::Window(4)));
        assert_eq!(a.take_due(), Some(Target::Window(4)));
        assert_eq!(a.take_due(), None);
    }

    #[test]
    fn navigating_away_and_back_within_the_debounce_sends_nothing() {
        let mut a = AutoShow::default();
        a.enable(Some(Target::Window(1)));
        a.queue(Target::Window(2));
        a.queue(Target::Window(1));
        assert_eq!(a.take_due(), None);
    }

    #[test]
    fn disabling_drops_a_pending_target() {
        let mut a = AutoShow::default();
        a.enable(Some(Target::Window(1)));
        a.queue(Target::Window(2));
        a.disable();
        assert!(!a.is_on());
        assert_eq!(a.take_due(), None);
    }

    #[test]
    fn re_enabling_seeds_from_the_new_focus() {
        let mut a = AutoShow::default();
        a.enable(Some(Target::Window(1)));
        a.queue(Target::Window(2));
        assert_eq!(a.take_due(), Some(Target::Window(2)));
        a.disable();
        a.enable(Some(Target::Window(9)));
        assert!(!a.queue(Target::Window(9)));
        assert!(a.queue(Target::Window(2)));
        assert_eq!(a.take_due(), Some(Target::Window(2)));
    }

    #[test]
    fn a_focus_from_elsewhere_is_recorded_so_navigating_back_still_sends() {
        let mut a = AutoShow::default();
        a.enable(Some(Target::Window(1)));
        // Something else (pressing Enter) focused window 5.
        a.record_sent(Target::Window(5));
        assert!(a.queue(Target::Window(1)));
        assert_eq!(a.take_due(), Some(Target::Window(1)));
    }

    #[test]
    fn a_recorded_focus_cancels_a_pending_send_of_the_same_target() {
        let mut a = AutoShow::default();
        a.enable(Some(Target::Window(1)));
        a.queue(Target::Window(2));
        a.record_sent(Target::Window(2));
        assert_eq!(a.take_due(), None);
    }

    #[test]
    fn nothing_is_recorded_while_the_mode_is_off() {
        let mut a = AutoShow::default();
        a.record_sent(Target::Window(5));
        a.enable(Some(Target::Window(1)));
        assert!(a.queue(Target::Window(5)));
        assert_eq!(a.take_due(), Some(Target::Window(5)));
    }

    #[test]
    fn a_timer_can_be_armed_again_after_one_fires() {
        let mut a = AutoShow::default();
        a.enable(None);
        assert!(a.queue(Target::Window(2)));
        a.take_due();
        assert!(a.queue(Target::Window(3)));
        assert_eq!(a.take_due(), Some(Target::Window(3)));
    }
}
