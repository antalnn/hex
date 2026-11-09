//! The live text plate (Labs): one click-through line
//! beside the recording HUD with what the dictation has said so far, on
//! every OS. It shows while a recording has text and hides when the
//! recording ends ([`crate::dictation::live_text`]); `rune_hud` says where the HUD is,
//! so the plate sits just past it, on the HUD's screen. Labs › Failure
//! notices reuses the same plate for one line saying why a dictation failed,
//! for a few seconds or until the next capture.
//!
//! The overlay is prewarmed: macOS keeps it drawn while hidden and refreshes
//! it before a reveal, so it never reappears with the last dictation's
//! words for a frame.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use rune::{
    Anim, Cx, El, Icon, OverlayHandle, OverlayOptions, OverlayPlacement, OverlaySpec, ScreenEdge,
    Spring, row, text,
};

use super::controls::Paint;
use crate::transcription::engine::Partial;

/// The overlay's size: room for about a dozen words on one line.
pub(crate) const SIZE: (f32, f32) = (560.0, 44.0);
/// The plate's height inside the overlay.
const PLATE_H: f32 = 32.0;
/// The overlay's room around the plate, across its height.
const PAD: f32 = (SIZE.1 - PLATE_H) / 2.0;
/// How far a [`Spot::Edge`] margin sits from the plate's middle.
pub(crate) const PLATE_CENTER: f32 = SIZE.1 / 2.0 - PAD;
/// How long the unified failure pill shows its exclamation alone before it
/// expands to the message: 0.75 to 1 second, per the indicator's goal.
pub(crate) const FAILURE_MARK_VISIBLE: Duration = Duration::from_millis(750);
/// How long the expanded pill holds the error text before it fades: 2.5 to
/// 3 seconds, and never longer.
pub(crate) const FAILURE_TEXT_VISIBLE: Duration = Duration::from_millis(2500);
/// The unified failure pill's height.
const FAILURE_PLATE_H: f32 = 36.0;
/// The pill's width while it shows the exclamation alone: the HUD's own
/// processing plate, so the pill appears where the plate was.
const FAILURE_MARK_W: f32 = 76.0;
/// The HUD's failed mark, drawn on a 24 point grid: the plate's soft danger
/// disc behind a solid exclamation. The disc is 20 pt across and the mark
/// 13 pt tall when the icon is drawn at 24 pt.
static FAILURE_MARK_ICON: Icon = Icon {
    name: "hex-failure-mark",
    view: 24.0,
    body: "<circle cx=\"12\" cy=\"12\" r=\"10\" fill-opacity=\"0.16\"/><path d=\"M10.605 6.885a1.395 1.395 0 0 1 2.79 0v6.355a1.395 1.395 0 0 1-2.79 0ZM10.45 16.805a1.55 1.55 0 1 0 3.1 0a1.55 1.55 0 1 0-3.1 0Z\"/>",
};

/// The two looks of the unified failure pill.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum FailurePhase {
    /// The exclamation alone, before the pill expands.
    #[default]
    Mark,
    /// The expanded pill, holding the message.
    Text,
}

/// One failure, as the unified pill draws it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Failure {
    message: String,
    phase: FailurePhase,
}

#[derive(Default)]
pub(crate) struct Plate {
    stable: String,
    tentative: String,
    /// Labs › Failure notices: why the last dictation failed, drawn as one
    /// pill that starts as its exclamation and expands to the message.
    failure: Option<Failure>,
    /// Whether live words are being published (a recording is showing text).
    live: bool,
    /// Whether the overlay is on screen. Every change happens in an overlay
    /// update, in queue order, so a queued hide cannot race a newer notice.
    shown: bool,
}

impl Plate {
    fn set_words(&mut self, stable: String, tentative: String) {
        self.stable = stable;
        self.tentative = tentative;
        self.live = true;
    }

    /// The recording stopped publishing words. Its last words stay drawn
    /// while the plate fades.
    fn words_ended(&mut self) {
        self.live = false;
    }

    fn show_failure(&mut self, message: String) {
        self.failure = Some(Failure {
            message,
            phase: FailurePhase::Mark,
        });
    }

    /// Expands the pill to the message it already carries.
    fn expand_failure(&mut self) {
        if let Some(failure) = &mut self.failure {
            failure.phase = FailurePhase::Text;
        }
    }

    fn take_failure(&mut self) -> bool {
        self.failure.take().is_some()
    }

    fn mark_shown(&mut self, shown: bool) {
        self.shown = shown;
    }

    /// Nothing live or failed remains to justify the plate.
    fn should_hide(&self) -> bool {
        !self.live && self.failure.is_none()
    }
}

/// A failure notice's message and deadlines, kept apart from the drawn pill
/// so a timer thread can advance it without the live text's state.
#[derive(Default)]
struct FailureNotice {
    message: Option<String>,
    phase: FailurePhase,
    /// Distinguishes this notice from a later one, so a stale timer cannot
    /// expand or dismiss it.
    serial: u64,
    expands_at: Option<Instant>,
    expires_at: Option<Instant>,
}

impl FailureNotice {
    /// Replaces any notice with `message`: its mark shows at once, its text
    /// after [`FAILURE_MARK_VISIBLE`], and it leaves [`FAILURE_TEXT_VISIBLE`]
    /// later. Returns the serial and both deadlines.
    fn show(&mut self, message: String, now: Instant) -> (u64, Instant, Instant) {
        self.serial = self.serial.wrapping_add(1);
        self.message = Some(message);
        self.phase = FailurePhase::Mark;
        let expands = now + FAILURE_MARK_VISIBLE;
        let expires = expands + FAILURE_TEXT_VISIBLE;
        self.expands_at = Some(expands);
        self.expires_at = Some(expires);
        (self.serial, expands, expires)
    }

    /// Expands the pill once its own mark has had its time.
    fn expand(&mut self, serial: u64, now: Instant) -> bool {
        if self.serial != serial || self.phase != FailurePhase::Mark {
            return false;
        }
        if self.expands_at.is_none_or(|at| now < at) {
            return false;
        }
        self.phase = FailurePhase::Text;
        true
    }

    /// A new capture dismisses the notice at once; whether one was showing.
    fn capture_started(&mut self) -> bool {
        self.expands_at = None;
        self.expires_at = None;
        self.serial = self.serial.wrapping_add(1);
        self.phase = FailurePhase::Mark;
        self.message.take().is_some()
    }

    /// Dismisses a notice whose own deadline is due; whether it was showing.
    fn expire(&mut self, serial: u64, now: Instant) -> bool {
        if self.serial != serial || self.expires_at.is_none_or(|at| now < at) {
            return false;
        }
        self.expands_at = None;
        self.expires_at = None;
        self.message.take().is_some()
    }
}

/// Where the plate starts, just past the HUD.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Spot {
    /// This far from the edge that the HUD is on, on the HUD's display
    /// (`rune_hud::display`).
    Edge(ScreenEdge, f32),
    /// At `y` in Rune's global space (points from the primary display's
    /// top-left, y down), centred on `x`; the plate hangs down from `y`
    /// when `below`, else rises from it.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Global { x: f32, y: f32, below: bool },
}

impl Spot {
    /// The overlay's placement for the plate to start at this spot.
    fn placement(self) -> OverlayPlacement {
        match self {
            Self::Edge(edge, margin) => OverlayPlacement::Edge {
                edge,
                margin: margin - PAD,
                align: rune::Align::Center,
                inset: 0.0,
            },
            Self::Global { x, y, below } => OverlayPlacement::Rect {
                x: x - SIZE.0 / 2.0,
                y: if below { y - PAD } else { y - SIZE.1 + PAD },
                width: SIZE.0,
                height: SIZE.1,
            },
        }
    }

    /// Places `overlay` for the plate to start at this spot.
    fn apply(self, overlay: &OverlayHandle<Plate>) {
        // Off macOS the HUD is placed with `place`, which picks the display
        // again and measures from the same edge of its work area (above the
        // Windows taskbar), so the plate is placed the same way and never
        // lands under the HUD (#797).
        #[cfg(not(target_os = "macos"))]
        if let Self::Edge(edge, margin) = self {
            overlay.place(edge, margin - PAD);
            return;
        }
        overlay.set_placement(Some(self.placement()));
    }
}

/// The plate's overlay, once installed.
static LIVE: std::sync::OnceLock<rune::OverlayHandle<Plate>> = std::sync::OnceLock::new();

/// Whether a non-blank partial has been queued since the last words-ended
/// update. It lets that update be skipped while nothing was ever shown,
/// without deciding visibility outside the overlay's queue.
static WORDS: AtomicBool = AtomicBool::new(false);

/// Where the plate goes, kept for a failure notice that appears after
/// installation.
static PLACE: OnceLock<Place> = OnceLock::new();

/// Where the unified failure pill goes: on the HUD's plate, so it takes the
/// plate's place instead of sitting beside it.
static FAILURE_PLACE: OnceLock<Place> = OnceLock::new();

/// The current failure notice, which a timer thread advances and dismisses.
static NOTICE: Mutex<FailureNotice> = Mutex::new(FailureNotice {
    message: None,
    phase: FailurePhase::Mark,
    serial: 0,
    expands_at: None,
    expires_at: None,
});

/// The timer thread's queue: each notice's own deadlines arrive in order.
static TIMER: OnceLock<Sender<TimerWake>> = OnceLock::new();

fn notice() -> std::sync::MutexGuard<'static, FailureNotice> {
    NOTICE.lock().unwrap_or_else(|error| error.into_inner())
}

/// Moves a shown plate to the display under the pointer when it changed
/// (`rune_hud`'s follow, #807).
#[cfg(not(target_os = "macos"))]
pub(crate) fn refollow_pointer() {
    if let Some(overlay) = LIVE.get() {
        overlay.refollow_pointer();
    }
}

/// Where the plate goes. Called on the app's thread each time the plate
/// appears, so it may read the screen.
pub(crate) type Place = fn() -> Spot;

/// Adds the plate to the app. It shows while a dictation publishes its text
/// (Labs › Live text) or while a failure notice explains a failed dictation
/// (Labs › Failure notices), applied at once. On macOS it's always prewarmed,
/// since either lab may be turned on after launch and an unprewarmed plate
/// reappears for a frame with the last dictation's words (#1075); elsewhere
/// when either is on at launch.
pub(crate) fn install<S: 'static>(
    app: &mut rune::App<S>,
    place: Place,
    failure_place: Option<Place>,
) {
    let spec = OverlaySpec {
        name: "Hex live text",
        size: SIZE,
        // Placed for real each time it appears.
        edge: ScreenEdge::Top,
        margin: 0.0,
        align: rune::Align::Center,
        inset: 0.0,
        interactive: false,
        view,
    }
    .options(OverlayOptions {
        prewarm: cfg!(target_os = "macos")
            || crate::labs::live_text()
            || crate::labs::failure_notices(),
        #[cfg(not(target_os = "macos"))]
        display: crate::rune_hud::display(),
        ..Default::default()
    })
    // Hidden, it holds no GPU surface (nor, on Windows, a window), so it
    // never passes for a game to capture tools (#1308).
    .release_gpu_when_hidden();
    let overlay = app.overlay(spec, Plate::default());
    let _ = LIVE.set(overlay.clone());
    let _ = PLACE.set(place);
    let _ = FAILURE_PLACE.set(failure_place.unwrap_or(place));
    crate::dictation::live_text::watch(move |partial| {
        let partial = partial.filter(|partial| !is_blank(partial));
        let Some(Partial { stable, tentative }) = partial.cloned() else {
            // Nothing was published since the last end and no notice is up:
            // this is a no-op, and skipping it keeps a dictation that never
            // showed text off the update queue.
            let words = WORDS.swap(false, Ordering::AcqRel);
            let notice_up = crate::labs::failure_notices() && notice().message.is_some();
            if !words && !notice_up {
                return;
            }
            let handle = overlay.clone();
            overlay.update(move |plate, _| {
                plate.words_ended();
                if plate.should_hide() && plate.shown {
                    // Hidden from the same queue that shows it, so a recording
                    // that ends as its plate appears never leaves it showing.
                    // Its last words stay while it fades (macOS and Windows);
                    // the next appearance replaces them first.
                    handle.fade_out(FADE);
                    plate.mark_shown(false);
                }
            });
            return;
        };
        WORDS.store(true, Ordering::Release);
        let handle = overlay.clone();
        overlay.update(move |plate, tx| {
            plate.set_words(stable, tentative);
            tx.instant();
            if plate.failure.is_none() && !plate.shown {
                // Placed and shown after the new words, so it appears
                // where it belongs with them already drawn.
                place().apply(&handle);
                handle.show();
                plate.mark_shown(true);
            }
        });
    });
}

/// Shows `message` as the unified failure pill (Labs › Failure notices): the
/// exclamation alone for [`FAILURE_MARK_VISIBLE`], the message for
/// [`FAILURE_TEXT_VISIBLE`] after it, then the usual fade. It replaces any
/// live words and takes the HUD plate's place.
pub(crate) fn show_failure(message: &str) {
    if !crate::labs::failure_notices() {
        return;
    }
    let (Some(overlay), Some(place)) = (LIVE.get(), FAILURE_PLACE.get().copied()) else {
        return;
    };
    let message = crate::desktop::activity::failure_explanation(message);
    let (serial, expands, expires) = notice().show(message.clone(), Instant::now());
    schedule(TimerWake::Expand(serial, expands));
    schedule(TimerWake::Expire(serial, expires));
    let handle = overlay.clone();
    overlay.update(move |plate, tx| {
        plate.show_failure(message);
        tx.instant();
        // The pill takes the HUD plate's place, even when the live words
        // were showing beside it.
        place().apply(&handle);
        if !plate.shown {
            handle.show();
            plate.mark_shown(true);
        }
    });
}

/// Hides a failure notice at once when a new capture starts; live words
/// already on the plate stay.
pub(crate) fn capture_started() {
    dismiss_notice();
}

/// Hides a failure notice when Labs › Failure notices is turned off.
pub(crate) fn dismiss_failure() {
    dismiss_notice();
}

fn dismiss_notice() {
    let cleared = { notice().capture_started() };
    if !cleared {
        return;
    }
    let Some(overlay) = LIVE.get() else {
        return;
    };
    let handle = overlay.clone();
    overlay.update(move |plate, tx| {
        // A failure that arrived after this dismissal keeps its notice.
        if notice().message.is_some() {
            return;
        }
        if plate.take_failure() {
            tx.instant();
        }
        if plate.should_hide() && plate.shown {
            handle.fade_out(FADE);
            plate.mark_shown(false);
        }
    });
}

/// One scheduled step of the unified failure pill.
#[derive(Clone, Copy, Debug)]
enum TimerWake {
    /// Show the message in the pill.
    Expand(u64, Instant),
    /// Fade the pill away.
    Expire(u64, Instant),
}

impl TimerWake {
    fn at(self) -> Instant {
        match self {
            Self::Expand(_, at) | Self::Expire(_, at) => at,
        }
    }
}

/// Queues `wake` to the timer thread, starting it on the first notice so a
/// Hex with the lab off runs no notice thread.
fn schedule(wake: TimerWake) {
    let sender = TIMER.get_or_init(|| {
        let (sender, receiver) = mpsc::channel();
        let _ = std::thread::Builder::new()
            .name("hex-failure-notice".into())
            .spawn(move || run_timer(receiver));
        sender
    });
    let _ = sender.send(wake);
}

fn run_timer(receiver: Receiver<TimerWake>) {
    let mut pending: Vec<TimerWake> = Vec::new();
    loop {
        if pending.is_empty() {
            let Ok(wake) = receiver.recv() else {
                return;
            };
            pending.push(wake);
        }
        pending.sort_by_key(|wake| wake.at());
        let at = pending[0].at();
        let now = Instant::now();
        if now >= at {
            match pending.remove(0) {
                TimerWake::Expand(serial, _) => expand_notice(serial),
                TimerWake::Expire(serial, _) => hide_expired(serial),
            }
            continue;
        }
        match receiver.recv_timeout(at - now) {
            Ok(wake) => pending.push(wake),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Expands the pill to its message at the notice's own mark deadline.
fn expand_notice(serial: u64) {
    let expanded = { notice().expand(serial, Instant::now()) };
    if !expanded {
        return;
    }
    let Some(overlay) = LIVE.get() else {
        return;
    };
    overlay.update(move |plate, _| plate.expand_failure());
}

fn hide_expired(serial: u64) {
    let expired = { notice().expire(serial, Instant::now()) };
    if !expired {
        return;
    }
    let Some(overlay) = LIVE.get() else {
        return;
    };
    let handle = overlay.clone();
    overlay.update(move |plate, tx| {
        // A failure that arrived after this deadline keeps its notice.
        if notice().message.is_some() {
            return;
        }
        if plate.take_failure() {
            tx.instant();
        }
        if plate.should_hide() && plate.shown {
            handle.fade_out(FADE);
            plate.mark_shown(false);
        }
    });
}

/// How long the plate fades out once a dictation stops publishing.
const FADE: Duration = Duration::from_millis(150);

/// The model's first update often has no words yet.
fn is_blank(partial: &Partial) -> bool {
    partial.stable.trim().is_empty() && partial.tentative.trim().is_empty()
}

/// About one space of the plate's font, between right-to-left stable and
/// tentative words.
const RTL_SPACE: f32 = 4.0;

/// Whether the line's first strong letter is right-to-left (Hebrew, Arabic,
/// Persian, Syriac, Thaana, N'Ko, and their presentation forms).
fn is_right_to_left(stable: &str, tentative: &str) -> bool {
    stable
        .chars()
        .chain(tentative.chars())
        .find(|c| c.is_alphabetic() || is_rtl_letter(*c))
        .is_some_and(is_rtl_letter)
}

fn is_rtl_letter(c: char) -> bool {
    matches!(
        c,
        '\u{0590}'..='\u{08FF}' | '\u{FB1D}'..='\u{FDFF}' | '\u{FE70}'..='\u{FEFF}'
    )
}

fn plate(p: &Paint, line: El) -> El {
    let c = &p.k.c;
    row()
        .key("plate")
        .h(PLATE_H)
        .max_w(SIZE.0 - 8.0)
        .px(14.0)
        .center_y()
        .radius(PLATE_H / 2.0)
        .fill(c.bg_base)
        .border(p.hair, c.border_base)
        .child(line)
}

/// The spring the unified failure pill expands on.
const EXPAND: Spring = Spring::new(0.28, 0.0);

/// The unified failure pill: the HUD's original mark alone, then the message
/// as the pill widens. The mark leaves with the phase change, so the expanded
/// pill holds the words alone.
fn failure_pill(p: &Paint, failure: &Failure) -> El {
    let c = &p.k.c;
    let mark = || {
        rune::icon(&FAILURE_MARK_ICON, 24.0)
            .key("failure-mark")
            .color(p.danger())
    };
    let pill = row()
        .key("failure-pill")
        .h(FAILURE_PLATE_H)
        .px(14.0)
        .center_y()
        .radius(FAILURE_PLATE_H / 2.0)
        .fill(c.bg_base)
        .border(p.hair, c.border_base)
        .motion(EXPAND);
    match failure.phase {
        FailurePhase::Mark => pill.w(FAILURE_MARK_W).center().child(mark()),
        FailurePhase::Text => {
            let message = text(failure.message.clone())
                .key("failure")
                .truncate()
                .font(p.k.ty.ui)
                .color(p.danger());
            pill.max_w(SIZE.0 - 8.0).child(message)
        }
    }
}

fn view(plate_state: &Plate, cx: &mut Cx<Plate>) -> El {
    let p = Paint::new(cx.system_dark(), cx.env().scale);
    let root = row()
        .key("live-text")
        .w_full()
        .h_full()
        .center()
        .px(4.0)
        .hover_motion_within(Anim::INSTANT, Anim::INSTANT);
    if let Some(failure) = &plate_state.failure {
        return root.child(failure_pill(&p, failure));
    }
    if plate_state.stable.trim().is_empty() && plate_state.tentative.trim().is_empty() {
        return root;
    }
    // The newest words stay in view: the start gives way with "…".
    let stable = text(plate_state.stable.trim().to_owned())
        .key("stable")
        .truncate_head()
        .font(p.k.ty.ui)
        .color(p.text());
    let rtl = is_right_to_left(&plate_state.stable, &plate_state.tentative);
    let tentative = (!plate_state.tentative.trim().is_empty()).then(|| {
        let words = text(if rtl {
            // A leading space would sit on the right-to-left line's far
            // (right) side; the gap goes between the words instead.
            plate_state.tentative.trim().to_owned()
        } else {
            plate_state.tentative.clone()
        })
        .key("tentative")
        .truncate()
        .font(p.k.ty.ui)
        .color(p.faint());
        let spaced = rtl
            && !plate_state.stable.trim().is_empty()
            && plate_state.tentative.starts_with(char::is_whitespace);
        if spaced { words.mr(RTL_SPACE) } else { words }
    });
    let line = row().key("line").min_w(0.0).shrink();
    // Right-to-left, the newer (tentative) words read after the stable ones,
    // so they sit to their left.
    let line = if rtl {
        line.children(tentative).child(stable)
    } else {
        line.child(stable).children(tentative)
    };
    root.child(plate(&p, line))
}

#[cfg(test)]
mod tests {
    use rune::Runtime;
    use rune::text::TextSystem;

    use super::*;

    const SHORT: &str = "So the plan for today";
    const LONG: &str = "So the plan for today is to finish the release notes, then check the \
        streaming numbers on the M4, and after lunch we can review the onboarding flow together";
    const HEBREW: &str = "אז התוכנית להיום";

    #[test]
    fn a_failure_pill_shows_its_mark_then_holds_the_text() {
        assert!(FAILURE_MARK_VISIBLE >= Duration::from_millis(750));
        assert!(FAILURE_MARK_VISIBLE <= Duration::from_secs(1));
        assert!(FAILURE_TEXT_VISIBLE >= Duration::from_millis(2500));
        assert!(FAILURE_TEXT_VISIBLE <= Duration::from_millis(3000));

        let start = Instant::now();
        let mut notice = FailureNotice::default();
        let (serial, expands, expires) = notice.show(
            "No microphone is available. Connect a microphone.".into(),
            start,
        );
        assert_eq!(notice.phase, FailurePhase::Mark);
        assert_eq!(expires - expands, FAILURE_TEXT_VISIBLE);
        assert!(!notice.expand(serial, expands - Duration::from_millis(1)));
        assert_eq!(notice.phase, FailurePhase::Mark);
        assert!(notice.expand(serial, expands));
        assert_eq!(notice.phase, FailurePhase::Text);
        assert!(!notice.expire(serial, expires - Duration::from_millis(1)));
        assert!(notice.expire(serial, expires));
        assert_eq!(notice.message, None);
    }

    #[test]
    fn a_new_capture_dismisses_a_failure_notice_at_once() {
        let start = Instant::now();
        let mut notice = FailureNotice::default();
        let (serial, expands, expires) = notice.show("Could not open microphone".into(), start);
        assert!(notice.capture_started());
        assert_eq!(notice.message, None);
        assert!(!notice.capture_started());
        assert!(
            !notice.expand(serial, expands),
            "the old mark deadline cannot expand a later notice"
        );
        assert!(
            !notice.expire(serial, expires),
            "the old deadline cannot dismiss a later notice"
        );
    }

    #[test]
    fn only_a_notices_own_deadlines_advance_it() {
        let start = Instant::now();
        let mut notice = FailureNotice::default();
        let (older, older_expands, older_expires) = notice.show("first".into(), start);
        let (newer, newer_expands, newer_expires) =
            notice.show("second".into(), start + Duration::from_secs(1));
        assert_ne!(older, newer);
        assert!(!notice.expand(older, older_expands));
        assert_eq!(notice.phase, FailurePhase::Mark);
        assert!(notice.expand(newer, newer_expands));
        assert!(!notice.expire(newer, older_expires));
        assert_eq!(notice.message.as_deref(), Some("second"));
        assert!(notice.expire(newer, newer_expires));
        assert_eq!(notice.message, None);
    }

    fn runtime(stable: &str, tentative: &str, dark: bool) -> Runtime<Plate> {
        runtime_plate(
            Plate {
                stable: stable.into(),
                tentative: tentative.into(),
                ..Plate::default()
            },
            dark,
        )
    }

    fn runtime_plate(plate: Plate, dark: bool) -> Runtime<Plate> {
        let mut rt = crate::rune_ui::offscreen::runtime(
            plate,
            view,
            SIZE,
            rune::Env {
                system_dark: dark,
                scale: 2.0,
            },
            TextSystem::new(rune_theme_opencode::FONTS, rune_theme_opencode::FAMILIES),
        );
        rt.settle_all();
        rt.frame(0.0);
        rt
    }

    fn rect(rt: &Runtime<Plate>, key: &str) -> Option<[f32; 4]> {
        rt.elements()
            .into_iter()
            .find(|e| !e.exiting && e.key.as_deref() == Some(key))
            .map(|e| e.rect)
    }

    #[test]
    fn a_short_line_hugs_its_words_in_the_middle() {
        let rt = runtime(SHORT, "", false);
        let [x, y, w, h] = rect(&rt, "plate").expect("the plate draws");
        assert!(w < SIZE.0 / 2.0, "the plate hugs a short line: {w}");
        assert!((x + w / 2.0 - SIZE.0 / 2.0).abs() < 1.0, "centred: {x} {w}");
        assert!((y + h / 2.0 - SIZE.1 / 2.0).abs() < 1.0, "centred: {y} {h}");
        assert!(rect(&rt, "tentative").is_none());
    }

    #[test]
    fn a_long_line_stays_inside_the_overlay_on_one_line() {
        for tentative in ["", " together with the team"] {
            let rt = runtime(LONG, tentative, true);
            let [x, _, w, h] = rect(&rt, "plate").unwrap();
            assert!(x >= 0.0 && x + w <= SIZE.0, "inside: {x} {w}");
            assert_eq!(h, PLATE_H);
            let [line_x, _, line_w, _] = rect(&rt, "line").unwrap();
            assert!(
                line_x + line_w <= x + w,
                "the words fit the plate: {line_x} {line_w}"
            );
            let [_, _, _, line] = rect(&rt, "stable").unwrap();
            assert!(line < PLATE_H, "one line: {line}");
            assert_eq!(rect(&rt, "tentative").is_some(), !tentative.is_empty());
        }
    }

    #[test]
    fn tentative_words_follow_the_stable_ones_in_reading_order() {
        let rt = runtime(SHORT, " is to finish", false);
        let [sx, _, sw, _] = rect(&rt, "stable").unwrap();
        let [tx, _, _, _] = rect(&rt, "tentative").unwrap();
        assert!(tx >= sx + sw - 0.5, "left to right: {sx} {sw} {tx}");

        let rt = runtime(HEBREW, " והיום", false);
        let [sx, _, _, _] = rect(&rt, "stable").unwrap();
        let [tx, _, tw, _] = rect(&rt, "tentative").unwrap();
        assert!(tx + tw < sx, "right to left, after a space: {tx} {tw} {sx}");

        assert!(is_right_to_left(HEBREW, ""));
        assert!(is_right_to_left("  ", " مرحبا"));
        assert!(!is_right_to_left(SHORT, " שלום"));
        assert!(!is_right_to_left("", ""));
    }

    #[test]
    fn a_spot_places_the_plate_just_past_the_hud() {
        assert_eq!(
            Spot::Edge(ScreenEdge::Bottom, 50.0).placement(),
            OverlayPlacement::Edge {
                edge: ScreenEdge::Bottom,
                margin: 50.0 - PAD,
                align: rune::Align::Center,
                inset: 0.0,
            }
        );
        let rect = |below| match (Spot::Global {
            x: 700.0,
            y: 100.0,
            below,
        })
        .placement()
        {
            OverlayPlacement::Rect { x, y, width, .. } => (x, y, width),
            other => panic!("{other:?}"),
        };
        // Centred on the HUD; the plate's top edge at the spot when hanging
        // below it, its bottom edge at the spot when rising above it.
        assert_eq!(rect(true), (700.0 - SIZE.0 / 2.0, 100.0 - PAD, SIZE.0));
        assert_eq!(
            rect(false),
            (700.0 - SIZE.0 / 2.0, 100.0 - PAD - PLATE_H, SIZE.0)
        );
    }

    #[test]
    fn a_failure_arriving_after_a_queued_words_end_stays_visible() {
        let mut plate = Plate::default();
        plate.set_words("old words".into(), String::new());
        plate.mark_shown(true);

        // A queued words-ended update runs first and would hide an empty
        // plate, but the failure queued behind it brings the plate back.
        plate.words_ended();
        assert!(plate.should_hide());
        plate.show_failure("No microphone is available.".into());
        assert!(!plate.should_hide());
        assert_eq!(plate.stable, "old words", "the old words stay for the fade");

        // Once the notice is up, a later words end cannot hide it.
        plate.words_ended();
        assert!(!plate.should_hide());
        assert!(plate.take_failure());
        assert!(plate.should_hide());
        plate.mark_shown(false);
    }

    #[test]
    fn a_failure_notice_takes_the_plate_from_the_live_words() {
        let rt = runtime_plate(
            Plate {
                stable: SHORT.into(),
                tentative: " and then".into(),
                failure: Some(Failure {
                    message: "No microphone is available. Connect a microphone.".into(),
                    phase: FailurePhase::Text,
                }),
                ..Plate::default()
            },
            false,
        );
        assert!(rect(&rt, "failure-pill").is_some());
        assert!(rect(&rt, "failure").is_some());
        assert!(
            rect(&rt, "failure-mark").is_none(),
            "the expanded pill holds the message alone"
        );
        assert!(rect(&rt, "stable").is_none());
        assert!(rect(&rt, "tentative").is_none());
    }

    #[test]
    fn the_failure_mark_is_a_soft_disc_behind_the_exclamation() {
        let px = 64;
        let coverage = rune::text::icon_coverage(&FAILURE_MARK_ICON, px);
        let at = |x: usize, y: usize| coverage[y * px as usize + x];
        // The disc is soft: 16% of the danger colour, inside but beside the bar.
        let disc = at(8, 32);
        assert!(disc > 30 && disc < 70, "the disc is soft: {disc}");
        // The bar and the dot are solid on top of it.
        assert!(at(32, 22) > 245, "the bar is solid: {}", at(32, 22));
        assert!(at(32, 46) > 245, "the dot is solid: {}", at(32, 46));
        assert!(at(38, 22) < 70, "the disc around the bar: {}", at(38, 22));
    }

    #[test]
    fn the_unified_pill_starts_as_its_mark_and_grows_to_the_message() {
        let mark = runtime_plate(
            Plate {
                failure: Some(Failure {
                    message: "the rewrite was cancelled".into(),
                    phase: FailurePhase::Mark,
                }),
                ..Plate::default()
            },
            true,
        );
        assert!(rect(&mark, "failure-pill").is_some());
        assert!(rect(&mark, "failure-mark").is_some());
        assert!(
            rect(&mark, "failure").is_none(),
            "the message waits for the expansion"
        );
        let [_, _, width, height] = rect(&mark, "failure-pill").unwrap();
        assert_eq!(height, FAILURE_PLATE_H);
        assert!(
            (width - FAILURE_MARK_W).abs() < 1.0,
            "the mark pill keeps the HUD's plate width: {width}"
        );

        // The expansion springs rather than snapping: a frame after the
        // phase changes sits between the mark and the message's width.
        let mut growing = mark;
        growing.settle_all();
        growing.frame(0.0);
        growing.update(|plate, _| plate.expand_failure());
        growing.frame(0.02);
        let between = rect(&growing, "failure-pill").unwrap()[2];
        growing.settle_all();
        growing.frame(0.02);
        let [_, _, width, _] = rect(&growing, "failure-pill").unwrap();
        assert!(
            between > FAILURE_MARK_W + 0.5 && between < width - 0.5,
            "the pill expands between {FAILURE_MARK_W} and {width}: {between}"
        );
        assert!(rect(&growing, "failure").is_some());
        assert!(
            rect(&growing, "failure-mark").is_none(),
            "the mark leaves as the pill expands"
        );
        assert!(
            width > FAILURE_MARK_W + 20.0,
            "the expanded pill fits the message: {width}"
        );
    }

    #[test]
    fn no_words_draw_nothing() {
        let rt = runtime("  ", "", false);
        assert!(rect(&rt, "plate").is_none());
        let blank = |stable: &str, tentative: &str| {
            is_blank(&Partial {
                stable: stable.into(),
                tentative: tentative.into(),
            })
        };
        assert!(blank("", ""));
        assert!(blank("  ", " "));
        assert!(!blank("", " so"));
        assert!(!blank("so", ""));
    }

    /// Renders the plate to `HEX_LIVE_TEXT_SNAPS/live-text-*.png` in both
    /// appearances, to look at.
    #[test]
    #[ignore = "set HEX_LIVE_TEXT_SNAPS to a folder; needs a GPU"]
    fn snapshots() {
        let folder = std::path::PathBuf::from(std::env::var("HEX_LIVE_TEXT_SNAPS").unwrap());
        let gpu = rune::render::Gpu::try_new(rune::render::Gpu::instance(), None).unwrap();
        let save = |mut rt: Runtime<Plate>, name: &str, dark: bool| {
            let (w, h) = ((SIZE.0 * 2.0) as u32, (SIZE.1 * 2.0) as u32);
            let mut renderer = crate::rune_ui::offscreen::renderer(&gpu.device);
            let target = crate::rune_ui::offscreen::target(&gpu.device, w, h);
            renderer.upload_images(&gpu.device, &gpu.queue, &mut rt.images);
            renderer.prepare(
                &gpu.device,
                &gpu.queue,
                &mut rt.text.atlas,
                &rt.instances,
                (w, h),
            );
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            // A desktop grey behind the clear overlay.
            let backdrop = if dark { 0.08 } else { 0.62 };
            renderer.draw(
                &mut encoder,
                &target.view,
                [backdrop, backdrop, backdrop, 1.0],
            );
            target.copy(&mut encoder);
            gpu.queue.submit([encoder.finish()]);
            let theme = if dark { "dark" } else { "light" };
            let path = folder.join(format!("live-text-{name}-{theme}.png"));
            rune::render::write_png(&path, w, h, &target.read(&gpu.device)).unwrap();
        };
        for (name, stable, tentative) in [
            ("short", SHORT, ""),
            ("long", LONG, ""),
            ("tentative", SHORT, " is to finish"),
            ("rtl", HEBREW, " והיום"),
        ] {
            for dark in [false, true] {
                save(runtime(stable, tentative, dark), name, dark);
            }
        }
        // The unified failure pill: the exclamation alone, then the message.
        for (name, phase) in [
            ("failure-mark", FailurePhase::Mark),
            ("failure-text", FailurePhase::Text),
        ] {
            for dark in [false, true] {
                let plate = Plate {
                    failure: Some(Failure {
                        message: "the rewrite was cancelled".into(),
                        phase,
                    }),
                    ..Plate::default()
                };
                save(runtime_plate(plate, dark), name, dark);
            }
        }
    }
}
