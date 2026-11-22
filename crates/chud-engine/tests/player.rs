//! Playback tests against a simulated platform: timing accuracy, pause,
//! stop, safety release and repeats.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use chud_core::document::{Environment, Monitor};
use chud_core::event::{EventKind, Key, MouseButton};
use chud_core::flow::{Block, Flow, Node};
use chud_core::playback::{Action, PlaybackSettings, Selection, TimingMode};
use chud_core::timeline::Timeline;
use chud_engine::player::{self, NoMacros, PlayEnd, PlayJob, PlayerShared};
use chud_platform::*;
use parking_lot::Mutex;

type Log = Arc<Mutex<Vec<(Instant, Action)>>>;

struct Mock {
    log: Log,
}

struct MockInjector {
    log: Log,
}

impl Injector for MockInjector {
    fn send(&mut self, actions: &[Action]) -> Result<()> {
        let now = Instant::now();
        let mut l = self.log.lock();
        for a in actions {
            l.push((now, *a));
        }
        Ok(())
    }
    fn type_char(&mut self, _: char) -> Result<()> {
        Ok(())
    }
}

impl Platform for Mock {
    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }
    fn start(&self, _: crossbeam_channel::Sender<Captured>) -> Result<()> {
        Ok(())
    }
    fn set_capture(&self, _: CaptureOptions) {}
    fn set_hotkeys(&self, _: Vec<Hotkey>) {}
    fn injector(&self) -> Result<Box<dyn Injector>> {
        Ok(Box::new(MockInjector { log: self.log.clone() }))
    }
    fn monitors(&self) -> Vec<Monitor> {
        vec![]
    }
    fn environment(&self) -> Environment {
        Environment::default()
    }
    fn cursor(&self) -> Option<(i32, i32)> {
        Some((0, 0))
    }
    fn set_cursor(&self, _: i32, _: i32) -> Result<()> {
        Ok(())
    }
    fn foreground(&self) -> Option<WindowInfo> {
        None
    }
    fn window_exists(&self, _: &str, _: &str) -> bool {
        false
    }
    fn pixel(&self, _: i32, _: i32) -> Option<(u8, u8, u8)> {
        None
    }
    fn key_held(&self, _: Key) -> bool {
        false
    }
    fn checks(&self) -> Vec<Check> {
        vec![]
    }
    fn boost_thread(&self) -> ThreadBoost {
        ThreadBoost::none()
    }
}

fn key(scan: u16) -> Key {
    Key::new(scan, 0x41)
}

fn job(items: Vec<(i64, EventKind)>, settings: PlaybackSettings) -> PlayJob {
    PlayJob {
        macro_id: None,
        name: "t".into(),
        timeline: Arc::new(Timeline::from_kinds(items)),
        env: Environment::default(),
        flow: None,
        use_flow: false,
        settings,
        selection: Selection::All,
        images: Arc::default(),
    }
}

fn run(job: &PlayJob, shared: &PlayerShared) -> (PlayEnd, Vec<(Instant, Action)>, Instant) {
    let log: Log = Arc::default();
    let mock = Mock { log: log.clone() };
    let (tx, _rx) = crossbeam_channel::unbounded();
    let start = Instant::now();
    let end = player::run_job(job, &mock, &NoMacros, shared, &tx);
    let l = log.lock().clone();
    (end, l, start)
}

#[test]
fn exact_timing_is_accurate() {
    // 200 key events every 2.5 ms.
    let items: Vec<_> = (0..200)
        .map(|i| {
            let k = key(0x10 + ((i / 2) % 8) as u16);
            let kind = if i % 2 == 0 { EventKind::KeyDown { key: k, repeat: false } } else { EventKind::KeyUp { key: k } };
            (i as i64 * 2_500, kind)
        })
        .collect();
    let j = job(items, PlaybackSettings::default());
    let shared = PlayerShared::default();
    let (end, log, start) = run(&j, &shared);
    assert_eq!(end, PlayEnd::Completed);
    assert_eq!(log.len(), 200);
    let mut errors: Vec<f64> = log
        .iter()
        .enumerate()
        .map(|(i, (t, _))| {
            let due = i as f64 * 2_500.0;
            t.duration_since(start).as_nanos() as f64 / 1_000.0 - due
        })
        .collect();
    errors.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = errors[errors.len() / 2];
    // Never early, and tightly on time (generous for loaded CI machines).
    assert!(errors[0] > -50.0, "early dispatch: {:?}", &errors[..3]);
    assert!(median < 500.0, "median error {median} µs");
}

#[test]
fn simultaneous_events_go_in_one_batch_in_order() {
    let ctrl = key(0x1D);
    let c = key(0x2E);
    let items = vec![
        (1_000, EventKind::KeyDown { key: ctrl, repeat: false }),
        (1_000, EventKind::KeyDown { key: c, repeat: false }),
        (1_000, EventKind::KeyUp { key: c }),
        (1_000, EventKind::KeyUp { key: ctrl }),
    ];
    let (_, log, _) = run(&job(items, PlaybackSettings::default()), &PlayerShared::default());
    assert_eq!(log.len(), 4);
    assert!(log.iter().all(|(t, _)| *t == log[0].0), "not dispatched together");
    assert!(matches!(log[0].1, Action::Key { key, down: true, .. } if key == ctrl));
    assert!(matches!(log[3].1, Action::Key { key, down: false, .. } if key == ctrl));
}

#[test]
fn stop_releases_everything_held() {
    let items = vec![
        (0, EventKind::KeyDown { key: key(0x11), repeat: false }),
        (0, EventKind::MouseDown { button: MouseButton::Left, x: 5, y: 5 }),
        (5_000_000, EventKind::MouseUp { button: MouseButton::Left, x: 5, y: 5 }),
        (5_000_000, EventKind::KeyUp { key: key(0x11) }),
    ];
    let j = job(items, PlaybackSettings::default());
    let shared = Arc::new(PlayerShared::default());
    let s2 = shared.clone();
    let stopper = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(60));
        s2.stop.store(true, Ordering::Release);
    });
    let (end, log, start) = run(&j, &shared);
    stopper.join().unwrap();
    assert_eq!(end, PlayEnd::Stopped);
    assert!(log.last().unwrap().0.duration_since(start) < Duration::from_millis(500));
    let ups: Vec<_> = log.iter().filter(|(_, a)| matches!(a, Action::Key { down: false, .. } | Action::Button { down: false, .. })).collect();
    assert_eq!(ups.len(), 2, "{log:?}");
    // Released in reverse press order: the mouse button first.
    assert!(matches!(ups[0].1, Action::Button { .. }));
}

#[test]
fn pause_releases_and_represses_then_resumes_on_schedule() {
    let w = key(0x11);
    let items = vec![
        (0, EventKind::KeyDown { key: w, repeat: false }),
        (100_000, EventKind::KeyUp { key: w }),
    ];
    let j = job(items, PlaybackSettings::default());
    let shared = Arc::new(PlayerShared::default());
    let s2 = shared.clone();
    let pauser = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(30));
        s2.paused.store(true, Ordering::Release);
        std::thread::sleep(Duration::from_millis(200));
        s2.paused.store(false, Ordering::Release);
    });
    let (end, log, start) = run(&j, &shared);
    pauser.join().unwrap();
    assert_eq!(end, PlayEnd::Completed);
    let kinds: Vec<bool> = log.iter().map(|(_, a)| matches!(a, Action::Key { down: true, .. })).collect();
    assert_eq!(kinds, vec![true, false, true, false]); // press, pause-release, re-press, final release
    let total = log.last().unwrap().0.duration_since(start);
    // The schedule shifts by the pause as observed by the player (polled every few ms).
    assert!(total >= Duration::from_millis(285) && total < Duration::from_millis(420), "{total:?}");
}

#[test]
fn repeats_with_delay_and_fast_mode() {
    let items = vec![
        (0, EventKind::KeyDown { key: key(0x1E), repeat: false }),
        (900_000, EventKind::KeyUp { key: key(0x1E) }),
    ];
    let settings = PlaybackSettings {
        repeat: 3,
        repeat_delay_ms: 20.0,
        timing: TimingMode::Fast,
        fast_gap_us: 1_000,
        ..Default::default()
    };
    let shared = PlayerShared::default();
    let (end, log, start) = run(&job(items, settings), &shared);
    assert_eq!(end, PlayEnd::Completed);
    assert_eq!(log.len(), 6);
    assert_eq!(shared.iteration.load(Ordering::Relaxed), 3);
    let total = log.last().unwrap().0.duration_since(start);
    assert!(total >= Duration::from_millis(42) && total < Duration::from_millis(150), "{total:?}");
}

#[test]
fn flow_drives_the_recording() {
    let items = vec![
        (0, EventKind::KeyDown { key: key(0x1E), repeat: false }),
        (1_000, EventKind::KeyUp { key: key(0x1E) }),
    ];
    let mut j = job(items, PlaybackSettings::default());
    j.use_flow = true;
    j.flow = Some(Flow {
        blocks: vec![Node::new(Block::Repeat {
            count: Some("4".into()),
            counter: "i".into(),
            body: vec![
                Node::new(Block::PlayRecording { start: None, end: None, speed: 1.0 }),
                Node::new(Block::Wait { ms: "2".into() }),
            ],
        })],
    });
    let (end, log, _) = run(&j, &PlayerShared::default());
    assert_eq!(end, PlayEnd::Completed);
    assert_eq!(log.len(), 8);
}
