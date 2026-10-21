//! A **silent media session of its own** for testing Kin's media controls (design 6.2), and a read-only reader that looks at **one named session only**. It never plays
//! audio, never uses a `MediaPlayer`, never shows `Playing`, never lists or opens any session but the one it is given, and never touches the system volume.
//!
//! ```text
//! cargo run --example media_test_session -- serve [--log <file>]      a session of its own (title: a random nonce), commands on stdin
//! cargo run --example media_test_session -- read <aumid>              what the session with exactly that AUMID shows (and nothing else of the PC's)
//! cargo run --example media_test_session -- control <aumid> <toggle|play|pause|next|prev|seek <ms>>   a command to exactly that session
//! cargo run --example media_test_session -- volume-read                the PC's master volume, READ ONLY (no Set* call exists in this file)
//! ```
//!
//! **serve.** Sets the process AUMID `app.kinrelay.dev.mediatest-<pid>`, makes a hidden top-level window and registers `SystemMediaTransportControls` on it (the same
//! code as Kin's flyout session, `kin_lib::media::surface::WinSurface`), with a random nonce as the title and the status `Paused` (never `Playing`), a generated
//! thumbnail and a timeline. It prints `READY aumid=<id> title=<nonce>` and then reads commands from stdin, one per line, and answers `OK` or `ERROR <why>`:
//! `title <text>`, `artist <text>`, `album <text>`, `state paused|stopped|changing`, `pos <ms>`, `duration <ms>`, `art`, `noart`,
//! `actions play,pause,next,prev,seek` (any of them; `toggle` enables play and pause) and `exit`. Every press of a flyout button and every drag of its bar that reaches
//! the session is printed (`EVENT button play`, `EVENT seek 5000`) and, with `--log <file>`, appended to that file. At the end (`exit`, or stdin closing) the session is
//! disabled and the process leaves: the session is gone from Windows' list.
//!
//! **read / control.** Only `app.kinrelay.dev.mediatest-<n>` (a test session of its own) or `app.kinrelay.desktop.dev-<key>` (a dev Kin's own flyout session, from its
//! `status.json`) is accepted, each with a non-empty suffix: **not** `app.kinrelay.desktop` (the installed Kin's, which shows the user's real phone) and no other app's
//! session. `read` prints one line, `SESSION status=<n>
//! title=<..> artist=<..> album=<..> positionMs=<n> durationMs=<n> controls=<flags> thumbnailBytes=<n>`; `control` asks that one session to do it and prints
//! `RESULT true|false`.
use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex};

use kin_lib::media::guard::Mode;
use kin_lib::media::map::{SmtcStatus, SmtcView};
use kin_lib::media::reader::{Control, Guarded, WinBackend};
use kin_lib::media::surface::{Button, MediaSurface, SurfaceEvent, SurfaceView, Timeline, WinSurface};
use kin_lib::media::volume::{VolumeEndpoint, WinVolume};

fn say(line: &str) {
    println!("{line}");
    let _ = std::io::stdout().flush();
}

fn guarded_for(aumid: &str) -> Guarded {
    if !kin_lib::media::guard::tool_may_look_at(aumid) {
        eprintln!(
            "refused: this tool only looks at {}<n> (its own) and {}<key> (a dev Kin's), never the installed Kin's",
            kin_lib::media::guard::TEST_AUMID_PREFIX,
            kin_lib::media::guard::DEV_KIN_AUMID_PREFIX
        );
        std::process::exit(2);
    }
    let wanted = aumid.to_string();
    let allowed: Arc<dyn Fn(&str) -> bool + Send + Sync> = Arc::new(move |a| a == wanted);
    let backend = WinBackend::start(allowed, Arc::new(|| {})).unwrap_or_else(|e| {
        eprintln!("the media sessions are not available: {e}");
        std::process::exit(1);
    });
    // `Mode::Test` with exactly this AUMID: every call is compared with it first, so no other session is ever read.
    Guarded::new(Arc::new(backend), Mode::Test { aumid: Some(aumid.to_string()), real_volume: false }, None)
}

fn generated_png(shade: u8) -> Vec<u8> {
    kin_lib::mirror::icons::test_png(64, 64, [shade, 120, 200, 255])
}

struct State {
    title: String,
    artist: String,
    album: String,
    status: SmtcStatus,
    position_ms: u64,
    duration_ms: u64,
    art: Option<Vec<u8>>,
    play: bool,
    pause: bool,
    next: bool,
    prev: bool,
    seek: bool,
}

impl State {
    fn view(&self) -> SurfaceView {
        SurfaceView {
            buttons: SmtcView { status: self.status, play: self.play, pause: self.pause, next: self.next, prev: self.prev },
            title: self.title.clone(),
            artist: self.artist.clone(),
            album_line: self.album.clone(),
            thumbnail: self.art.clone().map(Arc::new),
            timeline: (self.duration_ms > 0).then_some(Timeline {
                duration_ms: self.duration_ms,
                position_ms: self.position_ms,
                seek: self.seek,
            }),
            rate: 1.0,
        }
    }
}

fn serve(log: Option<String>) {
    let aumid = format!("app.kinrelay.dev.mediatest-{}", std::process::id());
    if let Err(e) = kin_lib::integration::set_process_app_user_model_id(&aumid) {
        eprintln!("the process AUMID could not be set: {e}");
        std::process::exit(1);
    }
    let log_file = Arc::new(Mutex::new(log));
    let events: kin_lib::media::surface::Events = {
        let log_file = log_file.clone();
        Arc::new(move |e| {
            let line = match e {
                SurfaceEvent::Button(Button::Play) => "button play".to_string(),
                SurfaceEvent::Button(Button::Pause) => "button pause".to_string(),
                SurfaceEvent::Button(Button::Next) => "button next".to_string(),
                SurfaceEvent::Button(Button::Prev) => "button prev".to_string(),
                SurfaceEvent::Seek(ms) => format!("seek {ms}"),
                SurfaceEvent::Locked(l) => format!("locked {l}"),
            };
            say(&format!("EVENT {line}"));
            if let Some(path) = log_file.lock().unwrap_or_else(|p| p.into_inner()).as_ref()
                && let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path)
            {
                let _ = writeln!(f, "{line}");
            }
        })
    };
    let surface = WinSurface::start(events).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1);
    });
    let nonce: String = kin_lib::encoding::random_bytes::<6>().iter().map(|b| format!("{b:02x}")).collect();
    let mut s = State {
        title: format!("mediatest-{nonce}"),
        artist: "Kin test artist".to_string(),
        album: "Kin test album".to_string(),
        status: SmtcStatus::Paused,
        position_ms: 42_000,
        duration_ms: 200_000,
        art: Some(generated_png(60)),
        play: true,
        pause: true,
        next: true,
        prev: true,
        seek: true,
    };
    if let Err(e) = surface.show(s.view()) {
        eprintln!("the session could not be shown: {e}");
        std::process::exit(1);
    }
    say(&format!("READY aumid={aumid} title={}", s.title));
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let (cmd, rest) = line.trim().split_once(' ').unwrap_or((line.trim(), ""));
        let mut changed = true;
        let answer = match cmd {
            "title" => {
                s.title = rest.to_string();
                Ok(())
            }
            "artist" => {
                s.artist = rest.to_string();
                Ok(())
            }
            "album" => {
                s.album = rest.to_string();
                Ok(())
            }
            // Never `Playing`: a Playing Windows session takes the keyboard's media keys.
            "state" => match rest {
                "paused" => {
                    s.status = SmtcStatus::Paused;
                    Ok(())
                }
                "stopped" => {
                    s.status = SmtcStatus::Stopped;
                    Ok(())
                }
                "changing" => {
                    s.status = SmtcStatus::Changing;
                    Ok(())
                }
                other => Err(format!("a state is paused, stopped or changing (never playing), not {other:?}")),
            },
            "pos" => rest.parse().map(|v| s.position_ms = v).map_err(|_| "pos needs milliseconds".to_string()),
            "duration" => rest.parse().map(|v| s.duration_ms = v).map_err(|_| "duration needs milliseconds".to_string()),
            "art" => {
                s.art = Some(generated_png(s.position_ms as u8));
                Ok(())
            }
            "noart" => {
                s.art = None;
                Ok(())
            }
            "actions" => {
                let names: Vec<&str> = rest.split(',').map(str::trim).filter(|n| !n.is_empty()).collect();
                let has = |n: &str| names.contains(&n);
                (s.play, s.pause, s.next, s.prev, s.seek) =
                    (has("play") || has("toggle"), has("pause") || has("toggle"), has("next"), has("prev"), has("seek"));
                Ok(())
            }
            "exit" => break,
            "" => {
                changed = false;
                Ok(())
            }
            other => Err(format!("no command {other:?}")),
        };
        match answer {
            Ok(()) => {
                if changed && let Err(e) = surface.show(s.view()) {
                    say(&format!("ERROR {e}"));
                    continue;
                }
                say("OK");
            }
            Err(e) => say(&format!("ERROR {e}")),
        }
    }
    surface.clear();
    surface.stop();
    say("BYE");
}

fn read(aumid: &str) {
    let g = guarded_for(aumid);
    match g.read(aumid) {
        None => say("NOSESSION"),
        Some(s) => {
            let thumbnail = g.thumbnail(aumid).map_or(0, |t| t.len());
            let c = s.controls;
            let flags: Vec<&str> =
                [(c.play, "play"), (c.pause, "pause"), (c.toggle, "toggle"), (c.next, "next"), (c.prev, "prev"), (c.position, "position")]
                    .into_iter()
                    .filter(|(on, _)| *on)
                    .map(|(_, n)| n)
                    .collect();
            say(&format!(
                "SESSION status={} title={} artist={} album={} positionMs={} durationMs={} controls={} thumbnailBytes={thumbnail}",
                s.status,
                s.title,
                s.artist,
                s.album,
                s.position_ms,
                s.duration_ms.unwrap_or(0),
                flags.join(",")
            ));
        }
    }
}

fn control(aumid: &str, what: &str, arg: Option<&str>) {
    let g = guarded_for(aumid);
    let c = match what {
        "toggle" => Control::Toggle,
        "play" => Control::Play,
        "pause" => Control::Pause,
        "next" => Control::Next,
        "prev" => Control::Prev,
        "seek" => Control::SeekTo(arg.and_then(|a| a.parse().ok()).expect("seek needs milliseconds")),
        other => {
            eprintln!("no command {other:?}");
            std::process::exit(2);
        }
    };
    match g.control(aumid, c) {
        Ok(done) => say(&format!("RESULT {done}")),
        Err(e) => say(&format!("ERROR {e}")),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("serve") => serve(args.iter().position(|a| a == "--log").and_then(|i| args.get(i + 1)).cloned()),
        Some("read") => read(args.get(1).expect("read needs an AUMID")),
        Some("control") => control(
            args.get(1).expect("control needs an AUMID"),
            args.get(2).expect("control needs a command"),
            args.get(3).map(String::as_str),
        ),
        Some("volume-read") => match WinVolume.get() {
            Ok(v) => say(&format!("VOLUME level={} muted={} steps={}", v.level, v.muted, WinVolume.steps())),
            Err(e) => say(&format!("ERROR {e}")),
        },
        _ => {
            eprintln!("usage: media_test_session serve [--log <file>] | read <aumid> | control <aumid> <cmd> | volume-read");
            std::process::exit(2);
        }
    }
}
