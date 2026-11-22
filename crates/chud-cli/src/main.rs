//! `chud` — command line companion of Chud Input Recorder.
//!
//! ```text
//! chud info <file.chud>                 describe a macro
//! chud play <file.chud> [--speed X] [--repeat N] [--mouse absolute|relative|off]
//! chud record <out.chud> [--seconds N]  record (F9 also stops)
//! chud export <file.chud> <out.json>    full JSON export
//! chud diag timer                       wake-up precision of this machine
//! chud diag loopback                    inject → capture fidelity test (moves the
//!                                       pointer briefly, presses only F13–F24,
//!                                       drives a virtual gamepad when available)
//! chud diag checks                      permissions and capabilities
//! chud diag gamepad [--seconds N]       plugs in a virtual controller and moves it
//!                                       (check that a game sees Chud's gamepad)
//! chud screen capture <out.png>         saves the screen    [--region x,y,w,h]
//! chud screen find <image.png>          looks for a picture [--threshold 0.9]
//!                                       [--any-scale] [--all] [--region x,y,w,h]
//! chud screen text [words]              reads the screen (Windows OCR) or finds
//!                                       words in it         [--region x,y,w,h]
//! ```

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chud_core::document::MacroDoc;
use chud_core::event::{EventKind, Key, PadAxis, PadButton};
use chud_core::format;
use chud_core::playback::{Action, KeyVia, MouseMode, PlaybackSettings, Selection};
use chud_engine::{Engine, EngineEvent, NoMacros, PlayJob, RecordOptions};
use chud_platform::timer::{JitterStats, measure_sleep_jitter};
use chud_platform::{CaptureOptions, Captured, Platform};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match run(&args) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    };
    std::process::exit(code);
}

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(String::as_str)
}

fn run(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("info") => info(args.get(1).ok_or("missing file")?),
        Some("play") => play(args.get(1).ok_or("missing file")?, args),
        Some("record") => record(args.get(1).ok_or("missing output file")?, args),
        Some("export") => {
            let doc = format::load(&PathBuf::from(args.get(1).ok_or("missing file")?)).map_err(|e| e.to_string())?;
            let json = format::to_json(&doc).map_err(|e| e.to_string())?;
            std::fs::write(args.get(2).ok_or("missing output")?, json).map_err(|e| e.to_string())
        }
        Some("diag") => match args.get(1).map(String::as_str) {
            Some("timer") => {
                print_jitter("Wake-up error, 1 ms period", &measure_sleep_jitter(2_000, Duration::from_millis(1)));
                print_jitter("Wake-up error, 7.3 ms period", &measure_sleep_jitter(300, Duration::from_micros(7_300)));
                Ok(())
            }
            Some("loopback") => loopback(),
            Some("gamepad") => gamepad_demo(flag(args, "--seconds").and_then(|s| s.parse().ok()).unwrap_or(5.0)),
            Some("checks") => {
                let p = chud_platform::native();
                println!("{:#?}", p.capabilities());
                for c in p.checks() {
                    println!("[{}] {} — {}", if c.ok { "ok" } else { "!!" }, c.title, c.detail);
                }
                for m in p.monitors() {
                    println!("monitor {:?} scale {} primary {}", m.rect, m.scale, m.primary);
                }
                Ok(())
            }
            _ => Err("usage: chud diag timer|loopback|checks|gamepad".into()),
        },
        Some("screen") => screen(args),
        _ => {
            println!("{}", include_str!("main.rs").lines().take(20).map(|l| l.trim_start_matches("//!")).collect::<Vec<_>>().join("\n"));
            Ok(())
        }
    }
}

fn region_flag(args: &[String]) -> Result<Option<chud_core::vision::Region>, String> {
    let Some(s) = flag(args, "--region") else { return Ok(None) };
    let n: Vec<i64> = s.split(',').map(|v| v.trim().parse::<i64>()).collect::<Result<_, _>>().map_err(|_| "bad --region (x,y,w,h)")?;
    match n[..] {
        [x, y, w, h] if w > 0 && h > 0 => Ok(Some(chud_core::vision::Region { x: x as i32, y: y as i32, w: w as u32, h: h as u32 })),
        _ => Err("bad --region (x,y,w,h)".into()),
    }
}

fn screen(args: &[String]) -> Result<(), String> {
    use chud_core::vision::{self, FindOptions, Image};
    let p = chud_platform::native();
    let region = region_flag(args)?;
    let t0 = Instant::now();
    let shot = p.capture(region).map_err(|e| e.to_string())?;
    let captured = t0.elapsed();
    match args.get(1).map(String::as_str) {
        Some("capture") => {
            let out = args.get(2).ok_or("missing output file")?;
            std::fs::write(out, shot.image.to_png()).map_err(|e| e.to_string())?;
            println!("{}×{} at ({}, {}) in {:.1} ms → {out}", shot.image.w, shot.image.h, shot.x, shot.y, captured.as_secs_f64() * 1e3);
        }
        Some("find") => {
            let bytes = std::fs::read(args.get(2).ok_or("missing image file")?).map_err(|e| e.to_string())?;
            let needle = Image::from_png(&bytes).map_err(|e| e.to_string())?;
            let all = args.iter().any(|a| a == "--all");
            let opts = FindOptions {
                threshold: flag(args, "--threshold").map(|t| t.parse().map_err(|_| "bad --threshold")).transpose()?.unwrap_or(0.9),
                any_scale: args.iter().any(|a| a == "--any-scale"),
                max_results: if all { 100 } else { 1 },
            };
            let t1 = Instant::now();
            let s = vision::find_image(&shot.image, &needle, &opts);
            println!("capture {:.1} ms, search {:.1} ms", captured.as_secs_f64() * 1e3, t1.elapsed().as_secs_f64() * 1e3);
            for m in &s.matches {
                let m = m.offset(shot.x, shot.y);
                println!("found at ({}, {}) {}×{} score {:.3} — centre {:?}", m.x, m.y, m.w, m.h, m.score, m.center());
            }
            if s.matches.is_empty() {
                match s.best {
                    Some(b) => println!("not found (closest {:.3} at ({}, {}))", b.score, b.x + shot.x, b.y + shot.y),
                    None => println!("not found"),
                }
            }
        }
        Some("text") => {
            let t1 = Instant::now();
            let words = p.read_text(&shot).map_err(|e| e.to_string())?;
            println!("capture {:.1} ms, recognition {:.1} ms, {} words", captured.as_secs_f64() * 1e3, t1.elapsed().as_secs_f64() * 1e3, words.len());
            let query: Vec<&str> = args[2..].iter().take_while(|a| !a.starts_with("--")).map(String::as_str).collect();
            if query.is_empty() {
                let mut line = None;
                for w in &words {
                    if line != Some(w.line) {
                        if line.is_some() {
                            println!();
                        }
                        print!("[{}, {}] ", w.x, w.y);
                        line = Some(w.line);
                    }
                    print!("{} ", w.text);
                }
                println!();
            } else {
                match vision::find_text(&words, &query.join(" "), false) {
                    Some(f) => println!("found at ({}, {}) {}×{} — centre {:?}", f.x, f.y, f.w, f.h, f.center()),
                    None => println!("not found"),
                }
            }
        }
        _ => return Err("usage: chud screen capture|find|text".into()),
    }
    Ok(())
}

fn print_jitter(title: &str, s: &JitterStats) {
    println!("{title}: n={} mean={:.1}µs median={:.1}µs p95={:.1}µs p99={:.1}µs max={:.1}µs", s.samples, s.mean_us, s.median_us, s.p95_us, s.p99_us, s.max_us);
}

fn info(path: &str) -> Result<(), String> {
    let doc = format::load(&PathBuf::from(path)).map_err(|e| e.to_string())?;
    let st = chud_core::stats::compute(&doc.timeline);
    println!("{} ({} events, {:.3} s)", doc.meta.name, st.events, st.duration_us as f64 / 1e6);
    println!("{}", serde_json::to_string_pretty(&st).unwrap());
    Ok(())
}

fn engine() -> Result<(Engine, crossbeam_channel::Receiver<EngineEvent>), String> {
    let platform: Arc<dyn Platform> = Arc::from(chud_platform::native());
    let (tx, rx) = crossbeam_channel::unbounded();
    let e = Engine::new(platform, Arc::new(NoMacros), tx).map_err(|e| e.to_string())?;
    Ok((e, rx))
}

fn play(path: &str, args: &[String]) -> Result<(), String> {
    let doc = format::load(&PathBuf::from(path)).map_err(|e| e.to_string())?;
    let mut settings = doc.settings.clone();
    if let Some(s) = flag(args, "--speed") {
        settings.speed = s.parse().map_err(|_| "bad --speed")?;
    }
    if let Some(r) = flag(args, "--repeat") {
        settings.repeat = r.parse().map_err(|_| "bad --repeat")?;
    }
    if let Some(m) = flag(args, "--mouse") {
        settings.mouse = match m {
            "absolute" => MouseMode::Absolute,
            "relative" => MouseMode::Relative,
            "off" => MouseMode::Off,
            _ => return Err("bad --mouse".into()),
        };
    }
    let (engine, rx) = engine()?;
    println!("Playing \"{}\" — Ctrl+Alt+Esc stops immediately.", doc.meta.name);
    engine.play(job_of(&doc, settings));
    for ev in rx {
        match ev {
            EngineEvent::PlaybackFinished { end, .. } => {
                println!("finished: {end:?}");
                break;
            }
            EngineEvent::Notice(n) => println!("[{}] {}", n.level, n.message),
            _ => {}
        }
    }
    engine.shutdown();
    Ok(())
}

fn job_of(doc: &MacroDoc, settings: PlaybackSettings) -> PlayJob {
    PlayJob::of_doc(doc, settings, Selection::All, true)
}

fn record(out: &str, args: &[String]) -> Result<(), String> {
    let seconds: Option<u64> = flag(args, "--seconds").map(|s| s.parse().map_err(|_| "bad --seconds")).transpose()?;
    let (engine, rx) = engine()?;
    let opts = RecordOptions { max_duration_ms: seconds.map(|s| s * 1_000), ..Default::default() };
    println!("Recording… press F9 to stop{}.", seconds.map(|s| format!(" (or wait {s} s)")).unwrap_or_default());
    engine.start_recording(opts);
    for ev in rx {
        if let EngineEvent::RecordingFinished { recording } = ev {
            let n = recording.items.len();
            let mut doc = MacroDoc::from_recording(
                PathBuf::from(out).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
                recording.items,
                recording.env,
            );
            doc.contexts = recording.contexts;
            doc.markers = recording.markers;
            format::save(&doc, &PathBuf::from(out)).map_err(|e| e.to_string())?;
            println!("Saved {n} events to {out} ({:?}).", recording.stopped_by);
            break;
        }
    }
    engine.shutdown();
    Ok(())
}

/// Injects known input through the real system path and captures it back
/// with Chud's own hooks.
fn loopback() -> Result<(), String> {
    let platform: Arc<dyn Platform> = Arc::from(chud_platform::native());
    let (tx, rx) = crossbeam_channel::unbounded();
    platform.start(tx).map_err(|e| e.to_string())?;
    platform.set_hotkeys(vec![]);
    platform.set_capture(CaptureOptions {
        forward: true,
        include_own: true,
        include_injected: true,
        exclude_self: false,
        mouse_raw: false,
        ..Default::default()
    });
    std::thread::sleep(Duration::from_millis(150));
    while rx.try_recv().is_ok() {}
    let mut inj = platform.injector().map_err(|e| e.to_string())?;
    let mut failures = 0;

    let wait_input = |pred: &dyn Fn(&EventKind) -> bool| -> Option<(Instant, EventKind)> {
        let deadline = Instant::now() + Duration::from_millis(300);
        while let Ok(c) = rx.recv_deadline(deadline) {
            if let Captured::Input { t, kind, .. } = c
                && pred(&kind) {
                    return Some((t, kind));
                }
        }
        None
    };

    // 1. Keyboard identity, scan-code and virtual-key paths.
    println!("== Keyboard (F13–F24, scan code and virtual key paths)");
    let mut latencies = Vec::new();
    for code in ["F13", "F14", "F15", "F16", "F17", "F18", "F19", "F20", "F21", "F22", "F23", "F24"] {
        let info = chud_core::keys::by_code(code).unwrap();
        let key = Key::new(info.scan, info.vk);
        for via in [KeyVia::Scan, KeyVia::Vk] {
            for down in [true, false] {
                let sent = Instant::now();
                inj.send(&[Action::Key { key, down, via }]).map_err(|e| e.to_string())?;
                let got = wait_input(&|k| matches!(k, EventKind::KeyDown { .. } | EventKind::KeyUp { .. }));
                match got {
                    Some((t, EventKind::KeyDown { key: k, .. })) | Some((t, EventKind::KeyUp { key: k })) => {
                        latencies.push(t.saturating_duration_since(sent).as_nanos() as f64 / 1e3);
                        if k.scan != key.scan || k.vk != key.vk {
                            failures += 1;
                            println!("  MISMATCH {code} {via:?}: sent {:#06X}/{:#04X} got {:#06X}/{:#04X}", key.scan, key.vk, k.scan, k.vk);
                        }
                    }
                    _ => {
                        failures += 1;
                        println!("  LOST {code} {via:?} down={down}");
                    }
                }
            }
        }
    }
    print_jitter("  inject → hook latency", &JitterStats::from_errors(latencies));

    // 2. Pointer accuracy: absolute on every monitor when the session allows
    //    reading positions (Windows, X11), otherwise exact relative motion.
    let caps = platform.capabilities();
    if !caps.absolute_pointer_capture {
        println!("== Relative pointer motion ({} session: no absolute positions)", caps.session);
        platform.set_capture(CaptureOptions {
            forward: true,
            include_own: true,
            include_injected: true,
            exclude_self: false,
            mouse_raw: true,
            ..Default::default()
        });
        std::thread::sleep(Duration::from_millis(100));
        while rx.try_recv().is_ok() {}
        let mut checked = 0;
        for (dx, dy) in [(1, 0), (0, 1), (-1, -1), (37, -12), (-250, 400), (3, 3)] {
            inj.send(&[Action::MoveBy { dx, dy }]).map_err(|e| e.to_string())?;
            let got = wait_input(&|k| matches!(k, EventKind::MouseRaw { .. }));
            checked += 1;
            if !matches!(got, Some((_, EventKind::MouseRaw { dx: gx, dy: gy })) if gx == dx && gy == dy) {
                failures += 1;
                println!("  WRONG delta ({dx},{dy}): got {:?}", got.map(|g| g.1));
            }
        }
        println!("  {checked} deltas checked");
    }
    if caps.absolute_pointer_capture {
    println!("== Pointer accuracy (all monitors)");
    let saved = platform.cursor();
    let mut checked = 0;
    for m in platform.monitors() {
        let r = m.rect;
        let pts = [
            (r.x, r.y),
            (r.x + r.width - 1, r.y),
            (r.x, r.y + r.height - 1),
            (r.x + r.width - 1, r.y + r.height - 1),
            (r.x + r.width / 2, r.y + r.height / 2),
            (r.x + r.width / 3 + 7, r.y + r.height / 5 + 3),
        ];
        for (x, y) in pts {
            inj.send(&[Action::MoveTo { x, y }]).map_err(|e| e.to_string())?;
            let got = wait_input(&|k| matches!(k, EventKind::MouseMove { .. }));
            let cur = platform.cursor();
            checked += 1;
            let hook_ok = matches!(got, Some((_, EventKind::MouseMove { x: gx, y: gy })) if gx == x && gy == y);
            if !hook_ok || cur != Some((x, y)) {
                failures += 1;
                println!("  OFF target ({x},{y}) on {}: hook {:?} cursor {:?}", m.name, got.map(|g| g.1), cur);
            }
        }
    }
    println!("  {checked} positions checked");
    if let Some((x, y)) = saved {
        let _ = platform.set_cursor(x, y);
    }
    }

    // 3. End-to-end timing through the real player.
    println!("== End-to-end timing (200 key events, 2.5 ms apart, real player thread)");
    let info = chud_core::keys::by_code("F24").unwrap();
    let key = Key::new(info.scan, info.vk);
    let items: Vec<_> = (0..200)
        .map(|i| (i as i64 * 2_500, if i % 2 == 0 { EventKind::KeyDown { key, repeat: false } } else { EventKind::KeyUp { key } }))
        .collect();
    let doc = MacroDoc::from_recording("loopback", items, Default::default());
    while rx.try_recv().is_ok() {}
    let shared = chud_engine::PlayerShared::default();
    let (ntx, _nrx) = crossbeam_channel::unbounded();
    let end = chud_engine::player::run_job(&job_of(&doc, PlaybackSettings::default()), platform.as_ref(), &NoMacros, &shared, &ntx);
    let mut stamps = Vec::new();
    let deadline = Instant::now() + Duration::from_millis(500);
    while let Ok(c) = rx.recv_deadline(deadline) {
        if let Captured::Input { t, kind: EventKind::KeyDown { .. } | EventKind::KeyUp { .. }, .. } = c {
            stamps.push(t);
        }
        if stamps.len() == 200 {
            break;
        }
    }
    println!("  player: {end:?}, captured {}/200", stamps.len());
    if stamps.len() != 200 {
        failures += 1;
    }
    if let Some(&first) = stamps.first() {
        // Signed error of each event relative to the first one; a constant
        // offset (first-event latency) is reported separately from jitter.
        let signed: Vec<f64> = stamps
            .iter()
            .enumerate()
            .skip(1)
            .map(|(i, t)| t.duration_since(first).as_nanos() as f64 / 1e3 - i as f64 * 2_500.0)
            .collect();
        let mut sorted = signed.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let offset = sorted[sorted.len() / 2];
        println!("  constant offset vs first event: {offset:.1}µs");
        let jitter: Vec<f64> = signed.iter().map(|e| (e - offset).abs()).collect();
        print_jitter("  jitter around the schedule", &JitterStats::from_errors(jitter));
    }
    // 4. Gamepad: a virtual controller is driven and read back through the
    //    same path as a physical one (XInput on Windows, evdev on Linux).
    if platform.capabilities().gamepad_output {
        failures += gamepad_loopback(inj.as_mut(), &rx)?;
    } else {
        println!("== Gamepad: skipped (no virtual gamepad driver)");
    }

    platform.set_capture(CaptureOptions::default());
    if failures == 0 {
        println!("RESULT: all checks passed");
        Ok(())
    } else {
        Err(format!("{failures} check(s) failed"))
    }
}

fn gamepad_loopback(inj: &mut dyn chud_platform::Injector, rx: &crossbeam_channel::Receiver<Captured>) -> Result<usize, String> {
    println!("== Gamepad (virtual controller → capture)");
    let t0 = Instant::now();
    inj.prepare_gamepads(1).map_err(|e| e.to_string())?;
    println!("  virtual controller ready in {} ms", t0.elapsed().as_millis());
    // The capture side looks for new controllers once per second.
    std::thread::sleep(Duration::from_millis(1_600));
    while rx.try_recv().is_ok() {}
    let wait_pad = |pred: &dyn Fn(&EventKind) -> bool| -> Option<(Instant, EventKind)> {
        let deadline = Instant::now() + Duration::from_millis(300);
        while let Ok(c) = rx.recv_deadline(deadline) {
            if let Captured::Input { t, kind, .. } = c
                && pred(&kind)
            {
                return Some((t, kind));
            }
        }
        None
    };
    let mut failures = 0;
    let mut latencies = Vec::new();
    let mut seen_pad: Option<u8> = None;
    let buttons = [
        PadButton::SOUTH, PadButton::EAST, PadButton::WEST, PadButton::NORTH, PadButton::BACK, PadButton::START,
        PadButton::LEFT_STICK, PadButton::RIGHT_STICK, PadButton::LEFT_SHOULDER, PadButton::RIGHT_SHOULDER,
        PadButton::DPAD_UP, PadButton::DPAD_DOWN, PadButton::DPAD_LEFT, PadButton::DPAD_RIGHT,
    ];
    for button in buttons {
        for down in [true, false] {
            let sent = Instant::now();
            inj.send(&[Action::PadButton { pad: 0, button, down }]).map_err(|e| e.to_string())?;
            match wait_pad(&|k| matches!(k, EventKind::PadButton { .. })) {
                Some((t, EventKind::PadButton { pad, button: b, down: d })) if b == button && d == down => {
                    latencies.push(t.saturating_duration_since(sent).as_nanos() as f64 / 1e3);
                    seen_pad.get_or_insert(pad);
                }
                other => {
                    failures += 1;
                    println!("  WRONG {} down={down}: got {:?}", button.name(), other.map(|o| o.1));
                }
            }
        }
    }
    println!("  {} buttons checked (pressed and released)", buttons.len());
    // Sticks are exact; triggers go through the controller's 8-bit range.
    let axes: [(PadAxis, i16, i32); 8] = [
        (PadAxis::LEFT_X, 12_345, 1),
        (PadAxis::LEFT_X, -32_768, 1),
        (PadAxis::LEFT_Y, -20_000, 1),
        (PadAxis::RIGHT_X, 32_767, 1),
        (PadAxis::RIGHT_Y, 32_767, 1),
        (PadAxis::LEFT_TRIGGER, 32_767, 1),
        (PadAxis::RIGHT_TRIGGER, 16_384, 130),
        (PadAxis::LEFT_TRIGGER, 0, 1),
    ];
    for (axis, value, tolerance) in axes {
        let sent = Instant::now();
        inj.send(&[Action::PadAxis { pad: 0, axis, value }]).map_err(|e| e.to_string())?;
        match wait_pad(&|k| matches!(k, EventKind::PadAxis { axis: a, .. } if *a == axis)) {
            Some((t, EventKind::PadAxis { value: v, .. })) if (v as i32 - value as i32).abs() < tolerance => {
                latencies.push(t.saturating_duration_since(sent).as_nanos() as f64 / 1e3);
            }
            other => {
                failures += 1;
                println!("  WRONG axis {} = {value}: got {:?}", axis.0, other.map(|o| o.1));
            }
        }
    }
    println!("  {} axis values checked", axes.len());
    // Back to rest, as the player does when it stops.
    let rest: Vec<Action> = (0..6).map(|a| Action::PadAxis { pad: 0, axis: PadAxis(a), value: 0 }).collect();
    inj.send(&rest).map_err(|e| e.to_string())?;
    if let Some(p) = seen_pad {
        println!("  captured as controller #{p}");
    }
    print_jitter("  inject → capture latency", &JitterStats::from_errors(latencies));
    Ok(failures)
}

/// Plugs in virtual controller 0 and plays a recognisable pattern: A, B, X,
/// Y in turn, the left stick circling, the right trigger breathing.
fn gamepad_demo(seconds: f64) -> Result<(), String> {
    let platform = chud_platform::native();
    let mut inj = platform.injector().map_err(|e| e.to_string())?;
    inj.prepare_gamepads(1).map_err(|e| e.to_string())?;
    println!("Virtual controller plugged in; moving it for {seconds:.1} s…");
    let start = Instant::now();
    let faces = [PadButton::SOUTH, PadButton::EAST, PadButton::WEST, PadButton::NORTH];
    let mut held: Option<PadButton> = None;
    while start.elapsed().as_secs_f64() < seconds {
        let e = start.elapsed().as_secs_f64();
        let face = faces[(e * 2.0) as usize % faces.len()];
        let mut batch = vec![
            Action::PadAxis { pad: 0, axis: PadAxis::LEFT_X, value: (e.cos() * 30_000.0) as i16 },
            Action::PadAxis { pad: 0, axis: PadAxis::LEFT_Y, value: (e.sin() * 30_000.0) as i16 },
            Action::PadAxis { pad: 0, axis: PadAxis::RIGHT_TRIGGER, value: (((e * 1.5).sin() + 1.0) * 16_000.0) as i16 },
        ];
        if held != Some(face) {
            if let Some(h) = held {
                batch.push(Action::PadButton { pad: 0, button: h, down: false });
            }
            batch.push(Action::PadButton { pad: 0, button: face, down: true });
            held = Some(face);
        }
        inj.send(&batch).map_err(|e| e.to_string())?;
        std::thread::sleep(Duration::from_millis(16));
    }
    let mut rest: Vec<Action> = (0..6).map(|a| Action::PadAxis { pad: 0, axis: PadAxis(a), value: 0 }).collect();
    if let Some(h) = held {
        rest.push(Action::PadButton { pad: 0, button: h, down: false });
    }
    inj.send(&rest).map_err(|e| e.to_string())?;
    println!("Done.");
    Ok(())
}
