//! What the server does with clients that are slow, idle or too many (carry-forward item (b) of the 1.1 review): a client that
//! passed the TLS handshake with a certificate of its own can't hold a connection forever, and can't use up the connections that
//! a paired phone needs.
//!
//! The limits are the production ones in `Limits::default()`, scaled down so the tests take a fraction of a second.
mod common;

use std::time::{Duration, Instant};

use common::*;
use futures_util::{SinkExt, StreamExt};
use kin_lib::identity::Identity;
use kin_lib::server::{self, Limits, ServerHandle};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_rustls::client::TlsStream;
use tokio_tungstenite::tungstenite::Message;

type Stream = TlsStream<tokio::net::TcpStream>;

fn short_limits() -> Limits {
    Limits {
        tls_handshake_timeout: Duration::from_millis(500),
        header_timeout: Duration::from_millis(400),
        body_timeout: Duration::from_millis(400),
        idle_timeout: Duration::from_millis(400),
        ..Limits::default()
    }
}

struct Rig {
    env: TestEnv,
    server: ServerHandle,
    pc: TestKey,
    phone: TestKey,
}

fn key_of(identity: Identity) -> TestKey {
    TestKey { pkcs8: identity.key_der, cert: identity.cert_der, device_id: identity.device_id, short_id: identity.short_id }
}

impl Rig {
    async fn new(limits: Limits) -> Rig {
        let pc_identity = Identity::generate().unwrap();
        let pc = TestKey {
            pkcs8: pc_identity.key_der.clone(),
            cert: pc_identity.cert_der.clone(),
            device_id: pc_identity.device_id.clone(),
            short_id: pc_identity.short_id.clone(),
        };
        let env = test_core(pc_identity, "Desk PC");
        let server = server::start_with(env.core.clone(), limits).await.unwrap();
        Rig { env, server, pc, phone: key_of(Identity::generate().unwrap()) }
    }

    async fn connect_as(&self, key: &TestKey) -> std::io::Result<Stream> {
        connect(self.server.local_addr, phone_config(&self.pc.device_id, key)).await
    }

    /// Pairs `self.phone` and drops the connection.
    async fn pair(&self) {
        let token = start_pairing_and_read_token(&self.env);
        let mut stream = self.connect_as(&self.phone).await.unwrap();
        let r = request(&mut stream, "POST", "/v1/pair", &[("Content-Type", "application/json")], &pair_body(&token, "Test Phone"))
            .await
            .unwrap();
        assert_eq!(r.status, 200);
    }

    /// An unpaired client with a key of its own that has finished the handshake and a first request, and then keeps the
    /// connection open.
    async fn stranger(&self) -> std::io::Result<Stream> {
        let mut stream = self.connect_as(&key_of(Identity::generate().unwrap())).await?;
        let r = request(&mut stream, "GET", "/v1/nothing", &[], &[]).await?;
        assert_eq!(r.status, 404);
        Ok(stream)
    }

    async fn open_ws(&self) -> tokio_tungstenite::WebSocketStream<Stream> {
        let stream = self.connect_as(&self.phone).await.unwrap();
        let url = format!("ws://127.0.0.1:{}/v1/ws", self.server.local_addr.port());
        let (mut ws, response) = tokio_tungstenite::client_async(url, stream).await.expect("the paired phone gets its WebSocket");
        assert_eq!(response.status().as_u16(), 101);
        let hello = next_json(&mut ws).await;
        assert_eq!(hello["type"], "hello");
        ws.send(Message::Text(
            json!({ "type": "hello", "id": "h1", "protocol": 1, "name": "Test Phone", "platform": "android", "appVersion": "0.1.0" })
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
        ws
    }
}

async fn next_json(ws: &mut tokio_tungstenite::WebSocketStream<Stream>) -> Value {
    loop {
        match tokio::time::timeout(Duration::from_secs(5), ws.next()).await.expect("a frame within 5 s") {
            Some(Ok(Message::Text(t))) => return serde_json::from_str(t.as_str()).unwrap(),
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
            other => panic!("expected a text frame, got {other:?}"),
        }
    }
}

/// Reads until the server closes the connection (end of stream or an error) and returns how long that took and what arrived.
async fn until_closed(stream: &mut Stream, within: Duration) -> (Duration, Vec<u8>) {
    let started = Instant::now();
    let mut got = Vec::new();
    loop {
        let mut chunk = [0u8; 1024];
        match tokio::time::timeout(within.saturating_sub(started.elapsed()), stream.read(&mut chunk)).await {
            Err(_) => panic!("the server did not close the connection within {within:?} (got {:?})", String::from_utf8_lossy(&got)),
            Ok(Ok(0)) | Ok(Err(_)) => return (started.elapsed(), got),
            Ok(Ok(n)) => got.extend_from_slice(&chunk[..n]),
        }
    }
}

// ----- timeouts ---------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_client_that_sends_half_a_header_and_stops_is_cut_off() {
    // The idle timeout is long here, so it is the header timeout that has to do it (in production the header timeout, 10 s, is
    // shorter than the idle one, 30 s).
    let rig = Rig::new(Limits { idle_timeout: Duration::from_secs(30), ..short_limits() }).await;
    let mut stream = rig.connect_as(&rig.phone).await.unwrap();
    stream.write_all(b"GET /v1/nothing HTTP/1.1\r\nHost: kin\r\nX-Slow: ").await.unwrap();
    stream.flush().await.unwrap();
    // The header timeout is 400 ms. Whatever the server says (hyper may answer 408), the connection is gone soon after.
    let (took, _) = until_closed(&mut stream, Duration::from_secs(3)).await;
    assert!(took >= Duration::from_millis(300), "closed too early: {took:?}");
}

#[tokio::test]
async fn a_client_that_sends_a_body_too_slowly_gets_400_and_is_cut_off() {
    // As above: the body timeout (15 s in production) is shorter than the idle timeout (30 s), so it is the one that fires.
    let rig = Rig::new(Limits { idle_timeout: Duration::from_secs(30), ..short_limits() }).await;
    let mut stream = rig.connect_as(&rig.phone).await.unwrap();
    // It promises 500 bytes of JSON and sends 10.
    stream
        .write_all(b"POST /v1/pair HTTP/1.1\r\nHost: kin\r\nContent-Type: application/json\r\nContent-Length: 500\r\n\r\n{\"token\":\"")
        .await
        .unwrap();
    stream.flush().await.unwrap();
    let started = Instant::now();
    let response = tokio::time::timeout(Duration::from_secs(3), read_response(&mut stream)).await.expect("an answer").unwrap();
    assert_eq!((response.status, response.json()["error"].clone()), (400, json!("bad_request")));
    assert!(started.elapsed() >= Duration::from_millis(300), "answered before the body timeout: {:?}", started.elapsed());
    // And the connection ends: the half-read body must not be taken for the next request.
    until_closed(&mut stream, Duration::from_secs(3)).await;
}

#[tokio::test]
async fn an_idle_connection_that_is_not_a_websocket_is_closed() {
    // hyper's header timeout also ends a keep-alive connection that is waiting for its next request, so it is made long here:
    // the idle timeout is what has to close this one.
    let rig = Rig::new(Limits { header_timeout: Duration::from_secs(30), ..short_limits() }).await;
    let mut stream = rig.connect_as(&rig.phone).await.unwrap();
    let r = request(&mut stream, "GET", "/v1/nothing", &[], &[]).await.unwrap();
    assert_eq!(r.status, 404);
    // Keep-alive: the connection stays open for the next request, but not for ever.
    let (took, got) = until_closed(&mut stream, Duration::from_secs(3)).await;
    assert!(got.is_empty());
    assert!(took >= Duration::from_millis(300), "closed too early: {took:?}");
}

#[tokio::test]
async fn a_connection_that_keeps_talking_is_not_idle() {
    let rig = Rig::new(Limits { header_timeout: Duration::from_secs(30), ..short_limits() }).await;
    let mut stream = rig.connect_as(&rig.phone).await.unwrap();
    for _ in 0..6 {
        tokio::time::sleep(Duration::from_millis(150)).await;
        let r = request(&mut stream, "GET", "/v1/nothing", &[], &[]).await.unwrap();
        assert_eq!(r.status, 404);
    }
}

#[tokio::test]
async fn a_websocket_is_not_closed_for_being_idle() {
    let rig = Rig::new(Limits { header_timeout: Duration::from_secs(30), ..short_limits() }).await;
    rig.pair().await;
    let mut ws = rig.open_ws().await;
    // Three times the idle timeout with nothing on the wire.
    tokio::time::sleep(Duration::from_millis(1300)).await;
    ws.send(Message::Text(json!({ "type": "ping", "id": "p1", "sentAt": 1 }).to_string().into())).await.unwrap();
    assert_eq!(next_json(&mut ws).await, json!({ "type": "ack", "ref": "p1" }));
}

#[tokio::test]
async fn a_client_that_never_finishes_the_tls_handshake_is_dropped() {
    let rig = Rig::new(short_limits()).await;
    let mut tcp = tokio::net::TcpStream::connect(rig.server.local_addr).await.unwrap();
    // Not a ClientHello, and then silence.
    tcp.write_all(&[0x16, 0x03, 0x01]).await.unwrap();
    let started = Instant::now();
    let mut buf = [0u8; 16];
    let n = tokio::time::timeout(Duration::from_secs(3), tcp.read(&mut buf)).await.expect("the server gives up").unwrap_or(0);
    assert_eq!(n, 0);
    assert!(started.elapsed() >= Duration::from_millis(400), "{:?}", started.elapsed());
}

// ----- capacity ---------------------------------------------------------------------------------------------------

#[tokio::test]
async fn strangers_can_fill_their_own_slots_but_not_the_paired_phones() {
    let limits = Limits { handshake_slots: 4, unpaired_slots: 3, paired_slots: 2, per_ip: 100, ..Limits::default() };
    let rig = Rig::new(limits).await;
    rig.pair().await;

    // Three strangers that passed the handshake (each has its own certificate) hold every unpaired slot.
    let mut strangers = Vec::new();
    for _ in 0..3 {
        strangers.push(rig.stranger().await.unwrap());
    }
    // A fourth is let through the handshake and then turned away: its first request gets no answer.
    let fourth = rig.stranger().await;
    assert!(fourth.is_err(), "a fourth unpaired connection should have been refused");

    // The paired phone isn't affected: its connection lands in the slots that are kept for paired devices.
    let mut ws = rig.open_ws().await;
    ws.send(Message::Text(json!({ "type": "ping", "id": "p1", "sentAt": 1 }).to_string().into())).await.unwrap();
    assert_eq!(next_json(&mut ws).await, json!({ "type": "ack", "ref": "p1" }));
    let mut http = rig.connect_as(&rig.phone).await.unwrap();
    assert_eq!(request(&mut http, "GET", "/v1/nothing", &[], &[]).await.unwrap().status, 404);

    // When a stranger leaves, its slot is free again.
    strangers.pop();
    tokio::time::sleep(Duration::from_millis(200)).await;
    rig.stranger().await.expect("a slot was freed");
}

#[tokio::test]
async fn one_address_cannot_hold_more_than_its_share() {
    let limits = Limits { per_ip: 2, ..Limits::default() };
    let rig = Rig::new(limits).await;
    let a = rig.stranger().await.unwrap();
    let _b = rig.stranger().await.unwrap();
    // The third connection from 127.0.0.1 is refused before the TLS handshake: the connect itself fails or the first request does.
    assert!(rig.stranger().await.is_err());
    drop(a);
    tokio::time::sleep(Duration::from_millis(200)).await;
    rig.stranger().await.expect("the count went down when a connection closed");
}

#[tokio::test]
async fn a_paired_phones_connections_do_not_count_against_its_address() {
    let limits = Limits { per_ip: 2, ..Limits::default() };
    let rig = Rig::new(limits).await;
    rig.pair().await;
    // Four connections from the same address, all paired: more than `per_ip`, and all fine, because the count is for
    // connections that are handshaking or unpaired. (Two at a time can be in the handshake, which is all this needs.)
    let mut open = Vec::new();
    for _ in 0..4 {
        let mut s = rig.connect_as(&rig.phone).await.unwrap();
        assert_eq!(request(&mut s, "GET", "/v1/nothing", &[], &[]).await.unwrap().status, 404);
        open.push(s);
    }
}
