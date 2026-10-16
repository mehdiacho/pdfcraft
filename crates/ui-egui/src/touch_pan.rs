//! Touch panning on the page view: a finger dragged over the pages with the Select or Hand tool
//! scrolls them, and a flick glides on after the finger lifts ([`crate::fling`]).
//!
//! egui's own drag-to-scroll can't do this here. The page content takes every drag (for text
//! selection and the Hand tool), and on a slow phone or emulator a flick's press, moves and lift
//! can all arrive in one or two passes, which egui's pass-by-pass drag detection never sees as a
//! drag. The touch events themselves carry all of it, so the pan reads them.
//!
//! With the Select tool, a finger held still for egui's longest click before it moves selects
//! text instead, as a long press does on a phone.

use egui::{Event, Pos2, Rect, TouchDeviceId, TouchId, TouchPhase, Vec2};

/// How far a finger travels before its touch is a pan, not a tap: egui's longest click.
const SLOP: f32 = 6.0;
/// Seconds of motion the lift speed is measured over.
const WINDOW: f64 = 0.1;
/// A lift this long after the last motion, or one and a half passes on a slow device (up to
/// [`MAX_STILL`]), is a finger that stopped first: no glide. A finger resting sends nothing, so
/// its lift can be the first pass in a while.
const STILL: f64 = 0.05;
const MAX_STILL: f64 = 0.2;

#[derive(Clone, Copy, PartialEq, Debug)]
enum State {
    /// Down, and not yet moved past [`SLOP`].
    Undecided,
    Panning,
    /// A long press that went on to select text: the drag is the content's.
    Selecting,
    /// A second finger came down (a pinch): this touch no longer pans.
    Ignored,
}

struct Touch {
    id: (TouchDeviceId, TouchId),
    start: Pos2,
    at: f64,
    last: Pos2,
    state: State,
    /// When each pass's motion arrived, and how far it went.
    samples: Vec<(f64, Vec2)>,
}

#[derive(Default)]
pub(crate) struct TouchPan {
    touch: Option<Touch>,
    /// When the pass before this one ran: a slow device's passes are far apart.
    last_pass: f64,
}

/// What a pass's touch events did.
#[derive(Default, Debug, PartialEq)]
pub(crate) struct Pass {
    /// How far the finger moved the pages this pass (positive moves the content down).
    pub delta: Vec2,
    /// The finger's speed as it lifted this pass, in points per second, when it should glide.
    pub lift: Option<Vec2>,
}

impl TouchPan {
    /// Whether a finger is down and the drag is the pan's (panning, or not yet decided): the page
    /// content must neither select text nor pan with it.
    pub(crate) fn owns_drag(&self) -> bool {
        self.touch.as_ref().is_some_and(|t| matches!(t.state, State::Undecided | State::Panning))
    }

    /// Reads one pass's events at time `now`. A touch pans only when it comes down inside `area`
    /// while `enabled`; `long_press` is how long a finger is held still before it selects text
    /// instead, or `None` when it never does.
    pub(crate) fn read(&mut self, events: &[Event], now: f64, area: Rect, enabled: bool, long_press: Option<f64>) -> Pass {
        let mut pass = Pass::default();
        let dt = now - std::mem::replace(&mut self.last_pass, now);
        if !enabled {
            self.touch = None;
            return pass;
        }
        for event in events {
            let Event::Touch { device_id, id, phase, pos, .. } = event else { continue };
            let key = (*device_id, *id);
            if !(pos.x.is_finite() && pos.y.is_finite()) {
                continue;
            }
            match (phase, self.touch.as_mut()) {
                (TouchPhase::Start, Some(t)) if t.id != key => t.state = State::Ignored,
                (TouchPhase::Start, _) => {
                    self.touch = area.contains(*pos).then(|| Touch {
                        id: key,
                        start: *pos,
                        at: now,
                        last: *pos,
                        state: State::Undecided,
                        samples: Vec::new(),
                    });
                }
                (TouchPhase::Move, Some(t)) if t.id == key => {
                    if t.state == State::Undecided && pos.distance(t.start) > SLOP {
                        let held = long_press.is_some_and(|hold| now - t.at >= hold);
                        t.state = if held { State::Selecting } else { State::Panning };
                    }
                    if t.state == State::Panning {
                        let d = *pos - t.last;
                        pass.delta += d;
                        match t.samples.last_mut() {
                            // Several moves in one pass are one sample: they share its time.
                            Some((at, sum)) if *at == now => *sum += d,
                            _ => t.samples.push((now, d)),
                        }
                        t.samples.retain(|(at, _)| now - at <= WINDOW.max(3.0 * dt));
                    }
                    // Until it pans, `last` stays where the finger came down, so the pan
                    // includes the slop and the page stays under the finger.
                    if t.state != State::Undecided {
                        t.last = *pos;
                    }
                }
                (TouchPhase::End, Some(t)) if t.id == key => {
                    if t.state == State::Panning {
                        pass.lift = lift_velocity(&t.samples, now, dt);
                    }
                    self.touch = None;
                }
                (TouchPhase::Cancel, Some(t)) if t.id == key => self.touch = None,
                _ => {}
            }
        }
        // egui makes a press held this long a drag even before it moves: hand it to the content
        // in the same pass, so the selection starts where the finger came down.
        if let (Some(t), Some(hold)) = (self.touch.as_mut(), long_press)
            && t.state == State::Undecided
            && now - t.at >= hold
        {
            t.state = State::Selecting;
        }
        pass
    }
}

/// The finger's speed as it lifted at `now`, `dt` after the pass before, from the motion of the
/// passes before.
fn lift_velocity(samples: &[(f64, Vec2)], now: f64, dt: f64) -> Option<Vec2> {
    let dt = if dt.is_finite() { dt.clamp(1e-3, 1.0) } else { 1e-3 };
    let last = samples.last()?;
    if now - last.0 > STILL.max(1.5 * dt).min(MAX_STILL) {
        return None;
    }
    let recent: Vec<&(f64, Vec2)> = samples.iter().filter(|(at, _)| now - at <= WINDOW.max(3.0 * dt)).collect();
    let first = recent.first()?;
    let (moved, span) = if recent.len() < 2 {
        // All of it in one pass: it took that pass.
        (last.1, dt)
    } else {
        // Each sample is the motion since the pass before it, so the first one's motion happened
        // before the span starts.
        (recent.iter().skip(1).fold(Vec2::ZERO, |sum, (_, d)| sum + *d), last.0 - first.0)
    };
    let v = moved / span as f32;
    (v.x.is_finite() && v.y.is_finite()).then_some(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{pos2, vec2};

    const AREA: Rect = Rect { min: pos2(0.0, 0.0), max: pos2(400.0, 800.0) };

    fn touch(phase: TouchPhase, y: f32) -> Event {
        Event::Touch { device_id: TouchDeviceId(1), id: TouchId(0), phase, pos: pos2(200.0, y), force: None }
    }

    #[test]
    fn a_drag_pans_by_the_finger_and_a_flick_lifts_at_its_speed() {
        let mut pan = TouchPan::default();
        let dt = 1.0 / 60.0_f64;
        let mut d = Vec2::ZERO;
        pan.read(&[touch(TouchPhase::Start, 600.0)], 0.0, AREA, true, Some(0.8));
        assert!(pan.owns_drag());
        for i in 1..=10 {
            d += pan.read(&[touch(TouchPhase::Move, 600.0 - 20.0 * i as f32)], dt * i as f64, AREA, true, Some(0.8)).delta;
        }
        assert_eq!(d, vec2(0.0, -200.0), "the pages follow the finger");
        let lift = pan.read(&[touch(TouchPhase::End, 400.0)], dt * 10.0, AREA, true, Some(0.8)).lift;
        let v = lift.expect("a flick glides");
        assert!((v.y + 1200.0).abs() < 1.0, "20 points a pass at 60 passes a second: {v:?}");
        assert!(!pan.owns_drag());
    }

    #[test]
    fn a_flick_squeezed_into_two_passes_still_pans_and_glides() {
        let mut pan = TouchPan::default();
        let mut first: Vec<Event> = vec![touch(TouchPhase::Start, 700.0)];
        first.extend((1..=10).map(|i| touch(TouchPhase::Move, 700.0 - 20.0 * i as f32)));
        let a = pan.read(&first, 0.0, AREA, true, Some(0.8));
        let mut second: Vec<Event> = (11..=20).map(|i| touch(TouchPhase::Move, 700.0 - 20.0 * i as f32)).collect();
        second.push(touch(TouchPhase::End, 300.0));
        let b = pan.read(&second, 0.1, AREA, true, Some(0.8));
        assert_eq!(a.delta + b.delta, vec2(0.0, -400.0));
        assert!(b.lift.is_some_and(|v| v.y < -1000.0), "{b:?}");
    }

    #[test]
    fn a_tap_a_rest_a_long_press_or_a_pinch_does_not_pan() {
        let mut pan = TouchPan::default();
        pan.read(&[touch(TouchPhase::Start, 600.0)], 0.0, AREA, true, Some(0.8));
        let p = pan.read(&[touch(TouchPhase::Move, 597.0), touch(TouchPhase::End, 597.0)], 0.05, AREA, true, Some(0.8));
        assert_eq!(p, Pass::default(), "a tap that wobbled");

        let mut pan = TouchPan::default();
        pan.read(&[touch(TouchPhase::Start, 600.0)], 0.0, AREA, true, Some(0.8));
        pan.read(&[touch(TouchPhase::Move, 500.0)], 0.016, AREA, true, Some(0.8));
        let p = pan.read(&[touch(TouchPhase::End, 500.0)], 0.5, AREA, true, Some(0.8));
        assert_eq!(p.lift, None, "the finger rested before it lifted");

        let mut pan = TouchPan::default();
        pan.read(&[touch(TouchPhase::Start, 600.0)], 0.0, AREA, true, Some(0.8));
        pan.read(&[], 0.9, AREA, true, Some(0.8));
        assert!(!pan.owns_drag(), "held for a long press, the drag selects text");
        let p = pan.read(&[touch(TouchPhase::Move, 500.0)], 0.95, AREA, true, Some(0.8));
        assert_eq!(p.delta, Vec2::ZERO);

        let mut pan = TouchPan::default();
        pan.read(&[touch(TouchPhase::Start, 600.0)], 0.0, AREA, true, None);
        let p = pan.read(&[touch(TouchPhase::Move, 500.0)], 0.9, AREA, true, None);
        assert_eq!(p.delta, vec2(0.0, -100.0), "the Hand tool always pans");

        let mut pan = TouchPan::default();
        pan.read(&[touch(TouchPhase::Start, 600.0)], 0.0, AREA, true, Some(0.8));
        let second = Event::Touch { device_id: TouchDeviceId(1), id: TouchId(1), phase: TouchPhase::Start, pos: pos2(100.0, 300.0), force: None };
        let p = pan.read(&[second, touch(TouchPhase::Move, 500.0)], 0.016, AREA, true, Some(0.8));
        assert_eq!(p.delta, Vec2::ZERO, "two fingers pinch");

        let mut pan = TouchPan::default();
        pan.read(&[touch(TouchPhase::Start, 900.0)], 0.0, AREA, true, Some(0.8));
        assert!(!pan.owns_drag(), "a touch that came down off the pages");
        pan.read(&[touch(TouchPhase::Start, 600.0)], 0.0, AREA, false, Some(0.8));
        assert!(!pan.owns_drag(), "a tool that draws");
    }
}
