//! A fake phone for manual checks of a running Kin, with no Android involved. It pairs from a pairing URL, opens the WebSocket,
//! says `hello`, acks every `ping`, `text` and `clipboard` it gets, and can send pings and `text` messages of its own.
//!
//! ```text
//! cargo run --example fake_phone -- "<kinrelay://pair?...>" --host 127.0.0.1 [--name "Fake Phone"] [--seconds 30] [--ping-every 5]
//!     [--send-text "some text" [--send-text-after 2]] [--no-ack]
//! ```
//!
//! * `--send-text <text>` sends one `text` message `--send-text-after` seconds (default 2) after the `hello`, with `isUrl` set when the
//!   whole text is one http(s) link, and prints how long its `ack` took. Repeat the flag to send several, one per second.
//! * `--no-ack` doesn't ack `text` and `clipboard` from the PC (to see the PC's 5 s ack timeout and its notice).
//! * What it receives is printed on one line each, for scripts to look for: `received clipboard id=<id> text=<text>` and
//!   `received text id=<id> isUrl=<bool> text=<text>`. (This is a developer tool: it prints the text. Kin itself never logs it.)
//!
//! `--host` replaces the host of the first address in the URL (in developer mode that is `10.0.2.2`, the emulator's alias for
//! the PC, which the PC itself can't reach). The port is the one in the URL. The phone's key is made fresh on every run.
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use kin_lib::encoding::{device_id_from_spki, spki_of_certificate};
use kin_lib::identity::Identity;
use kin_lib::tls::crypto_provider;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{WebPkiSupportedAlgorithms, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, Error as TlsError, SignatureScheme};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_tungstenite::tungstenite::Message;

#[derive(Debug)]
struct Pinned {
    device_id: String,
    algs: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        match spki_of_certificate(end_entity).map(device_id_from_spki) {
            Some(id) if id == self.device_id => Ok(ServerCertVerified::assertion()),
            _ => Err(TlsError::General("the PC's key is not the one in the pairing URL".into())),
        }
    }

    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        Err(TlsError::General("TLS 1.2 is not allowed".into()))
    }

    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        verify_tls13_signature(m, c, d, &self.algs)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algs.supported_schemes()
    }
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

/// Every value of a flag that can be given more than once.
fn all_args(args: &[String], name: &str) -> Vec<String> {
    args.iter().enumerate().filter(|(_, a)| *a == name).filter_map(|(i, _)| args.get(i + 1)).cloned().collect()
}

/// The same rule as `isUrl` in protocol.md, for this tool: the whole text, trimmed, is one http(s) URL.
fn is_url(text: &str) -> bool {
    kin_lib::links::is_http_url(text)
}

fn param<'a>(query: &'a str, name: &str) -> Option<&'a str> {
    query.split('&').find_map(|p| p.strip_prefix(name)?.strip_prefix('='))
}

fn say(msg: &str) {
    println!("[fake phone] {msg}");
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let url = args.first().expect("usage: fake_phone <pairing URL> [--host H] [--name N] [--seconds S] [--ping-every S]");
    let query = url.split_once('?').expect("not a pairing URL").1;
    let (token, pc_id) = (param(query, "tk").expect("no tk"), param(query, "id").expect("no id"));
    let first = param(query, "a").expect("no a").split(',').next().unwrap();
    let port: u16 = first.rsplit_once(':').unwrap().1.parse().unwrap();
    let host = arg(&args, "--host").unwrap_or_else(|| first.rsplit_once(':').unwrap().0.to_string());
    let name = arg(&args, "--name").unwrap_or_else(|| "Fake Phone".to_string());
    let seconds: u64 = arg(&args, "--seconds").map_or(30, |s| s.parse().unwrap());
    let ping_every: Option<u64> = arg(&args, "--ping-every").map(|s| s.parse().unwrap());
    let texts = all_args(&args, "--send-text");
    let send_text_after: u64 = arg(&args, "--send-text-after").map_or(2, |s| s.parse().unwrap());
    let no_ack = args.iter().any(|a| a == "--no-ack");

    let me = Identity::generate().unwrap();
    say(&format!("my device ID is {}", me.device_id));
    let provider = crypto_provider();
    let verifier = Pinned { device_id: pc_id.to_string(), algs: provider.signature_verification_algorithms };
    let mut config = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_client_auth_cert(
            vec![CertificateDer::from(me.cert_der.clone())],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(me.key_der.clone())),
        )
        .unwrap();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let connector = TlsConnector::from(Arc::new(config));
    let connect = || async {
        let tcp = TcpStream::connect((host.as_str(), port)).await.expect("connect");
        connector.connect(ServerName::try_from("kin").unwrap(), tcp).await.expect("TLS handshake")
    };

    // Pair.
    let mut tls = connect().await;
    let body =
        json!({ "token": token, "name": name, "platform": "android", "model": "Fake", "appVersion": "0.0.0", "protocol": 1 }).to_string();
    let request = format!(
        "POST /v1/pair HTTP/1.1\r\nHost: kin\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    tls.write_all(request.as_bytes()).await.unwrap();
    let mut response = String::new();
    let _ = tls.read_to_string(&mut response).await;
    let status = response.lines().next().unwrap_or("");
    say(&format!("pair: {status} {}", response.rsplit("\r\n\r\n").next().unwrap_or("")));
    assert!(status.contains(" 200"), "pairing failed");

    // WebSocket.
    let (mut ws, _) =
        tokio_tungstenite::client_async(format!("ws://{host}:{port}/v1/ws"), connect().await).await.expect("WebSocket upgrade");
    let hello = json!({ "type": "hello", "id": "fake-hello", "protocol": 1, "name": name, "platform": "android", "appVersion": "0.0.0" });
    ws.send(Message::Text(hello.to_string().into())).await.unwrap();
    say("connected, hello sent");

    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut next_ping = ping_every.map(|s| Instant::now() + Duration::from_secs(s));
    let mut sent: Option<(String, Instant)> = None;
    let mut counter = 0;
    let mut texts = texts.into_iter().enumerate();
    let mut next_text = Instant::now() + Duration::from_secs(send_text_after);
    let mut text_sent: Option<(String, Instant)> = None;
    let mut pending_text = texts.next();
    loop {
        let mut wake = next_ping.map_or(deadline, |p| p.min(deadline));
        if pending_text.is_some() {
            wake = wake.min(next_text);
        }
        tokio::select! {
            () = tokio::time::sleep_until(wake.into()) => {
                if Instant::now() >= deadline { break; }
                if let Some((i, text)) = pending_text.take().filter(|_| Instant::now() >= next_text) {
                    let id = format!("fake-text-{}", i + 1);
                    let frame = json!({ "type": "text", "id": id, "text": text, "isUrl": is_url(&text) });
                    ws.send(Message::Text(frame.to_string().into())).await.unwrap();
                    say(&format!("sent text id={id} isUrl={}", is_url(&text)));
                    text_sent = Some((id, Instant::now()));
                    pending_text = texts.next();
                    next_text = Instant::now() + Duration::from_secs(1);
                    continue;
                }
                if pending_text.is_some() && next_ping.is_none_or(|p| Instant::now() < p) { continue; }
                counter += 1;
                let id = format!("fake-ping-{counter}");
                ws.send(Message::Text(json!({ "type": "ping", "id": id, "sentAt": kin_lib::encoding::unix_ms() }).to_string().into())).await.unwrap();
                sent = Some((id, Instant::now()));
                next_ping = ping_every.map(|s| Instant::now() + Duration::from_secs(s));
            }
            msg = ws.next() => match msg {
                Some(Ok(Message::Text(t))) => {
                    let v: Value = serde_json::from_str(t.as_str()).unwrap();
                    match v["type"].as_str() {
                        Some("ping") => {
                            say(&format!("ping from the PC ({}): acking", v["id"]));
                            ws.send(Message::Text(json!({ "type": "ack", "ref": v["id"] }).to_string().into())).await.unwrap();
                        }
                        Some("ack") => {
                            if let Some((id, at)) = sent.take().filter(|(id, _)| v["ref"] == *id) {
                                say(&format!("ack for {id} after {} ms", at.elapsed().as_millis()));
                            } else if let Some((id, at)) = text_sent.take().filter(|(id, _)| v["ref"] == *id) {
                                say(&format!("ack for text {id} after {} ms", at.elapsed().as_millis()));
                            }
                        }
                        Some(kind @ ("clipboard" | "text")) => {
                            let text = v["text"].as_str().unwrap_or("");
                            if kind == "text" {
                                say(&format!("received text id={} isUrl={} text={text}", v["id"], v["isUrl"]));
                            } else {
                                say(&format!("received clipboard id={} text={text}", v["id"]));
                            }
                            if no_ack {
                                say("not acking (--no-ack)");
                            } else {
                                ws.send(Message::Text(json!({ "type": "ack", "ref": v["id"] }).to_string().into())).await.unwrap();
                            }
                        }
                        Some("unpair") => {
                            say("the PC unpaired this phone");
                            ws.send(Message::Text(json!({ "type": "ack", "ref": v["id"] }).to_string().into())).await.unwrap();
                        }
                        _ => say(&format!("received {v}")),
                    }
                }
                Some(Ok(Message::Close(frame))) => { say(&format!("closed by the PC: {frame:?}")); return; }
                Some(Ok(_)) => {}
                other => { say(&format!("connection ended: {other:?}")); return; }
            }
        }
    }
    say("done, closing");
    let _ = ws.close(None).await;
}
