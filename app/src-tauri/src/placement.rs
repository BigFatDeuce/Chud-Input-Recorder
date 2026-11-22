//! Keeps the main window where the user can grab it.
//!
//! The window has no system frame: its title bar is drawn by the interface.
//! If the top of the window leaves the screen, nothing is left to drag it
//! back with. That happened with the default size (1480 × 920) on screens
//! whose work area is shorter, e.g. 2560 × 1440 at 175 % (775 px usable):
//! the window was centred with its top above the screen.
//!
//! So the window is sized to the work area at start, and whenever a resize,
//! a DPI change or a move leaves the title bar out of reach, it is brought
//! back onto the nearest screen.

use std::sync::Mutex;

use tauri::{LogicalSize, Monitor, PhysicalPosition, PhysicalSize, Runtime, WebviewWindow};

/// Height of the interface's title bar (`--titlebar-h`), logical pixels.
const TITLEBAR: f64 = 40.0;
/// Width of title bar that must stay on screen. The window buttons take
/// about 140 px on the right, so this always leaves some draggable bar.
const GRAB: f64 = 240.0;
/// Gap left around the window when it is sized at start.
const MARGIN: f64 = 24.0;
/// Size the window asks for at start (tauri.conf.json), logical pixels.
const PREFERRED: (f64, f64) = (1480.0, 920.0);
/// Minimum size (tauri.conf.json), lowered on screens that are smaller.
const MIN: (f64, f64) = (800.0, 500.0);

/// A rectangle in physical pixels of the virtual desktop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    /// Width and height of the intersection (0 when they do not touch).
    fn overlap(&self, o: &Rect) -> (i32, i32) {
        let w = self.right().min(o.right()) - self.x.max(o.x);
        let h = self.bottom().min(o.bottom()) - self.y.max(o.y);
        (w.max(0), h.max(0))
    }
    fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }
}

/// How much of the title bar has to be visible.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Need {
    /// All of its height: after a resize, a DPI change, at start.
    Whole,
    /// A quarter of it. Used while the window moves, so that a drag that
    /// briefly pushes the bar against the top edge is not fought.
    Part,
}

/// Is enough of the title bar of `win` (client area) on one of `areas`?
pub fn reachable(win: Rect, areas: &[Rect], bar: i32, grab: i32, need: Need) -> bool {
    let strip = Rect { h: bar.min(win.h), ..win };
    let tall = match need {
        Need::Whole => strip.h,
        Need::Part => (strip.h / 4).max(1),
    };
    areas.iter().any(|a| {
        let (w, h) = strip.overlap(a);
        w >= grab.min(strip.w) && h >= tall
    })
}

/// The work area the window belongs to: the one under the middle of its
/// title bar, else the one it overlaps most, else the closest one.
pub fn home(win: Rect, areas: &[Rect], bar: i32) -> Option<usize> {
    let (cx, cy) = (win.x + win.w / 2, win.y + bar.min(win.h) / 2);
    if let Some(i) = areas.iter().position(|a| a.contains(cx, cy)) {
        return Some(i);
    }
    let best = (0..areas.len())
        .map(|i| {
            let (w, h) = win.overlap(&areas[i]);
            (i, w as i64 * h as i64)
        })
        .max_by_key(|&(_, s)| s)?;
    if best.1 > 0 {
        return Some(best.0);
    }
    (0..areas.len()).min_by_key(|&i| {
        let a = &areas[i];
        let dx = (a.x + a.w / 2 - cx) as i64;
        let dy = (a.y + a.h / 2 - cy) as i64;
        dx * dx + dy * dy
    })
}

/// `win` moved (and shrunk if needed) so that it lies inside `area`.
pub fn bring_back(win: Rect, area: Rect) -> Rect {
    let w = win.w.min(area.w);
    let h = win.h.min(area.h);
    Rect { x: win.x.clamp(area.x, area.right() - w), y: win.y.clamp(area.y, area.bottom() - h), w, h }
}

/// Start position: the preferred size, capped by the area minus a margin,
/// centred in it.
pub fn centred(preferred: (i32, i32), area: Rect, margin: i32) -> Rect {
    let w = preferred.0.min(area.w - 2 * margin).max(1);
    let h = preferred.1.min(area.h - 2 * margin).max(1);
    Rect { x: area.x + (area.w - w) / 2, y: area.y + (area.h - h) / 2, w, h }
}

fn area_of(m: &Monitor) -> Rect {
    let a = m.work_area();
    Rect { x: a.position.x, y: a.position.y, w: a.size.width as i32, h: a.size.height as i32 }
}

/// Client-area rectangle of the window, and the offset from its outer
/// position (Windows keeps invisible resize borders around frameless
/// windows: `set_position` places the outer corner).
fn client<R: Runtime>(w: &WebviewWindow<R>) -> Option<(Rect, (i32, i32))> {
    let inner = w.inner_position().ok()?;
    let outer = w.outer_position().ok()?;
    let size = w.inner_size().ok()?;
    let r = Rect { x: inner.x, y: inner.y, w: size.width as i32, h: size.height as i32 };
    Some((r, (inner.x - outer.x, inner.y - outer.y)))
}

/// Last minimum size applied, to avoid re-applying it on every event.
static MIN_APPLIED: Mutex<Option<(u32, u32)>> = Mutex::new(None);

/// Lowers the minimum size on screens smaller than it (and restores it on
/// larger ones), otherwise the window could not be made to fit.
fn adapt_min_size<R: Runtime>(w: &WebviewWindow<R>, area: Rect, scale: f64) {
    let min_w = MIN.0.min(area.w as f64 / scale).floor();
    let min_h = MIN.1.min(area.h as f64 / scale).floor();
    let key = (min_w as u32, min_h as u32);
    let mut last = MIN_APPLIED.lock().unwrap_or_else(|e| e.into_inner());
    if *last != Some(key) {
        *last = Some(key);
        let _ = w.set_min_size(Some(LogicalSize::new(min_w, min_h)));
    }
}

fn apply<R: Runtime>(w: &WebviewWindow<R>, now: Rect, to: Rect, offset: (i32, i32)) {
    if (to.w, to.h) != (now.w, now.h) {
        let _ = w.set_size(PhysicalSize::new(to.w as u32, to.h as u32));
    }
    if (to.x, to.y) != (now.x, now.y) {
        let _ = w.set_position(PhysicalPosition::new(to.x - offset.0, to.y - offset.1));
    }
}

/// Sizes and centres the (still hidden) main window on its screen.
pub fn place_initial<R: Runtime>(w: &WebviewWindow<R>) {
    let monitor = w.current_monitor().ok().flatten().or_else(|| w.primary_monitor().ok().flatten());
    let (Some(m), Some((now, offset))) = (monitor, client(w)) else { return };
    let (area, scale) = (area_of(&m), m.scale_factor());
    adapt_min_size(w, area, scale);
    let preferred = ((PREFERRED.0 * scale) as i32, (PREFERRED.1 * scale) as i32);
    let to = centred(preferred, area, (MARGIN * scale) as i32);
    apply(w, now, to, offset);
    log::info!("main window placed at {to:?} in work area {area:?} (scale {scale})");
}

/// Brings the window back if its title bar is out of reach.
pub fn keep_reachable<R: Runtime>(w: &WebviewWindow<R>, need: Need) {
    if w.is_minimized().unwrap_or(true) || w.is_maximized().unwrap_or(true) || w.is_fullscreen().unwrap_or(true) {
        return;
    }
    if !w.is_visible().unwrap_or(false) {
        return;
    }
    let Ok(monitors) = w.available_monitors() else { return };
    let areas: Vec<Rect> = monitors.iter().map(area_of).collect();
    let Some((now, offset)) = client(w) else { return };
    let Some(i) = home(now, &areas, 1) else { return };
    let scale = monitors[i].scale_factor();
    adapt_min_size(w, areas[i], scale);
    let bar = (TITLEBAR * scale) as i32;
    if reachable(now, &areas, bar, (GRAB * scale) as i32, need) {
        return;
    }
    let to = bring_back(now, areas[i]);
    log::info!("title bar out of reach at {now:?}: window brought back to {to:?}");
    apply(w, now, to, offset);
}

#[cfg(test)]
mod tests {
    use super::*;

    const BAR: i32 = 70; // 40 px at 175 %
    const GRAB: i32 = 420;
    // 2560 × 1440 at 175 %, taskbar at the bottom.
    const SCREEN: Rect = Rect { x: 0, y: 0, w: 2560, h: 1356 };

    #[test]
    fn the_reported_case_is_detected_and_fixed() {
        // Measured on the user's machine: client area at y = -89.
        let win = Rect { x: 1, y: -89, w: 1872, h: 1146 };
        assert!(!reachable(win, &[SCREEN], BAR, GRAB, Need::Whole));
        assert!(!reachable(win, &[SCREEN], BAR, GRAB, Need::Part));
        let fixed = bring_back(win, SCREEN);
        assert_eq!(fixed, Rect { x: 1, y: 0, w: 1872, h: 1146 });
        assert!(reachable(fixed, &[SCREEN], BAR, GRAB, Need::Whole));
    }

    #[test]
    fn a_window_taller_than_the_screen_is_shrunk() {
        let win = Rect { x: 100, y: -127, w: 2590, h: 1610 };
        let fixed = bring_back(win, SCREEN);
        assert_eq!(fixed, SCREEN);
    }

    #[test]
    fn start_size_fits_the_work_area() {
        // 1480 × 920 logical at 175 % does not fit in 1356 px.
        let r = centred((2590, 1610), SCREEN, 42);
        assert_eq!((r.w, r.h), (2476, 1272));
        assert_eq!((r.x, r.y), (42, 42));
        assert!(reachable(r, &[SCREEN], BAR, GRAB, Need::Whole));
        // On a large screen the preferred size is kept and centred.
        let big = Rect { x: 0, y: 0, w: 3840, h: 2100 };
        let r = centred((1480, 920), big, 24);
        assert_eq!(r, Rect { x: 1180, y: 590, w: 1480, h: 920 });
    }

    #[test]
    fn a_bar_pushed_slightly_against_the_top_is_tolerated_while_moving() {
        let win = Rect { x: 200, y: -30, w: 1800, h: 1100 };
        assert!(reachable(win, &[SCREEN], BAR, GRAB, Need::Part));
        assert!(!reachable(win, &[SCREEN], BAR, GRAB, Need::Whole));
    }

    #[test]
    fn a_bar_hidden_on_the_side_is_brought_back() {
        // Only the window buttons would remain on screen.
        let win = Rect { x: -1700, y: 100, w: 1800, h: 1100 };
        assert!(!reachable(win, &[SCREEN], BAR, GRAB, Need::Part));
        assert_eq!(bring_back(win, SCREEN).x, 0);
        // A window hanging off the bottom keeps its place: the bar is fine.
        let low = Rect { x: 200, y: 1000, w: 1800, h: 1100 };
        assert!(reachable(low, &[SCREEN], BAR, GRAB, Need::Whole));
    }

    #[test]
    fn a_screen_above_counts_as_on_screen() {
        let above = Rect { x: 0, y: -1080, w: 1920, h: 1040 };
        let win = Rect { x: 100, y: -500, w: 1400, h: 900 };
        assert!(reachable(win, &[SCREEN, above], BAR, GRAB, Need::Whole));
        // Across the two screens, the bar is on the upper one.
        assert_eq!(home(win, &[SCREEN, above], 1), Some(1));
    }

    #[test]
    fn a_window_on_no_screen_goes_to_the_closest_one() {
        let right = Rect { x: 2560, y: 0, w: 1920, h: 1040 };
        let win = Rect { x: 6000, y: 200, w: 1400, h: 900 };
        let i = home(win, &[SCREEN, right], 1).unwrap();
        assert_eq!(i, 1);
        let fixed = bring_back(win, right);
        // Kept whole inside that screen: 900 px tall in 1040 px of work area.
        assert_eq!((fixed.x, fixed.y), (right.right() - 1400, 140));
    }
}
