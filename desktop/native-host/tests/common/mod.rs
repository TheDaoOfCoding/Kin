// Shared helpers of the integration tests. Every test uses a pipe name of its own (`\\.\pipe\kin-host-test-<pid>-<n>`): nothing here can reach
// the user's Kin, and tests can run in parallel.
#![allow(dead_code)]

use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kin_native_host::config::Config;
use kin_native_host::framing;
use kin_native_host::host::{self, Ended};
use kin_native_host::mock::server::{Handler, Reply};
use serde_json::{Value, json};

static N: AtomicU32 = AtomicU32::new(0);

pub fn unique_pipe() -> String {
    format!(r"\\.\pipe\kin-host-test-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst))
}

/// A host config for a test pipe: no Kin to start, no check of the server's program, short waits.
pub fn config(pipe: &str) -> Config {
    let mut c = Config::release();
    c.pipe_name = Some(pipe.to_string());
    c.lock_name = pipe.rsplit('\\').next().unwrap().to_string();
    c.kin_exe = None;
    c.expected_server_image = None;
    c.start_wait = Duration::from_secs(5);
    c.status_busy_wait = Duration::from_millis(300);
    c.busy_wait = Duration::from_millis(500);
    c.status_timeout = Duration::from_secs(5);
    c.start_timeout = Duration::from_secs(8);
    c.send_timeout = Duration::from_secs(10);
    c
}

pub fn req(ty: &str) -> Value {
    json!({ "v": 1, "id": format!("id-{ty}"), "type": ty })
}

pub fn frame_bytes(body: &[u8]) -> Vec<u8> {
    let mut v = (body.len() as u32).to_le_bytes().to_vec();
    v.extend_from_slice(body);
    v
}

pub fn frame(v: &Value) -> Vec<u8> {
    frame_bytes(v.to_string().as_bytes())
}

/// Runs the host loop over these input bytes (in memory), returning how it ended and every response.
pub fn run_host(cfg: &Config, origin: &str, input: Vec<u8>) -> (Ended, Vec<Value>) {
    let mut out = Vec::new();
    let ended = host::run(cfg, origin, &mut &input[..], &mut out);
    let mut responses = Vec::new();
    let mut r = &out[..];
    while let Some(b) = framing::read_frame(&mut r, framing::MAX_RESPONSE, 0).expect("the host wrote a bad frame") {
        responses.push(serde_json::from_slice(&b).expect("the host wrote bad JSON"));
    }
    (ended, responses)
}

/// One request, one response.
pub fn ask(cfg: &Config, request: &Value) -> Value {
    let (ended, mut r) = run_host(cfg, "chrome-extension://testid/", frame(request));
    assert_eq!(ended, Ended::InputClosed);
    assert_eq!(r.len(), 1, "exactly one response per request");
    r.remove(0)
}

/// A pipe handler that records every request it gets (raw bytes) and answers with `answer(request)`.
pub fn recording(answer: impl Fn(&Value) -> Reply + Send + Sync + 'static) -> (Handler, Arc<Mutex<Vec<Vec<u8>>>>) {
    let seen: Arc<Mutex<Vec<Vec<u8>>>> = Arc::default();
    let s2 = seen.clone();
    let h: Handler = Arc::new(move |body: Vec<u8>| {
        s2.lock().unwrap().push(body.clone());
        match serde_json::from_slice::<Value>(&body) {
            Ok(v) => answer(&v),
            Err(_) => Reply::Close,
        }
    });
    (h, seen)
}

pub fn respond(v: Value) -> Reply {
    Reply::Respond(v.to_string().into_bytes())
}

/// The standard good answer: echo the ref and type.
pub fn ok_for(v: &Value) -> Reply {
    let mut m = json!({ "v": 1, "ref": v["id"], "type": v["type"], "ok": true });
    if v["type"] == "status" || v["type"] == "start" {
        m["running"] = json!(true);
        m["devices"] = json!([]);
    }
    respond(m)
}

pub fn mock_kin_exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_mock-kin"))
}

/// A fresh state folder for a mock-kin (scenario.json, log.jsonl, kin.pid, starts.log).
pub struct State {
    pub dir: PathBuf,
}

impl State {
    pub fn new() -> State {
        let dir = std::env::temp_dir().join(format!("kin-host-test-state-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        std::fs::create_dir_all(&dir).unwrap();
        State { dir }
    }
    pub fn scenario(&self, v: Value) {
        std::fs::write(self.dir.join("scenario.json"), v.to_string()).unwrap();
    }
    pub fn starts(&self) -> usize {
        std::fs::read_to_string(self.dir.join("starts.log")).map(|s| s.lines().count()).unwrap_or(0)
    }
    pub fn pid(&self) -> Option<u32> {
        std::fs::read_to_string(self.dir.join("kin.pid")).ok().and_then(|s| s.trim().parse().ok())
    }
    pub fn log(&self) -> Vec<Value> {
        std::fs::read_to_string(self.dir.join("log.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }
}

impl Drop for State {
    fn drop(&mut self) {
        // Only the mock-kin this test's host started (its pid is in our own state folder).
        if let Some(pid) = self.pid() {
            let _ = std::process::Command::new("taskkill").args(["/PID", &pid.to_string(), "/F"]).output();
        }
        std::thread::sleep(Duration::from_millis(100));
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A config that starts the mock-kin (serving `pipe`, with `state`) when needed, and checks that the pipe's server is that program.
pub fn config_with_kin(pipe: &str, state: &State) -> Config {
    let mut c = config(pipe);
    c.kin_exe = Some(mock_kin_exe());
    c.kin_args = vec!["--pipe".into(), pipe.to_string(), "--state".into(), state.dir.to_string_lossy().to_string()];
    c.expected_server_image = Some(mock_kin_exe());
    c
}

pub fn read_all<R: Read>(mut r: R) -> Vec<u8> {
    let mut v = Vec::new();
    let _ = r.read_to_end(&mut v);
    v
}

pub fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    for _ in 0..100 {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("timed out waiting for: {what}");
}
