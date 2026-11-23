//! Pen input on Android (LightCraft for Android patch A1, see `PATCHES.md`).
//!
//! Upstream winit 0.30 turns every Android `MotionEvent` pointer into a `WindowEvent::Touch` with
//! the raw pressure as its force, drops hover events, the stylus buttons, the tool type and the
//! batched history samples. This module is the state machine that fixes that for active pens
//! (Samsung S Pen, USI pens, Wacom EMR):
//!
//! - **Pressure** is reported only for a pen or an eraser: a finger's "pressure" on Android is a
//!   contact-area estimate, so fingers report no force and paint as a mouse does.
//! - **Hover**: a pen (or mouse) hovering over the screen moves the cursor (`CursorEntered`,
//!   `CursorMoved`, `CursorLeft`), so apps can draw a brush outline before the pen touches.
//! - **Side button and eraser**: holding the pen's side button, or touching with an eraser end,
//!   reports the Alt modifier ("the other mode": erase while painting, paint while erasing).
//! - **Palm rejection**: while a pen hovers or touches, new finger touches are ignored; a touch
//!   Android flags as a palm (`TOOL_TYPE_PALM` or `FLAG_CANCELED`) is ended; fingers already down
//!   when the pen lands are ended.
//! - **History**: the samples Android batches into one move event (an S Pen reports at up to
//!   240 Hz, the display refreshes slower) are each reported, so strokes keep their shape.
//!
//! The logic here works on plain data ([`Sample`], [`Out`]) so it can be tested without a device.

use std::collections::HashSet;

/// The kind of tool a pointer is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tool {
    Finger,
    Stylus,
    Eraser,
    Mouse,
    Palm,
    Unknown,
}

impl Tool {
    /// An active pen: its pressure is real and its presence means the hand on the screen is a palm.
    pub(crate) fn is_pen(self) -> bool {
        matches!(self, Tool::Stylus | Tool::Eraser)
    }

    /// A tool that can hover (and so moves the cursor without touching).
    fn hovers(self) -> bool {
        matches!(self, Tool::Stylus | Tool::Eraser | Tool::Mouse)
    }
}

/// The action of a motion event, reduced to what matters here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    /// The first pointer, or another one (`ACTION_DOWN`, `ACTION_POINTER_DOWN`).
    Down,
    /// The last pointer, or another one (`ACTION_UP`, `ACTION_POINTER_UP`).
    Up,
    Move,
    Cancel,
    HoverEnter,
    HoverMove,
    HoverExit,
    Other,
}

/// One pointer sample, in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Sample {
    pub id: i32,
    pub x: f32,
    pub y: f32,
    pub pressure: f32,
    pub tool: Tool,
}

/// The phase of a touch, as winit reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    Started,
    Moved,
    Ended,
}

/// What the event loop sends on, in order.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Out {
    Touch { id: i32, phase: Phase, x: f32, y: f32, force: Option<f64> },
    CursorEntered,
    CursorMoved { x: f32, y: f32 },
    CursorLeft,
    /// The pen's side button or eraser, reported as the Alt modifier.
    Alt(bool),
}

/// One motion event, reduced.
#[derive(Debug, Clone)]
pub(crate) struct Motion<'a> {
    pub action: Action,
    /// The pointer the action is about (for `Down` and `Up`).
    pub acting: Sample,
    /// All the pointers of the event, at its latest time.
    pub pointers: &'a [Sample],
    /// Earlier samples batched into a `Move`, oldest first.
    pub history: &'a [Sample],
    /// A stylus button is held (`BUTTON_STYLUS_PRIMARY`, or the older `BUTTON_SECONDARY` some pens report).
    pub pen_button: bool,
    /// Android cancelled the gesture of the acting pointer (`FLAG_CANCELED`: it was a palm).
    pub canceled: bool,
}

/// Which pointers are down, which are ignored, and the pen's state.
#[derive(Debug, Default)]
pub(crate) struct PenState {
    /// Pointers reported as touches (started, not yet ended).
    down: HashSet<i32>,
    /// Pens among them.
    pens_down: HashSet<i32>,
    /// Pointers ignored until they lift (palms, fingers while a pen is near).
    rejected: HashSet<i32>,
    /// A pen hovers.
    hovering: bool,
    /// The Alt modifier as last reported.
    alt: bool,
    /// The last position of each pointer that is down, to end it where it was.
    last: Vec<(i32, f32, f32)>,
}

impl PenState {
    /// A pen hovers over, or touches, the screen: fingers are then palms.
    pub(crate) fn pen_near(&self) -> bool {
        self.hovering || !self.pens_down.is_empty()
    }

    fn set_alt(&mut self, alt: bool, out: &mut Vec<Out>) {
        if self.alt != alt {
            self.alt = alt;
            out.push(Out::Alt(alt));
        }
    }

    fn force(s: &Sample) -> Option<f64> {
        if s.tool.is_pen() && s.pressure.is_finite() {
            Some(f64::from(s.pressure.clamp(0.0, 1.0)))
        } else {
            None
        }
    }

    fn remember(&mut self, s: &Sample) {
        match self.last.iter_mut().find(|l| l.0 == s.id) {
            Some(l) => *l = (s.id, s.x, s.y),
            None => self.last.push((s.id, s.x, s.y)),
        }
    }

    /// End a pointer that is down. egui needs an end (not a cancel) to release its emulated
    /// mouse button; a cancel would leave the button held and join the next touch to this one.
    fn end(&mut self, id: i32, at: Option<(f32, f32)>, out: &mut Vec<Out>) {
        if !self.down.remove(&id) {
            return;
        }
        let was_pen = self.pens_down.remove(&id);
        let (x, y) = at.or_else(|| self.last.iter().find(|l| l.0 == id).map(|l| (l.1, l.2))).unwrap_or((0.0, 0.0));
        self.last.retain(|l| l.0 != id);
        out.push(Out::Touch { id, phase: Phase::Ended, x, y, force: if was_pen { Some(0.0) } else { None } });
    }

    fn start(&mut self, s: &Sample, out: &mut Vec<Out>) {
        self.down.insert(s.id);
        if s.tool.is_pen() {
            self.pens_down.insert(s.id);
        }
        self.remember(s);
        out.push(Out::Touch { id: s.id, phase: Phase::Started, x: s.x, y: s.y, force: Self::force(s) });
    }

    /// The events one motion event becomes.
    pub(crate) fn handle(&mut self, m: &Motion<'_>) -> Vec<Out> {
        let mut out = Vec::new();
        let a = m.acting;
        match m.action {
            Action::HoverEnter | Action::HoverMove if a.tool.hovers() => {
                if a.tool.is_pen() {
                    self.set_alt(m.pen_button || a.tool == Tool::Eraser, &mut out);
                    self.hovering = true;
                }
                if m.action == Action::HoverEnter {
                    out.push(Out::CursorEntered);
                }
                for h in m.history.iter().filter(|h| h.id == a.id) {
                    out.push(Out::CursorMoved { x: h.x, y: h.y });
                }
                out.push(Out::CursorMoved { x: a.x, y: a.y });
            }
            Action::HoverExit if a.tool.hovers() => {
                self.hovering = false;
                // (a pen about to touch sends this first: its button comes back with the touch)
                if self.pens_down.is_empty() {
                    self.set_alt(false, &mut out);
                }
                out.push(Out::CursorLeft);
            }
            Action::Down => {
                if self.down.contains(&a.id) || self.rejected.contains(&a.id) {
                    return out;
                }
                if a.tool == Tool::Palm || m.canceled || (!a.tool.is_pen() && self.pen_near()) {
                    self.rejected.insert(a.id);
                    return out;
                }
                if a.tool.is_pen() {
                    // fingers already down when the pen lands were the hand holding it
                    let fingers: Vec<i32> = self.down.iter().copied().filter(|id| !self.pens_down.contains(id)).collect();
                    for id in fingers {
                        self.end(id, None, &mut out);
                        self.rejected.insert(id);
                    }
                    self.set_alt(m.pen_button || a.tool == Tool::Eraser, &mut out);
                } else if self.alt {
                    // a finger never touches with the pen's button
                    self.set_alt(false, &mut out);
                }
                self.start(&a, &mut out);
            }
            Action::Move => {
                if m.pointers.iter().any(|p| p.tool.is_pen() && self.pens_down.contains(&p.id)) {
                    self.set_alt(m.pen_button || m.pointers.iter().any(|p| p.tool == Tool::Eraser), &mut out);
                }
                for p in m.pointers {
                    if self.rejected.contains(&p.id) || !self.down.contains(&p.id) {
                        continue;
                    }
                    if p.tool == Tool::Palm {
                        // Android decided mid-touch that this is a palm
                        self.end(p.id, None, &mut out);
                        self.rejected.insert(p.id);
                        continue;
                    }
                    for h in m.history.iter().filter(|h| h.id == p.id) {
                        out.push(Out::Touch { id: p.id, phase: Phase::Moved, x: h.x, y: h.y, force: Self::force(h) });
                    }
                    self.remember(p);
                    out.push(Out::Touch { id: p.id, phase: Phase::Moved, x: p.x, y: p.y, force: Self::force(p) });
                }
            }
            Action::Up => {
                if self.rejected.remove(&a.id) {
                    return out;
                }
                // a cancelled palm ends where it last was, not where Android moved it on lifting
                let at = if m.canceled || a.tool == Tool::Palm { None } else { Some((a.x, a.y)) };
                self.end(a.id, at, &mut out);
                // (Alt stays as it was: the app reads it when the stroke ends, in this frame)
            }
            Action::Cancel => {
                let ids: Vec<i32> = self.down.iter().copied().collect();
                for id in ids {
                    self.end(id, None, &mut out);
                }
                self.rejected.clear();
                self.set_alt(false, &mut out);
            }
            Action::HoverEnter | Action::HoverMove | Action::HoverExit | Action::Other => {}
        }
        if self.down.is_empty() && !self.hovering {
            // every pointer has lifted: nothing is left to ignore
            self.rejected.clear();
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(id: i32, tool: Tool, x: f32, pressure: f32) -> Sample {
        Sample { id, x, y: 0.0, pressure, tool }
    }

    fn ev<'a>(action: Action, acting: Sample, pointers: &'a [Sample]) -> Motion<'a> {
        Motion { action, acting, pointers, history: &[], pen_button: false, canceled: false }
    }

    fn touches(out: &[Out]) -> Vec<(i32, Phase)> {
        out.iter()
            .filter_map(|o| match o {
                Out::Touch { id, phase, .. } => Some((*id, *phase)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_pen_reports_pressure_and_a_finger_does_not() {
        let mut st = PenState::default();
        let pen = s(0, Tool::Stylus, 1.0, 0.4);
        let out = st.handle(&ev(Action::Down, pen, &[pen]));
        assert!(matches!(out.last(), Some(Out::Touch { force: Some(f), .. }) if (*f - 0.4).abs() < 1e-6));
        st.handle(&ev(Action::Up, pen, &[pen]));
        let finger = s(1, Tool::Finger, 1.0, 0.9);
        let out = st.handle(&ev(Action::Down, finger, &[finger]));
        assert!(matches!(out.last(), Some(Out::Touch { force: None, .. })));
    }

    #[test]
    fn fingers_are_ignored_while_the_pen_hovers_and_until_they_lift() {
        let mut st = PenState::default();
        let pen = s(0, Tool::Stylus, 1.0, 0.0);
        st.handle(&ev(Action::HoverEnter, pen, &[pen]));
        let palm = s(1, Tool::Finger, 5.0, 1.0);
        assert!(touches(&st.handle(&ev(Action::Down, palm, &[pen, palm]))).is_empty());
        // the pen leaves; the palm is still ignored until it lifts
        st.handle(&ev(Action::HoverExit, pen, &[pen]));
        assert!(touches(&st.handle(&ev(Action::Move, palm, &[palm]))).is_empty());
        assert!(touches(&st.handle(&ev(Action::Up, palm, &[palm]))).is_empty());
        // a new finger touch afterwards works again
        let tap = s(2, Tool::Finger, 3.0, 1.0);
        assert_eq!(touches(&st.handle(&ev(Action::Down, tap, &[tap]))), vec![(2, Phase::Started)]);
    }

    #[test]
    fn a_finger_down_before_the_pen_is_ended_when_the_pen_lands() {
        let mut st = PenState::default();
        let palm = s(1, Tool::Finger, 5.0, 1.0);
        st.handle(&ev(Action::Down, palm, &[palm]));
        let pen = s(0, Tool::Stylus, 1.0, 0.5);
        let out = st.handle(&ev(Action::Down, pen, &[palm, pen]));
        assert_eq!(touches(&out), vec![(1, Phase::Ended), (0, Phase::Started)]);
        // its later moves are dropped
        assert_eq!(touches(&st.handle(&ev(Action::Move, pen, &[palm, pen]))), vec![(0, Phase::Moved)]);
    }

    #[test]
    fn the_side_button_holds_alt_through_the_stroke_end() {
        let mut st = PenState::default();
        let pen = s(0, Tool::Stylus, 1.0, 0.5);
        let pens = [pen];
        let mut down = ev(Action::Down, pen, &pens);
        down.pen_button = true;
        assert_eq!(st.handle(&down).first(), Some(&Out::Alt(true)));
        let up = st.handle(&ev(Action::Up, pen, &[pen]));
        assert!(!up.contains(&Out::Alt(false)), "Alt is still held when the stroke ends");
        // hovering away with the button released lets go of it
        assert!(st.handle(&ev(Action::HoverExit, pen, &[pen])).contains(&Out::Alt(false)));
    }

    #[test]
    fn the_eraser_end_reports_alt() {
        let mut st = PenState::default();
        let eraser = s(0, Tool::Eraser, 1.0, 0.5);
        assert!(st.handle(&ev(Action::Down, eraser, &[eraser])).contains(&Out::Alt(true)));
    }

    #[test]
    fn a_finger_tap_after_a_button_stroke_is_not_an_alt_tap() {
        let mut st = PenState::default();
        let pen = s(0, Tool::Stylus, 1.0, 0.5);
        let pens = [pen];
        let mut down = ev(Action::Down, pen, &pens);
        down.pen_button = true;
        st.handle(&down);
        st.handle(&ev(Action::Up, pen, &[pen]));
        let tap = s(1, Tool::Finger, 3.0, 1.0);
        let out = st.handle(&ev(Action::Down, tap, &[tap]));
        assert_eq!(out.first(), Some(&Out::Alt(false)));
    }

    #[test]
    fn batched_samples_are_each_reported_in_order() {
        let mut st = PenState::default();
        let pen = s(0, Tool::Stylus, 1.0, 0.5);
        st.handle(&ev(Action::Down, pen, &[pen]));
        let now = s(0, Tool::Stylus, 4.0, 0.8);
        let history = [s(0, Tool::Stylus, 2.0, 0.6), s(0, Tool::Stylus, 3.0, 0.7)];
        let out = st.handle(&Motion { history: &history, ..ev(Action::Move, now, &[now]) });
        let xs: Vec<f32> = out
            .iter()
            .filter_map(|o| match o {
                Out::Touch { x, .. } => Some(*x),
                _ => None,
            })
            .collect();
        assert_eq!(xs, vec![2.0, 3.0, 4.0]);
    }

    #[test]
    fn a_touch_android_cancels_as_a_palm_ends_where_it_was() {
        let mut st = PenState::default();
        let palm = s(1, Tool::Finger, 5.0, 1.0);
        st.handle(&ev(Action::Down, palm, &[palm]));
        let lifted = s(1, Tool::Finger, 50.0, 1.0);
        let out = st.handle(&Motion { canceled: true, ..ev(Action::Up, lifted, &[lifted]) });
        assert!(matches!(out.as_slice(), [Out::Touch { phase: Phase::Ended, x, .. }] if *x == 5.0));
    }

    #[test]
    fn hover_moves_the_cursor_and_exit_removes_it() {
        let mut st = PenState::default();
        let pen = s(0, Tool::Stylus, 7.0, 0.0);
        let out = st.handle(&ev(Action::HoverEnter, pen, &[pen]));
        assert_eq!(out, vec![Out::CursorEntered, Out::CursorMoved { x: 7.0, y: 0.0 }]);
        assert!(st.pen_near());
        assert_eq!(st.handle(&ev(Action::HoverExit, pen, &[pen])), vec![Out::CursorLeft]);
        assert!(!st.pen_near());
    }
}
