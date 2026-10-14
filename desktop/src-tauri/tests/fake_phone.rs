//! A fake phone against the real server in the test process: a rustls client with its own P-256 certificate that pairs with a
//! real token, opens the WebSocket, says `hello`, sends and receives `ping` and `ack`, is replaced by a second connection
//! (4001), is refused after being unpaired (401), and gets the close codes for bad frames.
//!
//! Milestone 1.2 adds the text and clipboard flows: a PC clipboard change reaches the phone as `clipboard` and is acked, a
//! `text` from the phone lands on the clipboard without bouncing back, password-manager content is not sent, the hotkey path
//! sends `text`, links open from the toast, and a send without an `ack` fails after 5 s.
//!
//! Everything listens on 127.0.0.1 with a random port. The clipboard is an in-memory one: **no test here touches the real
//! clipboard.**
mod common;

use std::time::{Duration, Instant};

use common::*;
use futures_util::{SinkExt, StreamExt};
use kin_lib::clipboard::{CAN_INCLUDE_IN_HISTORY, CAN_UPLOAD_TO_CLOUD, ClipFormat, EXCLUDE_FROM_MONITOR};
use kin_lib::core::{Core, Event, LinkState, PingReport};
use kin_lib::identity::Identity;
use kin_lib::sync::SendResult;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_rustls::client::TlsStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;

type Ws = WebSocketStream<TlsStream<tokio::net::TcpStream>>;

const PHONE_NAME: &str = "Test Phone";

struct World {
    env: TestEnv,
    server: kin_lib::server::ServerHandle,
    pc: TestKey,
    phone: TestKey,
}

impl World {
    async fn new() -> World {
        let pc_identity = Identity::generate().unwrap();
        let pc = TestKey {
            pkcs8: pc_identity.key_der.clone(),
            cert: pc_identity.cert_der.clone(),
            device_id: pc_identity.device_id.clone(),
            short_id: pc_identity.short_id.clone(),
        };
        let phone_identity = Identity::generate().unwrap();
        let phone = TestKey {
            pkcs8: phone_identity.key_der,
            cert: phone_identity.cert_der,
            device_id: phone_identity.device_id,
            short_id: phone_identity.short_id,
        };
        let env = test_core(pc_identity, "Desk PC");
        let server = start(&env.core).await;
        World { env, server, pc, phone }
    }

    fn core(&self) -> &Core {
        &self.env.core
    }

    async fn tls(&self) -> TlsStream<tokio::net::TcpStream> {
        connect(self.server.local_addr, phone_config(&self.pc.device_id, &self.phone)).await.unwrap()
    }

    /// Pairs the phone: starts a pairing like a developer would, and sends the token from its URL.
    async fn pair(&self) {
        let token = start_pairing_and_read_token(&self.env);
        let mut stream = self.tls().await;
        let response = request(
            &mut stream,
            "POST",
            "/v1/pair",
            &[("Content-Type", "application/json; charset=utf-8")],
            &pair_body(&token, PHONE_NAME),
        )
        .await
        .unwrap();
        assert_eq!(response.status, 200, "{:?}", response.json());
    }

    /// Opens the WebSocket. `Err` carries the HTTP status that refused the upgrade.
    async fn try_open_ws(&self) -> Result<Ws, u16> {
        let stream = self.tls().await;
        match tokio_tungstenite::client_async(format!("ws://127.0.0.1:{}/v1/ws", self.server.local_addr.port()), stream).await {
            Ok((ws, response)) => {
                assert_eq!(response.status().as_u16(), 101);
                Ok(ws)
            }
            Err(tokio_tungstenite::tungstenite::Error::Http(response)) => Err(response.status().as_u16()),
            Err(other) => panic!("unexpected WebSocket error: {other}"),
        }
    }

    /// Opens the WebSocket and does the `hello` exchange.
    async fn open_ws(&self) -> Ws {
        let mut ws = self.try_open_ws().await.expect("the upgrade works for a paired phone");
        let hello = next_json(&mut ws).await;
        assert_eq!(hello["type"], "hello");
        assert_eq!(hello["protocol"], 1);
        assert_eq!(hello["name"], "Desk PC");
        assert_eq!(hello["platform"], "windows");
        assert_eq!(hello["id"].as_str().unwrap().len(), 36, "a UUIDv7");
        assert!(hello["appVersion"].is_string());
        send_json(&mut ws, &phone_hello()).await;
        self.wait_for_state(LinkState::PhoneConnected).await;
        ws
    }

    async fn wait_for_state(&self, state: LinkState) {
        for _ in 0..100 {
            if self.core().snapshot().state == state {
                return;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        panic!("the state never became {state:?}, it is {:?}", self.core().snapshot().state);
    }
}

fn phone_hello() -> Value {
    json!({ "type": "hello", "id": "0193a5c2-7b1e-7c3a-9d4f-5e6a7b8c9d01", "protocol": 1, "name": PHONE_NAME, "platform": "android", "appVersion": "0.1.0" })
}

async fn send_json(ws: &mut Ws, v: &Value) {
    ws.send(Message::Text(v.to_string().into())).await.unwrap();
}

/// The next text frame as JSON. Ping and pong frames are skipped.
async fn next_json(ws: &mut Ws) -> Value {
    loop {
        match tokio::time::timeout(Duration::from_secs(5), ws.next()).await.expect("a frame within 5 s") {
            Some(Ok(Message::Text(t))) => return serde_json::from_str(t.as_str()).unwrap(),
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
            other => panic!("expected a text frame, got {other:?}"),
        }
    }
}

/// Reads until the close frame and returns its code. `None` if the connection ended without one.
async fn next_close(ws: &mut Ws, within: Duration) -> Option<u16> {
    loop {
        match tokio::time::timeout(within, ws.next()).await.expect("a close within the time") {
            Some(Ok(Message::Close(frame))) => return frame.map(|f: CloseFrame| u16::from(f.code)),
            Some(Ok(_)) => continue,
            _ => return None,
        }
    }
}

fn drain(events: &mut tokio::sync::mpsc::UnboundedReceiver<Event>) -> Vec<Event> {
    std::iter::from_fn(|| events.try_recv().ok()).collect()
}

fn toasts(events: &[Event]) -> Vec<String> {
    events.iter().filter_map(|e| if let Event::Toast(t) = e { Some(t.clone()) } else { None }).collect()
}

// ----- pairing ----------------------------------------------------------------------------------------------------

#[tokio::test]
async fn pairing_with_a_real_token() {
    let mut w = World::new().await;
    let port = w.server.local_addr.port();

    // Before pairing, the phone gets 401 on the WebSocket: a valid certificate, but not a paired device.
    assert_eq!(w.try_open_ws().await.err(), Some(401));
    assert_eq!(w.core().snapshot().state, LinkState::NotPaired);

    let token = start_pairing_and_read_token(&w.env);
    let url = std::fs::read_to_string(w.env.dir.path().join("pair-url.txt")).unwrap();
    assert_eq!(
        url.trim(),
        format!("kinrelay://pair?v=1&a=10.0.2.2:{port}&id={}&tk={token}&n=Desk%20PC", w.pc.device_id),
        "the dev QR address is used with the real port"
    );

    // A wrong token is bad_token and doesn't use the real one up.
    let mut stream = w.tls().await;
    let headers = [("Content-Type", "application/json")];
    let wrong = request(&mut stream, "POST", "/v1/pair", &headers, &pair_body("ZZZZZZZZZZZZZZZZZZZZZZ", PHONE_NAME)).await.unwrap();
    assert_eq!((wrong.status, wrong.json()["error"].clone()), (401, json!("bad_token")));

    let ok = request(&mut stream, "POST", "/v1/pair", &headers, &pair_body(&token, PHONE_NAME)).await.unwrap();
    assert_eq!(ok.status, 200);
    assert_eq!(ok.header("content-type"), Some("application/json; charset=utf-8"));
    assert_eq!(ok.json(), json!({ "name": "Desk PC", "deviceId": w.pc.device_id, "protocol": 1 }));

    // The token works once.
    let again = request(&mut stream, "POST", "/v1/pair", &headers, &pair_body(&token, PHONE_NAME)).await.unwrap();
    assert_eq!((again.status, again.json()["error"].clone()), (401, json!("token_used")));

    // The device was saved under the ID of its certificate.
    let device = w.core().db.get_device(&w.phone.device_id).unwrap().expect("the phone is saved");
    assert_eq!((device.name.as_str(), device.platform.as_str()), (PHONE_NAME, "android"));
    assert_eq!(w.core().snapshot().state, LinkState::PhoneOffline);
    let events = drain(&mut w.env.events);
    assert!(events.iter().any(|e| matches!(e, Event::DevicePaired(d) if d.device_id == w.phone.device_id)));

    // The pairing window shows "paired" with the device.
    let view = w.core().pairing_view().unwrap();
    assert_eq!(view.status, "paired");
    assert_eq!(view.device.unwrap().name, PHONE_NAME);

    // Now the WebSocket upgrade works.
    drop(w.open_ws().await);
}

#[tokio::test]
async fn closing_the_pairing_window_stops_the_token() {
    let w = World::new().await;
    let token = start_pairing_and_read_token(&w.env);
    w.core().end_pairing();
    let mut stream = w.tls().await;
    let r =
        request(&mut stream, "POST", "/v1/pair", &[("Content-Type", "application/json")], &pair_body(&token, PHONE_NAME)).await.unwrap();
    assert_eq!((r.status, r.json()["error"].clone()), (401, json!("token_expired")));
    assert!(w.core().db.list_devices().unwrap().is_empty());
}

#[tokio::test]
async fn five_wrong_guesses_kill_the_token_over_real_tls() {
    let w = World::new().await;
    let token = start_pairing_and_read_token(&w.env);
    let mut stream = w.tls().await;
    let headers = [("Content-Type", "application/json")];
    for i in 0..5 {
        let body = pair_body(&format!("ZZZZZZZZZZZZZZZZZZZZZ{i}"), PHONE_NAME);
        let r = request(&mut stream, "POST", "/v1/pair", &headers, &body).await.unwrap();
        assert_eq!(r.json()["error"], "bad_token");
    }
    let r = request(&mut stream, "POST", "/v1/pair", &headers, &pair_body(&token, PHONE_NAME)).await.unwrap();
    assert_eq!(r.json()["error"], "token_expired");
}

#[tokio::test]
async fn oversize_pair_body_gets_413_and_is_read_to_the_end() {
    let w = World::new().await;
    let mut stream = w.tls().await;
    let body = vec![b'a'; 1_048_577];
    let r = request(&mut stream, "POST", "/v1/pair", &[("Content-Type", "application/json")], &body).await.unwrap();
    assert_eq!((r.status, r.json()["error"].clone()), (413, json!("too_large")));
}

// ----- the WebSocket ----------------------------------------------------------------------------------------------

#[tokio::test]
async fn ping_in_both_directions_and_duplicates() {
    let mut w = World::new().await;
    w.pair().await;
    let mut ws = w.open_ws().await;
    drain(&mut w.env.events);

    // Phone -> PC: acked at once, and a toast "Ping from <name>".
    let ping = json!({ "type": "ping", "id": "ping-1", "sentAt": 1760443200000u64 });
    send_json(&mut ws, &ping).await;
    assert_eq!(next_json(&mut ws).await, json!({ "type": "ack", "ref": "ping-1" }));
    // A repeat gets the same ack and no second toast.
    send_json(&mut ws, &ping).await;
    assert_eq!(next_json(&mut ws).await, json!({ "type": "ack", "ref": "ping-1" }));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(toasts(&drain(&mut w.env.events)), ["Ping from Test Phone"]);

    // PC -> phone: the PC sends a ping and measures the round trip to the ack.
    let core = w.env.core.clone();
    let pinging = tokio::spawn(async move { core.ping_phone().await });
    let frame = next_json(&mut ws).await;
    assert_eq!(frame["type"], "ping");
    assert!(frame["sentAt"].as_u64().unwrap() > 1_700_000_000_000);
    tokio::time::sleep(Duration::from_millis(40)).await;
    send_json(&mut ws, &json!({ "type": "ack", "ref": frame["id"] })).await;
    match pinging.await.unwrap() {
        PingReport::Rtt(rtt) => assert!(rtt >= Duration::from_millis(40) && rtt < Duration::from_secs(2), "{rtt:?}"),
        other => panic!("expected a round-trip time, got {other:?}"),
    }
    let shown = toasts(&drain(&mut w.env.events));
    assert!(shown.len() == 1 && shown[0].starts_with("Ping: ") && shown[0].ends_with(" ms"), "{shown:?}");
}

#[tokio::test]
async fn ping_without_a_phone_says_so() {
    let mut w = World::new().await;
    assert_eq!(w.core().ping_phone().await, PingReport::NotConnected);
    assert_eq!(toasts(&drain(&mut w.env.events)), ["No phone connected"]);
}

#[tokio::test]
async fn ping_without_an_ack_is_no_reply_and_the_connection_closes() {
    let w = World::new().await;
    w.pair().await;
    let mut ws = w.open_ws().await;
    let core = w.env.core.clone();
    let pinging = tokio::spawn(async move { core.ping_phone().await });
    assert_eq!(next_json(&mut ws).await["type"], "ping");
    // No ack. After 5 seconds the PC says "no reply" and treats the connection as dead.
    assert_eq!(pinging.await.unwrap(), PingReport::NoReply);
    assert_eq!(next_close(&mut ws, Duration::from_secs(3)).await, Some(1000));
    w.wait_for_state(LinkState::PhoneOffline).await;
}

#[tokio::test]
async fn websocket_ping_frames_are_answered() {
    let w = World::new().await;
    w.pair().await;
    let mut ws = w.open_ws().await;
    ws.send(Message::Ping(vec![1, 2, 3].into())).await.unwrap();
    loop {
        match tokio::time::timeout(Duration::from_secs(5), ws.next()).await.unwrap() {
            Some(Ok(Message::Pong(p))) => {
                assert_eq!(p.as_ref(), [1, 2, 3]);
                break;
            }
            Some(Ok(_)) => continue,
            other => panic!("expected a pong, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn a_second_connection_replaces_the_first_with_4001() {
    let w = World::new().await;
    w.pair().await;
    let mut first = w.open_ws().await;
    let mut second = w.open_ws().await;
    assert_eq!(next_close(&mut first, Duration::from_secs(5)).await, Some(4001));
    // The new one is the one in use.
    send_json(&mut second, &json!({ "type": "ping", "id": "p", "sentAt": 1 })).await;
    assert_eq!(next_json(&mut second).await, json!({ "type": "ack", "ref": "p" }));
    assert_eq!(w.core().snapshot().state, LinkState::PhoneConnected);
    // The old connection ending must not take the new one's place with it.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(w.core().snapshot().state, LinkState::PhoneConnected);
}

#[tokio::test]
async fn unpair_from_the_phone_acks_closes_with_1000_and_then_401() {
    let w = World::new().await;
    w.pair().await;
    let mut ws = w.open_ws().await;
    send_json(&mut ws, &json!({ "type": "unpair", "id": "u1" })).await;
    assert_eq!(next_json(&mut ws).await, json!({ "type": "ack", "ref": "u1" }));
    assert_eq!(next_close(&mut ws, Duration::from_secs(5)).await, Some(1000));
    w.wait_for_state(LinkState::NotPaired).await;
    assert_eq!(w.try_open_ws().await.err(), Some(401));
}

#[tokio::test]
async fn removing_the_device_on_the_pc_sends_unpair_and_then_401() {
    let w = World::new().await;
    w.pair().await;
    let mut ws = w.open_ws().await;
    assert!(w.core().remove_device(&w.phone.device_id).unwrap());
    let unpair = next_json(&mut ws).await;
    assert_eq!(unpair["type"], "unpair");
    send_json(&mut ws, &json!({ "type": "ack", "ref": unpair["id"] })).await;
    assert_eq!(next_close(&mut ws, Duration::from_secs(5)).await, Some(1000));
    assert_eq!(w.try_open_ws().await.err(), Some(401));
    assert_eq!(w.core().snapshot().state, LinkState::NotPaired);
    // The pairing window that showed "New device paired" now shows "Device removed".
    assert_eq!(w.core().pairing_view().unwrap().status, "removed");
}

#[tokio::test]
async fn invalid_messages_get_errors_and_keep_the_connection_open() {
    let w = World::new().await;
    w.pair().await;
    let mut ws = w.open_ws().await;
    ws.send(Message::Text("not json".into())).await.unwrap();
    let e = next_json(&mut ws).await;
    assert_eq!((e["type"].clone(), e["code"].clone()), (json!("error"), json!("bad_request")));
    assert!(e.get("ref").is_none());
    send_json(&mut ws, &json!({ "type": "frobnicate", "id": "x1" })).await;
    let e = next_json(&mut ws).await;
    assert_eq!((e["code"].clone(), e["ref"].clone()), (json!("unsupported_type"), json!("x1")));
    send_json(&mut ws, &json!({ "type": "ping", "id": "x2", "sentAt": "late" })).await;
    let e = next_json(&mut ws).await;
    assert_eq!((e["code"].clone(), e["ref"].clone()), (json!("bad_request"), json!("x2")));
    // Still open.
    send_json(&mut ws, &json!({ "type": "ping", "id": "x3", "sentAt": 1 })).await;
    assert_eq!(next_json(&mut ws).await, json!({ "type": "ack", "ref": "x3" }));
}

#[tokio::test]
async fn a_binary_frame_closes_with_1003() {
    let w = World::new().await;
    w.pair().await;
    let mut ws = w.open_ws().await;
    ws.send(Message::Binary(vec![0, 1, 2, 3].into())).await.unwrap();
    assert_eq!(next_close(&mut ws, Duration::from_secs(5)).await, Some(1003));
}

#[tokio::test]
async fn a_frame_of_exactly_1_mib_is_accepted_and_one_byte_more_closes_with_1009() {
    let w = World::new().await;
    w.pair().await;
    let mut ws = w.open_ws().await;
    // Exactly 1,048,576 bytes: {"type":"ping","id":"big","sentAt":1,"pad":"aaa…"}
    let head = r#"{"type":"ping","id":"big","sentAt":1,"pad":""#;
    let tail = r#""}"#;
    let pad = |total: usize| "a".repeat(total - head.len() - tail.len());
    let exact = format!("{head}{}{tail}", pad(1_048_576));
    assert_eq!(exact.len(), 1_048_576);
    ws.send(Message::Text(exact.into())).await.unwrap();
    assert_eq!(next_json(&mut ws).await, json!({ "type": "ack", "ref": "big" }));
    let over = format!("{head}{}{tail}", pad(1_048_577));
    // The PC sets the library's limit to 1 MiB, so it stops reading at the frame header and closes with 1009 and no reply. A
    // client that is still writing the rest of the frame can then see the connection reset before it reads the close frame.
    // Either is fine for the phone (it never sends such a frame). What matters is that the PC dropped the connection.
    let _ = ws.send(Message::Text(over.into())).await;
    let closed = next_close(&mut ws, Duration::from_secs(5)).await;
    assert!(matches!(closed, Some(1009) | None), "{closed:?}");
    w.wait_for_state(LinkState::PhoneOffline).await;
}

/// A client that writes its own frames, to send what the WebSocket library won't: invalid UTF-8 in a text frame.
async fn raw_upgrade(w: &World) -> TlsStream<tokio::net::TcpStream> {
    let mut stream = w.tls().await;
    let headers = [
        ("Connection", "Upgrade"),
        ("Upgrade", "websocket"),
        ("Sec-WebSocket-Version", "13"),
        ("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ=="),
    ];
    let response = request(&mut stream, "GET", "/v1/ws", &headers, &[]).await.unwrap();
    assert_eq!(response.status, 101);
    stream
}

fn masked_text_frame(payload: &[u8]) -> Vec<u8> {
    let mask = [0x12, 0x34, 0x56, 0x78];
    let mut frame = vec![0x81, 0x80 | payload.len() as u8];
    frame.extend_from_slice(&mask);
    frame.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    frame
}

#[tokio::test]
async fn invalid_utf8_in_a_text_frame_closes_with_1007() {
    let w = World::new().await;
    w.pair().await;
    let mut stream = raw_upgrade(&w).await;
    // {"type":"text","id":"x","text":"<ff fe>","isUrl":false}
    let mut payload = br#"{"type":"text","id":"x","text":""#.to_vec();
    payload.extend_from_slice(&[0xff, 0xfe]);
    payload.extend_from_slice(br#"","isUrl":false}"#);
    stream.write_all(&masked_text_frame(&payload)).await.unwrap();
    // Read frames until the close frame (opcode 8). The PC's hello comes first.
    let mut buf = Vec::new();
    let code = loop {
        let mut chunk = [0u8; 1024];
        let n = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut chunk)).await.unwrap().unwrap();
        assert!(n > 0, "the connection ended without a close frame");
        buf.extend_from_slice(&chunk[..n]);
        // Walk the unmasked server frames.
        let mut pos = 0;
        let mut found = None;
        while pos + 2 <= buf.len() {
            let (opcode, mut len, mut hdr) = (buf[pos] & 0x0f, (buf[pos + 1] & 0x7f) as usize, 2);
            if len == 126 && pos + 4 <= buf.len() {
                len = u16::from_be_bytes([buf[pos + 2], buf[pos + 3]]) as usize;
                hdr = 4;
            }
            if pos + hdr + len > buf.len() {
                break;
            }
            if opcode == 8 {
                found = Some(u16::from_be_bytes([buf[pos + hdr], buf[pos + hdr + 1]]));
                break;
            }
            pos += hdr + len;
        }
        if let Some(c) = found {
            break c;
        }
    };
    assert_eq!(code, 1007);
}

#[tokio::test]
async fn no_hello_within_10_seconds_closes_with_1002() {
    let w = World::new().await;
    w.pair().await;
    let mut ws = w.try_open_ws().await.unwrap();
    assert_eq!(next_json(&mut ws).await["type"], "hello");
    // Say nothing. The PC closes with 1002 after 10 seconds, and sends no error message first.
    let started = std::time::Instant::now();
    assert_eq!(next_close(&mut ws, Duration::from_secs(13)).await, Some(1002));
    let waited = started.elapsed();
    assert!(waited >= Duration::from_secs(9) && waited < Duration::from_secs(12), "{waited:?}");
}

#[tokio::test]
async fn a_message_before_hello_is_an_error_and_closes_with_1002() {
    let w = World::new().await;
    w.pair().await;
    let mut ws = w.try_open_ws().await.unwrap();
    assert_eq!(next_json(&mut ws).await["type"], "hello");
    send_json(&mut ws, &json!({ "type": "ping", "id": "early", "sentAt": 1 })).await;
    let e = next_json(&mut ws).await;
    assert_eq!((e["code"].clone(), e["ref"].clone()), (json!("bad_request"), json!("early")));
    assert_eq!(next_close(&mut ws, Duration::from_secs(5)).await, Some(1002));
}

#[tokio::test]
async fn a_future_hello_gets_unsupported_version_and_4002() {
    let w = World::new().await;
    w.pair().await;
    let mut ws = w.try_open_ws().await.unwrap();
    assert_eq!(next_json(&mut ws).await["type"], "hello");
    send_json(&mut ws, &json!({ "type": "hello", "id": "h2", "protocol": 2, "device": { "name": "Future" } })).await;
    let e = next_json(&mut ws).await;
    assert_eq!((e["code"].clone(), e["ref"].clone()), (json!("unsupported_version"), json!("h2")));
    assert_eq!(next_close(&mut ws, Duration::from_secs(5)).await, Some(4002));
}

#[tokio::test]
async fn app_quit_closes_connections_with_1000() {
    let w = World::new().await;
    w.pair().await;
    let mut ws = w.open_ws().await;
    w.core().shutdown().await;
    assert_eq!(next_close(&mut ws, Duration::from_secs(5)).await, Some(1000));
}

// ----- 1.2: pairing window: a shut-down or expired token (carry-forward item (a)) -----------------------------------------

#[tokio::test]
async fn five_wrong_guesses_stop_the_qr_code_and_clear_pairing_active() {
    let mut w = World::new().await;
    let _token = start_pairing_and_read_token(&w.env);
    let view = w.core().pairing_view().unwrap();
    assert_eq!(view.status, "waiting");
    assert!(view.svg.is_some(), "the QR code is on screen");
    assert_eq!(w.core().status_json()["pairing"]["active"], json!(true));
    assert!(w.core().pairing_deadline().is_some());
    drain(&mut w.env.events);

    let mut stream = w.tls().await;
    for i in 0..5 {
        let body = pair_body(&format!("ZZZZZZZZZZZZZZZZZZZZZ{i}"), PHONE_NAME);
        let r = request(&mut stream, "POST", "/v1/pair", &[("Content-Type", "application/json")], &body).await.unwrap();
        assert_eq!(r.json()["error"], "bad_token");
    }

    // The window is told, and what it is told is that there is no code any more.
    let events = drain(&mut w.env.events);
    assert!(events.iter().any(|e| matches!(e, Event::PairingChanged)), "the pairing window is not told: {events:?}");
    let view = w.core().pairing_view().unwrap();
    assert_eq!(view.status, "locked");
    assert!(view.svg.is_none(), "the window must stop showing the QR code");
    let status = w.core().status_json();
    assert!(status["pairing"].is_null(), "pairing.active must leave status.json: {}", status["pairing"]);
    assert!(w.core().pairing_deadline().is_none());
}

#[tokio::test]
async fn a_pairing_code_that_runs_out_clears_pairing_active_and_the_window() {
    let w = World::new().await;
    w.core().start_pairing();
    assert_eq!(w.core().status_json()["pairing"]["active"], json!(true));
    // The same state the app is in 5 minutes later: the token is there but its time is up.
    w.core().seed_pairing_token("AAAAAAAAAAAAAAAAAAAAAA", kin_lib::pairing::SeedState::Expired);
    assert!(w.core().pairing_deadline().is_none(), "nothing left to wait for");
    assert!(w.core().status_json()["pairing"].is_null());
    let view = w.core().pairing_view().unwrap();
    assert_eq!((view.status, view.svg), ("expired", None));
}

// ----- 1.2: PC clipboard -> phone ---------------------------------------------------------------------------------------

/// Reads frames for `ms` milliseconds and fails if one is a message (WebSocket pings and pongs don't count).
async fn assert_silent(ws: &mut Ws, ms: u64) {
    let until = tokio::time::Instant::now() + Duration::from_millis(ms);
    loop {
        match tokio::time::timeout_at(until, ws.next()).await {
            Err(_) => return,
            Ok(Some(Ok(Message::Ping(_) | Message::Pong(_)))) => continue,
            Ok(other) => panic!("the PC sent something it should not have: {other:?}"),
        }
    }
}

async fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    for _ in 0..200 {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("timed out waiting for {what}");
}

impl World {
    /// Starts what the app starts: the task that turns clipboard change events into `clipboard` messages.
    fn start_clipboard_sync(&self) {
        let core = self.env.core.clone();
        let changes = self.env.clipboard.changes();
        tokio::spawn(async move { core.run_clipboard_sync(changes).await });
    }

    /// A paired, connected phone, with the clipboard sync running.
    async fn connected(&self) -> Ws {
        self.pair().await;
        let ws = self.open_ws().await;
        self.start_clipboard_sync();
        ws
    }
}

#[tokio::test]
async fn a_pc_clipboard_change_reaches_the_phone_as_a_clipboard_message_and_is_acked() {
    let w = World::new().await;
    let mut ws = w.connected().await;

    let copied = Instant::now();
    w.env.clipboard.user_copy("copied on the PC ✂ 日本", vec![]);
    let frame = next_json(&mut ws).await;
    let took = copied.elapsed();
    assert_eq!(frame["type"], "clipboard");
    assert_eq!(frame["text"], "copied on the PC ✂ 日本");
    assert_eq!(frame["id"].as_str().unwrap().len(), 36, "a UUIDv7");
    assert!(frame.get("isUrl").is_none(), "only `text` has isUrl");
    // The debounce: nothing goes out until 300 ms after the last change. And the 1 s target of targets.md has room to spare.
    assert!(took >= Duration::from_millis(280), "sent after only {took:?}: the debounce is 300 ms");
    assert!(took < Duration::from_millis(1000), "took {took:?}");

    tokio::time::sleep(Duration::from_millis(30)).await;
    send_json(&mut ws, &json!({ "type": "ack", "ref": frame["id"] })).await;
    let core = w.env.core.clone();
    wait_until("the ack to be recorded", || core.status_json()["lastSent"]["clipboard"]["result"] == "acked").await;

    // status.json: the id, the result and the arrival time (tap to ack), which includes the debounce.
    let sent = w.core().status_json()["lastSent"]["clipboard"].clone();
    assert_eq!(sent["id"], frame["id"]);
    assert_eq!(sent["trigger"], "clipboard");
    assert_eq!(sent["chars"], "copied on the PC ✂ 日本".chars().count());
    let (tap_to_ack, rtt) = (sent["tapToAckMs"].as_f64().unwrap(), sent["rttMs"].as_f64().unwrap());
    assert!((30.0..1000.0).contains(&rtt), "rtt {rtt}");
    assert!(tap_to_ack >= rtt + 250.0 && tap_to_ack < 1500.0, "tap to ack {tap_to_ack} vs rtt {rtt}");
    assert!(w.core().status_json()["lastSent"]["text"].is_null(), "no explicit send was made");
    // Nothing of the text is in status.json.
    assert!(!w.core().status_json().to_string().contains("copied on the PC"));
}

#[tokio::test]
async fn a_burst_of_clipboard_changes_is_sent_once_with_the_last_text() {
    let w = World::new().await;
    let mut ws = w.connected().await;
    for text in ["one", "two", "three"] {
        w.env.clipboard.user_copy(text, vec![]);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let frame = next_json(&mut ws).await;
    assert_eq!((frame["type"].clone(), frame["text"].clone()), (json!("clipboard"), json!("three")));
    send_json(&mut ws, &json!({ "type": "ack", "ref": frame["id"] })).await;
    assert_silent(&mut ws, 700).await;
}

#[tokio::test]
async fn clipboard_content_marked_by_a_password_manager_is_not_sent() {
    let w = World::new().await;
    let mut ws = w.connected().await;
    let marks = [
        (vec![ClipFormat::named(EXCLUDE_FROM_MONITOR)], "ExcludeClipboardContentFromMonitorProcessing"),
        (vec![ClipFormat::with_dword(CAN_INCLUDE_IN_HISTORY, 0)], "CanIncludeInClipboardHistory"),
        (vec![ClipFormat::with_dword(CAN_UPLOAD_TO_CLOUD, 0)], "CanUploadToCloudClipboard"),
    ];
    for (formats, name) in marks {
        w.env.clipboard.user_copy("hunter2 (a password)", formats);
        assert_silent(&mut ws, 600).await;
        let skip = w.core().status_json()["clipboardSync"]["lastSkip"]["reason"].clone();
        assert_eq!(skip, json!(format!("password_manager:{name}")));
    }
    assert_eq!(w.core().status_json()["clipboardSync"]["skipped"], 3);
    assert_eq!(w.core().status_json()["clipboardSync"]["sent"], 0);
    assert!(w.core().status_json()["lastSent"]["clipboard"].is_null());

    // A format that says "yes, you may keep this" doesn't stop anything, and an ordinary copy after a skipped one goes through.
    w.env.clipboard.user_copy("an ordinary copy", vec![ClipFormat::with_dword(CAN_INCLUDE_IN_HISTORY, 1)]);
    let frame = next_json(&mut ws).await;
    assert_eq!((frame["type"].clone(), frame["text"].clone()), (json!("clipboard"), json!("an ordinary copy")));
}

#[tokio::test]
async fn a_clipboard_without_text_is_not_sent() {
    let w = World::new().await;
    let mut ws = w.connected().await;
    w.env.clipboard.user_copy_non_text();
    assert_silent(&mut ws, 600).await;
    assert_eq!(w.core().status_json()["clipboardSync"]["lastSkip"]["reason"], "not_text");
}

#[tokio::test]
async fn automatic_clipboard_sync_can_be_turned_off_and_the_choice_is_kept() {
    let w = World::new().await;
    let mut ws = w.connected().await;
    assert!(w.core().auto_clipboard_sync(), "on by default");
    assert_eq!(w.core().status_json()["settings"]["autoClipboardSync"], true);

    w.core().set_auto_clipboard_sync(false);
    w.env.clipboard.user_copy("not for the phone", vec![]);
    assert_silent(&mut ws, 600).await;
    assert_eq!(w.core().status_json()["clipboardSync"]["enabled"], false);
    // It is written to settings.json in the data directory, and a new start reads it.
    let saved = kin_lib::settings::Settings::load(w.env.dir.path());
    assert!(!saved.auto_clipboard_sync);
    assert_eq!(saved.hotkey, kin_lib::settings::DEFAULT_HOTKEY);

    w.core().set_auto_clipboard_sync(true);
    w.env.clipboard.user_copy("now it is", vec![]);
    let frame = next_json(&mut ws).await;
    assert_eq!(frame["text"], "now it is");
}

#[tokio::test]
async fn a_text_over_1_mib_is_skipped_and_nothing_is_sent() {
    let w = World::new().await;
    let mut ws = w.connected().await;
    // 1 MiB of text is over the limit as a message, because the JSON around it counts.
    w.env.clipboard.user_copy(&"a".repeat(1_048_576), vec![]);
    assert_silent(&mut ws, 800).await;
    let skip = w.core().status_json()["clipboardSync"]["lastSkip"]["reason"].clone();
    assert_eq!(skip, "too_large");
    assert_eq!(w.core().status_json()["lastSent"]["clipboard"]["result"], "too_large");
    // And the connection is fine: a normal copy afterwards goes through.
    w.env.clipboard.user_copy("small again", vec![]);
    assert_eq!(next_json(&mut ws).await["text"], "small again");
}

#[tokio::test]
async fn a_clipboard_change_while_the_phone_is_offline_fails_quietly_and_says_so_in_status() {
    let mut w = World::new().await;
    w.pair().await;
    w.start_clipboard_sync();
    drain(&mut w.env.events);
    w.env.clipboard.user_copy("nobody is listening", vec![]);
    let core = w.env.core.clone();
    wait_until("the failed send to be recorded", || core.status_json()["lastSent"]["clipboard"]["result"] == "phone_offline").await;
    // No toast for an automatic sync: it would pop up on every copy while the phone sleeps.
    assert!(toasts(&drain(&mut w.env.events)).is_empty());
}

// ----- 1.2: phone -> PC text ----------------------------------------------------------------------------------------------

fn text_message(id: &str, text: &str, is_url: bool) -> Value {
    json!({ "type": "text", "id": id, "text": text, "isUrl": is_url })
}

fn text_toasts(events: &[Event]) -> Vec<(String, String, bool)> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::TextToast { from, preview, has_link, .. } => Some((from.clone(), preview.clone(), *has_link)),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn text_from_the_phone_goes_to_the_clipboard_with_a_toast_and_is_not_sent_back() {
    let mut w = World::new().await;
    let mut ws = w.connected().await;
    drain(&mut w.env.events);

    send_json(&mut ws, &text_message("t1", "hello from the phone", false)).await;
    assert_eq!(next_json(&mut ws).await, json!({ "type": "ack", "ref": "t1" }));
    let clipboard = w.env.clipboard.clone();
    wait_until("the clipboard to be set", || clipboard.text().as_deref() == Some("hello from the phone")).await;
    // The write caused a change event. Loop prevention: the PC says nothing back, not even after the debounce.
    assert_silent(&mut ws, 800).await;
    assert_eq!(w.env.clipboard.app_writes(), ["hello from the phone"]);
    assert_eq!(w.core().status_json()["clipboardSync"]["lastSkip"]["reason"], "echo_of_phone_text");
    assert_eq!(w.core().status_json()["clipboardSync"]["sent"], 0);

    // The toast: "Text from <phone>" with a preview, and no link.
    let events = drain(&mut w.env.events);
    assert_eq!(text_toasts(&events), [(PHONE_NAME.to_string(), "hello from the phone".to_string(), false)]);
    let received = w.core().status_json()["lastReceived"]["text"].clone();
    assert_eq!(
        (received["id"].clone(), received["clipboardSet"].clone(), received["isUrl"].clone()),
        (json!("t1"), json!(true), json!(false))
    );

    // A repeat of the id is acked again and not processed again: one write, one toast.
    send_json(&mut ws, &text_message("t1", "hello from the phone", false)).await;
    assert_eq!(next_json(&mut ws).await, json!({ "type": "ack", "ref": "t1" }));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(w.env.clipboard.app_writes().len(), 1);
    assert!(text_toasts(&drain(&mut w.env.events)).is_empty());

    // The user then copies something else on the PC: that is a real change and it goes to the phone.
    w.env.clipboard.user_copy("my own copy", vec![]);
    let frame = next_json(&mut ws).await;
    assert_eq!((frame["type"].clone(), frame["text"].clone()), (json!("clipboard"), json!("my own copy")));
}

#[tokio::test]
async fn the_same_text_copied_on_the_pc_after_the_phone_sent_it_is_a_new_copy() {
    let w = World::new().await;
    let mut ws = w.connected().await;
    send_json(&mut ws, &text_message("t1", "shared text", false)).await;
    next_json(&mut ws).await;
    assert_silent(&mut ws, 700).await;
    // Something else is copied on the PC (sent), and then the shared text again: the guard no longer remembers it, so it goes.
    w.env.clipboard.user_copy("other", vec![]);
    let frame = next_json(&mut ws).await;
    assert_eq!(frame["text"], "other");
    send_json(&mut ws, &json!({ "type": "ack", "ref": frame["id"] })).await;
    w.env.clipboard.user_copy("shared text", vec![]);
    assert_eq!(next_json(&mut ws).await["text"], "shared text");
}

#[tokio::test]
async fn new_lines_in_text_from_the_phone_become_windows_new_lines_on_the_clipboard() {
    let w = World::new().await;
    let mut ws = w.connected().await;
    send_json(&mut ws, &text_message("t1", "line one\nline two\r\nline three", false)).await;
    next_json(&mut ws).await;
    let clipboard = w.env.clipboard.clone();
    wait_until("the clipboard to be set", || clipboard.text().is_some()).await;
    assert_eq!(w.env.clipboard.text().as_deref(), Some("line one\r\nline two\r\nline three"));
    // And the echo of that is still recognised, though the text differs from the message's.
    assert_silent(&mut ws, 700).await;
}

#[tokio::test]
async fn text_with_unicode_survives_the_trip_and_the_toast_preview_is_one_short_line() {
    let mut w = World::new().await;
    let mut ws = w.connected().await;
    drain(&mut w.env.events);
    let long = format!("日本語 😀 {}\n{}", "word ".repeat(60), "tail");
    send_json(&mut ws, &text_message("t1", &long, false)).await;
    next_json(&mut ws).await;
    let events = {
        let core = w.env.core.clone();
        wait_until("the toast", || core.status_json()["lastReceived"]["text"]["id"] == "t1").await;
        drain(&mut w.env.events)
    };
    let toasts = text_toasts(&events);
    assert_eq!(toasts.len(), 1);
    let preview = &toasts[0].1;
    assert!(preview.starts_with("日本語 😀 word word"), "{preview}");
    assert!(preview.ends_with('…') && preview.chars().count() == 101 && !preview.contains('\n'), "{preview:?}");
}

#[tokio::test]
async fn a_link_toast_opens_the_link_when_clicked_and_only_for_http_and_https() {
    let mut w = World::new().await;
    let mut ws = w.connected().await;
    drain(&mut w.env.events);

    // A real link: the toast has a link, clicking it opens it once.
    send_json(&mut ws, &text_message("l1", "https://example.com/a?b=c#d", true)).await;
    next_json(&mut ws).await;
    let core = w.env.core.clone();
    wait_until("the toast", || core.status_json()["lastReceived"]["text"]["id"] == "l1").await;
    let events = drain(&mut w.env.events);
    let id = events
        .iter()
        .find_map(|e| match e {
            Event::TextToast { id, has_link: true, .. } => Some(*id),
            _ => None,
        })
        .expect("a toast with a link");
    assert!(w.opener_opened().is_empty(), "nothing opens before the click");
    assert!(w.core().toast_clicked(id));
    assert_eq!(w.opener_opened(), ["https://example.com/a?b=c#d"]);
    assert!(!w.core().toast_clicked(id), "a second click does nothing");
    assert_eq!(w.opener_opened().len(), 1);

    // Anything that is not an http(s) link has no link on its toast, even when the phone says isUrl, and a click opens nothing.
    for (i, text) in
        ["file:///C:/Windows/System32/calc.exe", "javascript:alert(1)", "ms-msdt:/id PCWDiagnostic", "https://example.com and more words"]
            .iter()
            .enumerate()
    {
        let id = format!("bad{i}");
        send_json(&mut ws, &text_message(&id, text, true)).await;
        next_json(&mut ws).await;
        let core = w.env.core.clone();
        let want = id.clone();
        wait_until("the toast", || core.status_json()["lastReceived"]["text"]["id"] == json!(want)).await;
        let received = w.core().status_json()["lastReceived"]["text"].clone();
        assert_eq!(received["openable"], false, "{text}");
        assert!(w.core().newest_link_toast().is_none(), "{text}");
    }
    let toasts = text_toasts(&drain(&mut w.env.events));
    assert_eq!(toasts.len(), 4);
    assert!(toasts.iter().all(|(_, _, has_link)| !has_link), "{toasts:?}");
    // And a real link with isUrl false is text, not a link.
    send_json(&mut ws, &text_message("plain", "https://example.com/not-flagged", false)).await;
    next_json(&mut ws).await;
    let core = w.env.core.clone();
    wait_until("the toast", || core.status_json()["lastReceived"]["text"]["id"] == "plain").await;
    assert!(w.core().newest_link_toast().is_none());
    assert_eq!(w.opener_opened().len(), 1, "only the first link was ever opened");
}

impl World {
    fn opener_opened(&self) -> Vec<String> {
        self.env.opener.opened()
    }
}

#[tokio::test]
async fn invalid_text_from_the_phone_is_rejected_and_leaves_the_clipboard_alone() {
    let w = World::new().await;
    let mut ws = w.connected().await;
    w.env.clipboard.user_copy("mine", vec![]);
    let frame = next_json(&mut ws).await;
    send_json(&mut ws, &json!({ "type": "ack", "ref": frame["id"] })).await;
    for bad in [json!({ "type": "text", "id": "b1", "text": "", "isUrl": false }), json!({ "type": "text", "id": "b2", "text": "x" })] {
        send_json(&mut ws, &bad).await;
        assert_eq!(next_json(&mut ws).await["code"], "bad_request");
    }
    // A `clipboard` message from the phone is the wrong direction.
    send_json(&mut ws, &json!({ "type": "clipboard", "id": "b3", "text": "x" })).await;
    assert_eq!(next_json(&mut ws).await["code"], "unsupported_type");
    assert_eq!(w.env.clipboard.text().as_deref(), Some("mine"));
    assert!(w.env.clipboard.app_writes().is_empty());
}

// ----- 1.2: the hotkey path ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_hotkey_sends_the_clipboard_text_as_a_text_message() {
    let mut w = World::new().await;
    let mut ws = w.connected().await;
    // Auto sync off, so the only message is the explicit one. (The hotkey works whether or not it is on.)
    w.core().set_auto_clipboard_sync(false);
    w.env.clipboard.user_copy("explicit send", vec![]);
    drain(&mut w.env.events);

    // `send_clipboard_text` is what the hotkey, the tray item and `--send-clipboard` all call.
    let core = w.env.core.clone();
    let sending = tokio::spawn(async move { core.send_clipboard_text("hotkey", kin_lib::sync::Tap::now()).await });
    let frame = next_json(&mut ws).await;
    assert_eq!(frame["type"], "text");
    assert_eq!(frame["text"], "explicit send");
    assert_eq!(frame["isUrl"], false);
    assert_eq!(frame["id"].as_str().unwrap().len(), 36);
    tokio::time::sleep(Duration::from_millis(25)).await;
    send_json(&mut ws, &json!({ "type": "ack", "ref": frame["id"] })).await;
    match sending.await.unwrap() {
        SendResult::Acked { rtt, tap_to_ack } => assert!(rtt >= Duration::from_millis(25) && tap_to_ack >= rtt, "{rtt:?} {tap_to_ack:?}"),
        other => panic!("{other:?}"),
    }
    let sent = w.core().status_json()["lastSent"]["text"].clone();
    assert_eq!(
        (sent["id"].clone(), sent["result"].clone(), sent["trigger"].clone()),
        (frame["id"].clone(), json!("acked"), json!("hotkey"))
    );
    assert!(sent["tapToAckMs"].as_f64().unwrap() > 0.0 && sent["rttMs"].as_f64().unwrap() > 0.0);
    assert_eq!(toasts(&drain(&mut w.env.events)), ["Sent to Test Phone"]);
}

#[tokio::test]
async fn the_hotkey_marks_a_link_as_a_link() {
    let w = World::new().await;
    let mut ws = w.connected().await;
    w.env.clipboard.user_copy("  https://example.com/path?x=1 \n", vec![]);
    let frame = next_json(&mut ws).await;
    assert_eq!(frame["type"], "clipboard", "the automatic sync has no isUrl");
    send_json(&mut ws, &json!({ "type": "ack", "ref": frame["id"] })).await;

    let core = w.env.core.clone();
    let sending = tokio::spawn(async move { core.send_clipboard_text("command", kin_lib::sync::Tap::now()).await });
    let frame = next_json(&mut ws).await;
    assert_eq!((frame["type"].clone(), frame["isUrl"].clone()), (json!("text"), json!(true)));
    assert_eq!(frame["text"], "  https://example.com/path?x=1 \n", "the text is sent as it is: only isUrl looks at the trimmed form");
    send_json(&mut ws, &json!({ "type": "ack", "ref": frame["id"] })).await;
    assert!(matches!(sending.await.unwrap(), SendResult::Acked { .. }));
}

#[tokio::test]
async fn the_hotkey_sends_even_what_a_password_manager_marked_because_the_user_asked() {
    let w = World::new().await;
    let mut ws = w.connected().await;
    w.env.clipboard.user_copy("pw-from-manager", vec![ClipFormat::named(EXCLUDE_FROM_MONITOR)]);
    assert_silent(&mut ws, 600).await; // the automatic sync leaves it alone
    let core = w.env.core.clone();
    let sending = tokio::spawn(async move { core.send_clipboard_text("hotkey", kin_lib::sync::Tap::now()).await });
    let frame = next_json(&mut ws).await;
    assert_eq!((frame["type"].clone(), frame["text"].clone()), (json!("text"), json!("pw-from-manager")));
    send_json(&mut ws, &json!({ "type": "ack", "ref": frame["id"] })).await;
    assert!(matches!(sending.await.unwrap(), SendResult::Acked { .. }));
}

#[tokio::test]
async fn the_hotkey_says_so_when_there_is_nothing_to_send_or_nobody_to_send_to() {
    let mut w = World::new().await;
    // No phone is paired.
    w.env.clipboard.user_copy("something", vec![]);
    assert_eq!(w.core().send_clipboard_text("hotkey", kin_lib::sync::Tap::now()).await, SendResult::NoPhonePaired);
    assert_eq!(toasts(&drain(&mut w.env.events)), ["No phone is paired yet."]);

    // Paired but not connected: the send just fails, with a notice (the queue is 1.4).
    w.pair().await;
    drain(&mut w.env.events);
    assert_eq!(w.core().send_clipboard_text("tray", kin_lib::sync::Tap::now()).await, SendResult::PhoneOffline);
    assert_eq!(toasts(&drain(&mut w.env.events)), ["The phone isn't connected. The text was not sent."]);
    assert_eq!(w.core().status_json()["lastSent"]["text"]["result"], "phone_offline");

    // Connected, but the clipboard holds no text.
    let _ws = w.open_ws().await;
    drain(&mut w.env.events);
    w.env.clipboard.user_copy_non_text();
    assert_eq!(w.core().send_clipboard_text("hotkey", kin_lib::sync::Tap::now()).await, SendResult::Rejected("no_text".into()));
    assert_eq!(toasts(&drain(&mut w.env.events)), ["The clipboard has no text to send."]);
}

// ----- 1.2: waiting for the ack -----------------------------------------------------------------------------------------

#[tokio::test]
async fn no_ack_in_5_seconds_marks_the_phone_offline_and_the_send_fails_visibly() {
    let mut w = World::new().await;
    let mut ws = w.connected().await;
    w.env.clipboard.user_copy("is anybody there", vec![]);
    let frame = next_json(&mut ws).await;
    assert_eq!(frame["type"], "clipboard");
    // The phone never answers. After 5 s the connection is closed as dead, so the PC shows the phone as offline.
    let started = Instant::now();
    assert_eq!(next_close(&mut ws, Duration::from_secs(7)).await, Some(1000));
    assert!(started.elapsed() >= Duration::from_secs(4) && started.elapsed() < Duration::from_secs(6), "{:?}", started.elapsed());
    w.wait_for_state(LinkState::PhoneOffline).await;
    let sent = w.core().status_json()["lastSent"]["clipboard"].clone();
    assert_eq!(sent["result"], "no_reply");
    assert!(sent["tapToAckMs"].is_null());

    // An explicit send after that fails at once with a notice: the phone is offline now.
    drain(&mut w.env.events);
    assert_eq!(w.core().send_clipboard_text("hotkey", kin_lib::sync::Tap::now()).await, SendResult::PhoneOffline);
    assert_eq!(toasts(&drain(&mut w.env.events)), ["The phone isn't connected. The text was not sent."]);
}

#[tokio::test]
async fn an_explicit_send_without_an_ack_shows_a_notice() {
    let mut w = World::new().await;
    let mut ws = w.connected().await;
    w.core().set_auto_clipboard_sync(false);
    w.env.clipboard.user_copy("explicit", vec![]);
    drain(&mut w.env.events);
    let core = w.env.core.clone();
    let sending = tokio::spawn(async move { core.send_clipboard_text("hotkey", kin_lib::sync::Tap::now()).await });
    assert_eq!(next_json(&mut ws).await["type"], "text");
    assert_eq!(sending.await.unwrap(), SendResult::NoReply);
    assert_eq!(toasts(&drain(&mut w.env.events)), ["No reply from the phone. The text was not sent."]);
}

#[tokio::test]
async fn an_error_from_the_phone_instead_of_an_ack_fails_the_send_with_its_code() {
    let mut w = World::new().await;
    let mut ws = w.connected().await;
    w.core().set_auto_clipboard_sync(false);
    w.env.clipboard.user_copy("explicit", vec![]);
    drain(&mut w.env.events);
    let core = w.env.core.clone();
    let sending = tokio::spawn(async move { core.send_clipboard_text("hotkey", kin_lib::sync::Tap::now()).await });
    let frame = next_json(&mut ws).await;
    send_json(&mut ws, &json!({ "type": "error", "ref": frame["id"], "code": "internal", "message": "the clipboard is not available" }))
        .await;
    assert_eq!(sending.await.unwrap(), SendResult::Rejected("internal".into()));
    assert_eq!(w.core().status_json()["lastSent"]["text"]["result"], "error:internal");
    assert_eq!(toasts(&drain(&mut w.env.events)), ["The phone couldn't take the text (internal)."]);
}

#[tokio::test]
async fn a_late_ack_for_a_send_that_already_failed_changes_nothing() {
    let w = World::new().await;
    let mut ws = w.connected().await;
    w.env.clipboard.user_copy("x", vec![]);
    let frame = next_json(&mut ws).await;
    next_close(&mut ws, Duration::from_secs(7)).await;
    drop(ws);
    // The phone reconnects and acks the old message: the PC has long given up on it, and ignores the ack.
    let mut ws = w.open_ws().await;
    send_json(&mut ws, &json!({ "type": "ack", "ref": frame["id"] })).await;
    send_json(&mut ws, &json!({ "type": "ping", "id": "after", "sentAt": 1 })).await;
    assert_eq!(next_json(&mut ws).await, json!({ "type": "ack", "ref": "after" }));
    assert_eq!(w.core().status_json()["lastSent"]["clipboard"]["result"], "no_reply");
}

// ----- logs -------------------------------------------------------------------------------------------------------

/// The real file logger, installed once for the whole test process (the global logger can only be set once), in a folder
/// under `target/` so nothing is left in the temp folder. Every test's log lines end up in it.
fn file_log_dir() -> &'static std::path::Path {
    static DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("test-logs").join(std::process::id().to_string());
        std::fs::create_dir_all(&dir).unwrap();
        kin_lib::filelog::init(&dir).unwrap();
        dir
    })
}

/// Connects, pairs and pings with the real file logger on, then checks that the log tells the story and holds no secret.
#[tokio::test]
async fn the_log_records_the_story_and_never_a_token_or_a_key() {
    let log_dir = file_log_dir();
    let w = World::new().await;
    let token = start_pairing_and_read_token(&w.env);
    let mut stream = w.tls().await;
    let r =
        request(&mut stream, "POST", "/v1/pair", &[("Content-Type", "application/json")], &pair_body(&token, PHONE_NAME)).await.unwrap();
    assert_eq!(r.status, 200);
    let mut ws = w.open_ws().await;
    send_json(&mut ws, &json!({ "type": "ping", "id": "log-ping", "sentAt": 1 })).await;
    next_json(&mut ws).await;
    ws.close(None).await.unwrap();
    w.wait_for_state(LinkState::PhoneOffline).await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    let log = std::fs::read_to_string(log_dir.join("kin.log")).unwrap();
    for needle in
        ["listening on", "pairing started", "pairing attempt", ": ok", "phone connected", "ping from Test Phone", "websocket ended"]
    {
        assert!(log.contains(needle), "the log has no {needle:?}:\n{log}");
    }
    assert!(!log.contains(&token), "the pairing token is in the log");
    assert!(!log.contains("kinrelay://"), "the pairing URL is in the log");
    assert!(!log.contains("PRIVATE KEY"));
    let key_b64 = |der: &[u8]| kin_lib::encoding::base64url(der);
    assert!(!log.contains(&key_b64(&w.pc.pkcs8)[..32]), "the private key is in the log");
}

/// The text of the clipboard is the user's own, and the log is something they export and send to us (D10). Run every 1.2 flow
/// with texts that are easy to search for, then look for them in the log and in status.json.
#[tokio::test]
async fn the_text_of_the_clipboard_never_reaches_the_log_or_status_json() {
    let log_dir = file_log_dir();
    let mut w = World::new().await;
    let mut ws = w.connected().await;
    let (auto, from_phone, link, hotkey, flagged, long) = (
        "AUTOSYNC-secret-4f1e",
        "FROMPHONE-secret-77ab",
        "https://example.com/LINK-secret-90cd?token=SECRETQUERY",
        "HOTKEY-secret-12ef",
        "FLAGGED-secret-5566",
        "TOOLONG-secret-aa11",
    );

    w.env.clipboard.user_copy(auto, vec![]);
    let frame = next_json(&mut ws).await;
    send_json(&mut ws, &json!({ "type": "ack", "ref": frame["id"] })).await;

    send_json(&mut ws, &text_message("s1", from_phone, false)).await;
    next_json(&mut ws).await;
    send_json(&mut ws, &text_message("s2", link, true)).await;
    next_json(&mut ws).await;
    let core = w.env.core.clone();
    wait_until("the link toast", || core.status_json()["lastReceived"]["text"]["id"] == "s2").await;
    let id = w.core().newest_link_toast().expect("a link toast");
    assert!(w.core().toast_clicked(id));

    w.env.clipboard.user_copy(flagged, vec![ClipFormat::named(EXCLUDE_FROM_MONITOR)]);
    assert_silent(&mut ws, 500).await;

    w.env.clipboard.user_copy(hotkey, vec![]);
    let frame = next_json(&mut ws).await; // the automatic `clipboard` for it
    send_json(&mut ws, &json!({ "type": "ack", "ref": frame["id"] })).await;
    let core = w.env.core.clone();
    let sending = tokio::spawn(async move { core.send_clipboard_text("hotkey", kin_lib::sync::Tap::now()).await });
    let frame = next_json(&mut ws).await;
    send_json(&mut ws, &json!({ "type": "ack", "ref": frame["id"] })).await;
    assert!(matches!(sending.await.unwrap(), SendResult::Acked { .. }));

    w.env.clipboard.user_copy(&format!("{long}{}", "x".repeat(1_048_576)), vec![]);
    assert_silent(&mut ws, 500).await;
    drain(&mut w.env.events);
    tokio::time::sleep(Duration::from_millis(200)).await;

    let log = std::fs::read_to_string(log_dir.join("kin.log")).unwrap();
    for secret in [auto, from_phone, hotkey, flagged, long, "LINK-secret", "SECRETQUERY"] {
        assert!(!log.contains(secret), "{secret} is in the log");
        assert!(!w.core().status_json().to_string().contains(secret), "{secret} is in status.json");
    }
    // What the log does say: that it happened, how long it was, a short hash, and the arrival times.
    for needle in [
        "clipboard ",
        "acked, tap to ack",
        "hash ",
        "characters",
        "opened a link",
        "not sent, the source app marked it ExcludeClipboardContentFromMonitorProcessing",
    ] {
        assert!(log.contains(needle), "the log has no {needle:?}");
    }
}
