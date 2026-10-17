//! The only fields of a request the host looks at: `v`, `id`, `type`. Everything else is forwarded byte for byte after the size checks, so a
//! new field needs no host update. Also the responses the host makes on its own (the errors marked "host" in docs/extension.md).

use serde::Deserialize;
use serde_json::{Value, json};

/// What the host looks at. serde skips every other field without keeping it (so a 40 MB `data` string costs no memory here).
#[derive(Deserialize)]
struct Raw {
    v: Option<Value>,
    id: Option<Value>,
    #[serde(rename = "type")]
    ty: Option<Value>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Envelope {
    pub id: String,
    pub ty: String,
}

pub const KIN_NOT_RUNNING: (&str, &str) = ("KIN_NOT_RUNNING", "Kin isn't running. Start Kin and try again.");
pub const TIMEOUT: (&str, &str) = ("TIMEOUT", "Kin didn't answer in time.");
pub const TOO_LARGE: (&str, &str) = ("TOO_LARGE", "That's too big to send.");
pub const BAD_REQUEST: (&str, &str) = ("BAD_REQUEST", "The request was not valid.");
pub const UNSUPPORTED_TYPE: (&str, &str) = ("UNSUPPORTED_TYPE", "Kin doesn't know that kind of request.");
pub const UNSUPPORTED_VERSION: (&str, &str) = ("UNSUPPORTED_VERSION", "Update Kin or the Kin browser extension.");
pub const INTERNAL: (&str, &str) = ("INTERNAL", "Something went wrong inside Kin.");

/// A host-made error response. `ty` is the request's type, or `"error"` when the request couldn't be understood; `id` is left out then.
pub fn error_response(id: Option<&str>, ty: &str, (code, message): (&str, &str)) -> Vec<u8> {
    let mut m = json!({ "v": 1, "type": ty, "ok": false, "code": code, "message": message });
    if let Some(id) = id {
        m["ref"] = json!(id);
    }
    if code == UNSUPPORTED_VERSION.0 {
        m["supported"] = json!([1]);
    }
    serde_json::to_vec(&m).expect("a Value always serializes")
}

/// Why a request was refused before it could be forwarded: the response to send.
pub type Rejection = Vec<u8>;

/// Checks a request body: a JSON object with a usable `id`, a string `type` and `v` 1.
pub fn parse(body: &[u8]) -> Result<Envelope, Rejection> {
    let bad = || error_response(None, "error", BAD_REQUEST);
    // serde would also take a JSON array for a struct, so the object check is by hand. After a successful parse the text is one complete
    // JSON value, so a body that starts with `{` is an object.
    if body.iter().find(|b| !b.is_ascii_whitespace()) != Some(&b'{') {
        return Err(bad());
    }
    let raw: Raw = serde_json::from_slice(body).map_err(|_| bad())?;
    let id = match raw.id {
        Some(Value::String(s)) if (1..=64).contains(&s.chars().count()) => s,
        _ => return Err(bad()),
    };
    let ty = match raw.ty {
        Some(Value::String(s)) => s,
        _ => return Err(bad()),
    };
    if raw.v.as_ref().and_then(Value::as_u64) != Some(1) {
        return Err(error_response(Some(&id), "error", UNSUPPORTED_VERSION));
    }
    Ok(Envelope { id, ty })
}

/// The request as it goes to Kin: the extension's bytes unchanged, with `origin` and `hostVersion` added at the END of the object (a parser
/// that keeps the last of two equal keys, as serde_json does, takes the host's values even if the extension sent its own).
pub fn add_host_fields(mut body: Vec<u8>, origin: &str, host_version: &str) -> Vec<u8> {
    let end = body.iter().rposition(|b| !b.is_ascii_whitespace()).expect("a parsed object is not empty");
    debug_assert_eq!(body[end], b'}');
    let empty = body[..end].iter().rev().find(|b| !b.is_ascii_whitespace()) == Some(&b'{');
    body.truncate(end);
    if !empty {
        body.push(b',');
    }
    body.extend_from_slice(b"\"origin\":");
    body.extend_from_slice(serde_json::to_string(origin).expect("a string serializes").as_bytes());
    body.extend_from_slice(b",\"hostVersion\":");
    body.extend_from_slice(serde_json::to_string(host_version).expect("a string serializes").as_bytes());
    body.push(b'}');
    body
}

/// The host's own answer when Kin isn't running (a normal answer for `status`, not an error).
pub fn not_running_status(id: &str, host_version: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({ "v": 1, "ref": id, "type": "status", "ok": true, "running": false, "hostVersion": host_version, "devices": [] }))
        .expect("a Value always serializes")
}

/// Kin's answer on its way to the browser. Unchanged, except that a `status` or `start` answer gets the host's version if Kin left it out.
/// `None` when it isn't a JSON object, or is over the browser's 1 MB limit.
pub fn finish_response(kin_answer: Vec<u8>, request_ty: &str, host_version: &str, max: usize) -> Option<Vec<u8>> {
    if kin_answer.len() > max {
        return None;
    }
    if request_ty != "status" && request_ty != "start" {
        // Still has to be a JSON object: the browser would otherwise be fed anything a same-user program wrote to the pipe.
        return match serde_json::from_slice::<Value>(&kin_answer) {
            Ok(Value::Object(_)) => Some(kin_answer),
            _ => None,
        };
    }
    match serde_json::from_slice::<Value>(&kin_answer) {
        Ok(Value::Object(mut m)) => {
            if m.get("ok") == Some(&Value::Bool(true)) && !m.contains_key("hostVersion") {
                m.insert("hostVersion".into(), json!(host_version));
                let out = serde_json::to_vec(&Value::Object(m)).ok()?;
                return if out.len() > max { None } else { Some(out) };
            }
            Some(kin_answer)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code_of(r: &[u8]) -> (Option<String>, String, String) {
        let v: Value = serde_json::from_slice(r).unwrap();
        (v.get("ref").and_then(|r| r.as_str()).map(String::from), v["type"].as_str().unwrap().into(), v["code"].as_str().unwrap().into())
    }

    #[test]
    fn accepts_a_valid_envelope_and_ignores_other_fields() {
        let e = parse(br#"  {"v":1,"id":"7d0a","type":"send","kind":"link","url":"https://x.test/","future":{"a":[1,2,3]}} "#).unwrap();
        assert_eq!(e, Envelope { id: "7d0a".into(), ty: "send".into() });
    }

    #[test]
    fn bad_json_and_non_objects_are_bad_request_without_a_ref() {
        for body in [&b"{"[..], b"not json", b"", b"[1,\"a\",\"status\"]", b"\"str\"", b"123", b"null", br#"{"v":1,"id":"a","type":"status"} trailing"#] {
            let r = parse(body).unwrap_err();
            assert_eq!(code_of(&r), (None, "error".into(), "BAD_REQUEST".into()), "{:?}", String::from_utf8_lossy(body));
        }
    }

    #[test]
    fn id_and_type_rules() {
        let ok = |id: &str| parse(format!(r#"{{"v":1,"id":{},"type":"status"}}"#, serde_json::to_string(id).unwrap()).as_bytes());
        assert!(ok("a").is_ok());
        assert!(ok(&"x".repeat(64)).is_ok());
        assert!(ok(&"é".repeat(64)).is_ok(), "64 characters, 128 bytes");
        assert!(ok("").is_err());
        assert!(ok(&"x".repeat(65)).is_err());
        for body in [
            r#"{"v":1,"type":"status"}"#,
            r#"{"v":1,"id":5,"type":"status"}"#,
            r#"{"v":1,"id":null,"type":"status"}"#,
            r#"{"v":1,"id":"a"}"#,
            r#"{"v":1,"id":"a","type":7}"#,
            r#"{"v":1,"id":"a","id":"b","type":"status"}"#,
        ] {
            assert_eq!(code_of(&parse(body.as_bytes()).unwrap_err()).2, "BAD_REQUEST", "{body}");
        }
    }

    #[test]
    fn wrong_or_missing_version_is_unsupported_version_with_the_ref() {
        for body in [
            r#"{"v":2,"id":"abc","type":"status"}"#,
            r#"{"v":0,"id":"abc","type":"status"}"#,
            r#"{"v":"1","id":"abc","type":"status"}"#,
            r#"{"v":1.5,"id":"abc","type":"status"}"#,
            r#"{"id":"abc","type":"status"}"#,
        ] {
            let r = parse(body.as_bytes()).unwrap_err();
            assert_eq!(code_of(&r), (Some("abc".into()), "error".into(), "UNSUPPORTED_VERSION".into()), "{body}");
            assert_eq!(serde_json::from_slice::<Value>(&r).unwrap()["supported"], json!([1]));
        }
    }

    #[test]
    fn host_fields_are_added_at_the_end_and_the_rest_is_unchanged() {
        let body = "{ \"v\":1, \"id\":\"a\", \"type\":\"send\", \"text\":\"h\\u00e9llo \\ud83d\\ude00\", \"n\": 1.50 }  ".as_bytes().to_vec();
        let out = add_host_fields(body.clone(), "chrome-extension://abc/", "0.1.0");
        let s = String::from_utf8(out.clone()).unwrap();
        assert!(s.starts_with("{ \"v\":1, \"id\":\"a\", \"type\":\"send\", \"text\":\"h\\u00e9llo \\ud83d\\ude00\", \"n\": 1.50 ,\"origin\":"), "{s}");
        assert!(s.ends_with(r#","origin":"chrome-extension://abc/","hostVersion":"0.1.0"}"#), "{s}");
        let v: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["origin"], "chrome-extension://abc/");
        assert_eq!(v["hostVersion"], "0.1.0");
        assert_eq!(v["text"], "héllo 😀");
    }

    #[test]
    fn host_fields_win_over_the_extensions_own() {
        let out = add_host_fields(br#"{"v":1,"id":"a","type":"status","origin":"evil","hostVersion":"9"}"#.to_vec(), "chrome-extension://abc/", "0.1.0");
        let v: Value = serde_json::from_slice(&out).unwrap(); // serde_json keeps the last of two equal keys
        assert_eq!((v["origin"].as_str(), v["hostVersion"].as_str()), (Some("chrome-extension://abc/"), Some("0.1.0")));
    }

    #[test]
    fn host_fields_escape_a_hostile_origin_and_handle_an_empty_object() {
        let out = add_host_fields(b"{}".to_vec(), "a\"b\\c\n", "1");
        let v: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["origin"], "a\"b\\c\n");
        let out = add_host_fields(b"{ }".to_vec(), "", "1");
        assert!(serde_json::from_slice::<Value>(&out).is_ok());
    }

    #[test]
    fn status_answers_get_the_host_version_when_kin_left_it_out() {
        let kin = br#"{"v":1,"ref":"a","type":"status","ok":true,"running":true,"devices":[]}"#.to_vec();
        let out = finish_response(kin.clone(), "status", "0.1.0", 1 << 20).unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&out).unwrap()["hostVersion"], "0.1.0");
        // Kin's own value stays, and so does everything else unchanged (same bytes when nothing is added).
        let with = br#"{"v":1,"ref":"a","type":"start","ok":true,"hostVersion":"kin says"}"#.to_vec();
        assert_eq!(finish_response(with.clone(), "start", "0.1.0", 1 << 20).unwrap(), with);
        // A send answer is never touched.
        let send = br#"{ "v":1, "ref":"a", "type":"send", "ok":true }"#.to_vec();
        assert_eq!(finish_response(send.clone(), "send", "0.1.0", 1 << 20).unwrap(), send);
        // An error answer to a status is not given a host version.
        let err = br#"{"v":1,"ref":"a","type":"status","ok":false,"code":"INTERNAL","message":"x"}"#.to_vec();
        assert_eq!(finish_response(err.clone(), "status", "0.1.0", 1 << 20).unwrap(), err);
    }

    #[test]
    fn answers_over_the_limit_or_not_objects_are_refused() {
        assert!(finish_response(vec![b' '; 100], "send", "1", 50).is_none());
        assert!(finish_response(b"[1]".to_vec(), "send", "1", 50).is_none());
        assert!(finish_response(b"nope".to_vec(), "status", "1", 50).is_none());
        assert!(finish_response(b"".to_vec(), "open", "1", 50).is_none());
    }
}
