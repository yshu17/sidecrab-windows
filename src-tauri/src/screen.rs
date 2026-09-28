//! Pure geometry: is a window rectangle visible on any currently connected
//! monitor? Recovers a pet stranded off-screen — the usual trigger is the
//! system sleeping/waking with a different monitor layout (an external
//! display or projector that was connected when the position was saved is
//! gone on wake), which leaves the window at physical coordinates that no
//! longer belong to any screen. It is still running — click-through and
//! topmost pollers keep going — it is just nowhere the user can see it.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    fn right(&self) -> i32 {
        self.x + self.w
    }
    fn bottom(&self) -> i32 {
        self.y + self.h
    }
    /// Pixels of overlap with `other` along each axis (0 when they don't meet).
    fn overlap(&self, other: &Rect) -> (i32, i32) {
        let ox = (self.right().min(other.right()) - self.x.max(other.x)).max(0);
        let oy = (self.bottom().min(other.bottom()) - self.y.max(other.y)).max(0);
        (ox, oy)
    }
}

/// True once at least `min_px` of `win` sits on some monitor, on both axes.
/// A sliver caught on a screen edge still counts — enough to see and grab it
/// is enough to call it found. `min_px` is capped to the window's own size so
/// a pet smaller than the threshold isn't impossible to satisfy.
pub fn is_onscreen(win: Rect, monitors: &[Rect], min_px: i32) -> bool {
    monitors.iter().any(|m| {
        let (ox, oy) = win.overlap(m);
        ox >= min_px.min(win.w) && oy >= min_px.min(win.h)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MON_1080P: Rect = Rect { x: 0, y: 0, w: 1920, h: 1080 };

    #[test]
    fn window_inside_its_monitor_is_onscreen() {
        let win = Rect { x: 1700, y: 900, w: 150, h: 144 };
        assert!(is_onscreen(win, &[MON_1080P], 40));
    }

    #[test]
    fn window_on_second_monitor_is_onscreen() {
        let second = Rect { x: 1920, y: 0, w: 1920, h: 1080 };
        let win = Rect { x: 2000, y: 100, w: 150, h: 144 };
        assert!(is_onscreen(win, &[MON_1080P, second], 40));
    }

    /// The scenario this module exists for: an external monitor the pet's
    /// saved position was on has been unplugged (typically discovered on
    /// wake from sleep), leaving only the laptop's own screen.
    #[test]
    fn window_on_a_now_disconnected_monitor_is_offscreen() {
        let win = Rect { x: 2400, y: 300, w: 150, h: 144 }; // was on the 2nd monitor
        assert!(!is_onscreen(win, &[MON_1080P], 40)); // only the 1st remains
    }

    #[test]
    fn window_fully_above_the_top_edge_is_offscreen() {
        let win = Rect { x: 100, y: -300, w: 150, h: 144 };
        assert!(!is_onscreen(win, &[MON_1080P], 40));
    }

    #[test]
    fn window_far_left_of_every_monitor_is_offscreen() {
        let win = Rect { x: -5000, y: 100, w: 150, h: 144 };
        assert!(!is_onscreen(win, &[MON_1080P], 40));
    }

    #[test]
    fn tiny_sliver_below_threshold_does_not_count() {
        // Only 10px of the window pokes onto the monitor at the left edge.
        let win = Rect { x: -140, y: 100, w: 150, h: 144 };
        assert!(!is_onscreen(win, &[MON_1080P], 40));
    }

    #[test]
    fn sliver_at_or_above_threshold_counts() {
        let win = Rect { x: -110, y: 100, w: 150, h: 144 }; // 40px visible
        assert!(is_onscreen(win, &[MON_1080P], 40));
    }

    #[test]
    fn no_monitors_at_all_is_offscreen() {
        let win = Rect { x: 100, y: 100, w: 150, h: 144 };
        assert!(!is_onscreen(win, &[], 40));
    }

    #[test]
    fn threshold_larger_than_window_falls_back_to_full_window() {
        // min_px (999) exceeds the window's own size — require full overlap,
        // not an impossible amount, so a small crab isn't judged unrecoverable.
        let win = Rect { x: 0, y: 0, w: 50, h: 50 };
        assert!(is_onscreen(win, &[MON_1080P], 999));
    }

    #[test]
    fn window_straddling_a_monitor_edge_is_onscreen() {
        let win = Rect { x: 1900, y: 900, w: 150, h: 144 }; // 20px hangs off the right
        assert!(is_onscreen(win, &[MON_1080P], 15));
    }
}
