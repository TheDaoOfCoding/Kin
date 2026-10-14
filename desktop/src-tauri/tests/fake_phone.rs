//! A fake phone against the real server in the test process: a rustls client with its own P-256 certificate that pairs with a
//! real token, opens the WebSocket, says `hello`, sends and receives `ping` and `ack`, is replaced by a second connection
//! (4001), is refused after being unpaired (401), and gets the close codes for bad frames.
//!
//! Everything listens on 127.0.0.1 with a random port.
mod common;

use std::time::Duration;

use common::*;
use futures_util::{SinkExt, StreamExt};
use kin_lib::core::{Core, Event, LinkState, PingReport};
use kin_lib::identity::Identity;
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

// ----- logs -------------------------------------------------------------------------------------------------------

/// Connects, pairs and pings with the real file logger on, then checks that the log tells the story and holds no secret.
#[tokio::test]
async fn the_log_records_the_story_and_never_a_token_or_a_key() {
    let log_dir = tempfile::tempdir().unwrap();
    kin_lib::filelog::init(log_dir.path()).unwrap();
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

    let log = std::fs::read_to_string(log_dir.path().join("kin.log")).unwrap();
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
