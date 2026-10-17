// The real kin-native-host.exe (the dev build of it: the dev pipe, overridden to a pipe of this test) with real stdio pipes, the way a browser
// starts it: framed requests on stdin, framed answers on stdout, and nothing else on stdout.
mod common;

use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use common::*;
use kin_native_host::framing::{self, MAX_RESPONSE};
use kin_native_host::mock::server::Server;
use serde_json::{Value, json};

fn spawn(pipe: &str, args: &[&str]) -> Child {
    // `cargo test` enables the dev feature (a dev-dependency of this package on itself); a build without it would talk to the real pipe.
    const _: () = assert!(cfg!(feature = "dev"));
    Command::new(env!("CARGO_BIN_EXE_kin-native-host"))
        .args(args)
        .env("KIN_HOST_DEV_PIPE", pipe)
        .env("KIN_HOST_DEV_ANY_SERVER", "1") // the server is this test process
        .env_remove("KIN_HOST_DEV_KIN_EXE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the host starts")
}

fn read_responses(mut child: Child) -> (std::process::ExitStatus, Vec<Value>, Vec<u8>, String) {
    let out = read_all(child.stdout.take().unwrap());
    let mut err = String::new();
    child.stderr.take().unwrap().read_to_string(&mut err).unwrap();
    let status = child.wait().unwrap();
    let mut responses = Vec::new();
    let mut r = &out[..];
    // Every byte of stdout must be part of a frame: read_frame errors on a partial one, and we check nothing is left over.
    while let Some(b) = framing::read_frame(&mut r, MAX_RESPONSE, 0).expect("stdout holds only whole frames") {
        responses.push(serde_json::from_slice(&b).unwrap());
    }
    (status, responses, out, err)
}

#[test]
fn one_request_one_answer_then_exit_when_stdin_closes() {
    let pipe = unique_pipe();
    let (h, seen) = recording(|v| respond(json!({ "v": 1, "ref": v["id"], "type": "status", "ok": true, "running": true, "devices": [] })));
    let _srv = Server::start(&pipe, h).unwrap();
    let mut child = spawn(&pipe, &["chrome-extension://cbioefpddjpaionhljhffgbedncfchmn/", "--parent-window=0"]);
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(&frame(&req("status"))).unwrap();
    drop(stdin); // the browser closing the pipe
    let t = Instant::now();
    let (status, responses, _, _) = read_responses(child);
    assert!(t.elapsed() < Duration::from_secs(5), "exits promptly");
    assert!(status.success(), "{status:?}");
    assert_eq!(responses.len(), 1);
    assert_eq!((responses[0]["ref"].as_str(), responses[0]["running"].clone()), (Some("id-status"), json!(true)));
    let got: Value = serde_json::from_slice(&seen.lock().unwrap()[0]).unwrap();
    assert_eq!(got["origin"], "chrome-extension://cbioefpddjpaionhljhffgbedncfchmn/", "the browser's first argument reaches Kin as origin");
}

#[test]
fn firefox_arguments_give_the_extension_id_as_origin() {
    let pipe = unique_pipe();
    let (h, seen) = recording(ok_for);
    let _srv = Server::start(&pipe, h).unwrap();
    let mut child = spawn(&pipe, &[r"C:\Program Files\Kin\native-messaging\app.kinrelay.desktop.firefox.json", "kin@kinrelay.app"]);
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(&frame(&req("status"))).unwrap();
    drop(stdin);
    let (status, responses, ..) = read_responses(child);
    assert!(status.success());
    assert_eq!(responses[0]["ok"], true);
    assert_eq!(serde_json::from_slice::<Value>(&seen.lock().unwrap()[0]).unwrap()["origin"], "kin@kinrelay.app");
}

#[test]
fn a_request_dribbled_in_byte_by_byte_is_assembled() {
    let pipe = unique_pipe();
    let (h, _) = recording(ok_for);
    let _srv = Server::start(&pipe, h).unwrap();
    let mut child = spawn(&pipe, &[]);
    let mut stdin = child.stdin.take().unwrap();
    let bytes = frame(&json!({ "v": 1, "id": "slow-writer", "type": "send", "kind": "text", "text": "hello world" }));
    for chunk in bytes.chunks(3) {
        stdin.write_all(chunk).unwrap();
        stdin.flush().unwrap();
        std::thread::sleep(Duration::from_millis(8));
    }
    drop(stdin);
    let (status, responses, ..) = read_responses(child);
    assert!(status.success());
    assert_eq!((responses[0]["ref"].as_str(), responses[0]["ok"].clone()), (Some("slow-writer"), json!(true)));
}

#[test]
fn a_long_lived_session_answers_each_request_as_it_comes() {
    let pipe = unique_pipe();
    let (h, _) = recording(ok_for);
    let _srv = Server::start(&pipe, h).unwrap();
    let mut child = spawn(&pipe, &[]);
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    for i in 0..3 {
        stdin.write_all(&frame(&json!({ "v": 1, "id": format!("n{i}"), "type": "status" }))).unwrap();
        stdin.flush().unwrap();
        let b = framing::read_frame(&mut stdout, MAX_RESPONSE, 0).unwrap().expect("an answer before the next request");
        assert_eq!(serde_json::from_slice::<Value>(&b).unwrap()["ref"], format!("n{i}"));
    }
    drop(stdin);
    assert!(child.wait().unwrap().success());
}

#[test]
fn bad_json_gets_bad_request_and_the_host_goes_on_then_exits_cleanly() {
    let pipe = unique_pipe();
    let (h, _) = recording(ok_for);
    let _srv = Server::start(&pipe, h).unwrap();
    let mut child = spawn(&pipe, &[]);
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(&frame_bytes(b"}{")).unwrap();
    stdin.write_all(&frame(&req("status"))).unwrap();
    drop(stdin);
    let (status, responses, ..) = read_responses(child);
    assert!(status.success());
    assert_eq!(responses[0]["code"], "BAD_REQUEST");
    assert_eq!(responses[1]["ok"], true);
}

#[test]
fn an_oversize_prefix_gets_too_large_and_the_host_exits_without_reading_on() {
    let pipe = unique_pipe();
    let mut child = spawn(&pipe, &[]);
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(&(64u32 * 1024 * 1024 + 1).to_le_bytes()).unwrap();
    // The browser keeps stdin open (no drop): the host must answer and exit by itself.
    let t = Instant::now();
    let out = {
        let mut stdout = child.stdout.take().unwrap();
        framing::read_frame(&mut stdout, MAX_RESPONSE, 0).unwrap().expect("a TOO_LARGE answer")
    };
    assert_eq!(serde_json::from_slice::<Value>(&out).unwrap()["code"], "TOO_LARGE");
    let status = child.wait().unwrap();
    assert!(t.elapsed() < Duration::from_secs(5));
    assert_eq!(status.code(), Some(3));
    drop(stdin);
}

#[test]
fn stdin_closing_mid_frame_exits_quietly_with_nothing_on_stdout() {
    let pipe = unique_pipe();
    let mut child = spawn(&pipe, &[]);
    let mut stdin = child.stdin.take().unwrap();
    let mut bytes = frame(&req("status"));
    bytes.truncate(bytes.len() - 5);
    stdin.write_all(&bytes).unwrap();
    drop(stdin);
    let (status, responses, out, _) = read_responses(child);
    assert_eq!(status.code(), Some(2));
    assert!(responses.is_empty() && out.is_empty());
}

#[test]
fn nothing_but_frames_on_stdout_even_when_kin_is_not_running() {
    let pipe = unique_pipe(); // no server
    let mut child = spawn(&pipe, &[]);
    let mut stdin = child.stdin.take().unwrap();
    for ty in ["status", "open", "send", "frobnicate"] {
        stdin.write_all(&frame(&json!({ "v": 1, "id": ty, "type": ty, "kind": "text", "text": "x" }))).unwrap();
    }
    drop(stdin);
    let (status, responses, ..) = read_responses(child);
    assert!(status.success());
    assert_eq!(
        responses.iter().map(|r| (r["type"].as_str().unwrap().to_string(), r["code"].as_str().unwrap_or("").to_string())).collect::<Vec<_>>(),
        [("status".to_string(), "".to_string()), ("open".into(), "KIN_NOT_RUNNING".into()), ("send".into(), "KIN_NOT_RUNNING".into()), ("frobnicate".into(), "UNSUPPORTED_TYPE".into())]
    );
    assert_eq!(responses[0]["running"], false);
}

#[test]
fn the_host_is_a_windows_gui_subsystem_program_so_no_console_can_flash() {
    // PE optional header, Subsystem field: 2 = WINDOWS_GUI, 3 = WINDOWS_CUI.
    let exe = std::fs::read(env!("CARGO_BIN_EXE_kin-native-host")).unwrap();
    let pe = u32::from_le_bytes(exe[0x3c..0x40].try_into().unwrap()) as usize;
    assert_eq!(&exe[pe..pe + 4], b"PE\0\0");
    let subsystem = u16::from_le_bytes(exe[pe + 24 + 68..pe + 24 + 70].try_into().unwrap());
    assert_eq!(subsystem, 2, "IMAGE_SUBSYSTEM_WINDOWS_GUI");
}
