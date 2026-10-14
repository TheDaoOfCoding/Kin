//! The JSON rules of protocol.md ("JSON conventions") that serde_json doesn't enforce by itself.
//!
//! serde_json is already strict RFC 8259 (no unquoted literals, no leading zeros, no trailing commas), rejects lone
//! surrogate escapes, keeps the last of duplicate keys and treats `1.0` and `1e3` as floats. What it lacks is our
//! nesting limit, so that is counted here before parsing.
use serde_json::{Map, Value};

pub const MAX_NESTING: usize = 64;
/// 2^53 - 1, so the TypeScript UI never loses precision.
pub const MAX_SAFE_INT: u64 = (1 << 53) - 1;
pub const MAX_ID_CHARS: usize = 64;

/// The deepest nesting of arrays and objects in the text, counted while skipping string contents.
/// The top-level object is level 1.
pub fn nesting_depth(text: &str) -> usize {
    let (mut depth, mut max) = (0usize, 0usize);
    let (mut in_string, mut escaped) = (false, false);
    for b in text.bytes() {
        if in_string {
            match (escaped, b) {
                (true, _) => escaped = false,
                (false, b'\\') => escaped = true,
                (false, b'"') => in_string = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'[' | b'{' => {
                depth += 1;
                max = max.max(depth);
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    max
}

/// Parses one JSON text into an object. Anything else (a syntax error, too deep, not an object) is `None`.
/// The caller maps `None` to `bad_request`.
pub fn parse_object(text: &str) -> Option<Map<String, Value>> {
    if nesting_depth(text) > MAX_NESTING {
        return None;
    }
    match serde_json::from_str::<Value>(text).ok()? {
        Value::Object(map) => Some(map),
        _ => None,
    }
}

/// An integer from 0 to 2^53 - 1. A fraction, an exponent, a sign, a string or a bigger number is `None`.
pub fn uint(v: &Value) -> Option<u64> {
    v.as_u64().filter(|n| *n <= MAX_SAFE_INT)
}

/// A string of 1 to 64 code points, as message IDs are.
pub fn valid_id(v: Option<&Value>) -> Option<&str> {
    let s = v?.as_str()?;
    let n = s.chars().count();
    (1..=MAX_ID_CHARS).contains(&n).then_some(s)
}

/// A required string of at least one code point.
pub fn non_empty_str<'a>(map: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    map.get(key)?.as_str().filter(|s| !s.is_empty())
}

/// A field that is present with a type the protocol doesn't allow.
#[derive(Debug, PartialEq, Eq)]
pub struct WrongType;

/// An optional field: absent and `null` are the same, any other non-string is an error.
pub fn optional_str<'a>(map: &'a Map<String, Value>, key: &str) -> Result<Option<&'a str>, WrongType> {
    match map.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(WrongType),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depth_ignores_brackets_in_strings() {
        assert_eq!(nesting_depth(r#"{"a":"[[[[","b":[1,{"c":"\"]]"}]}"#), 3);
        assert_eq!(nesting_depth("{}"), 1);
        assert_eq!(nesting_depth("1"), 0);
    }

    #[test]
    fn objects_only_and_strict() {
        assert!(parse_object(r#"{"a":1}"#).is_some());
        assert!(parse_object("[1]").is_none());
        assert!(parse_object(r#"{"a":01}"#).is_none());
        assert!(parse_object(r#"{"a":"\ud800"}"#).is_none());
        assert_eq!(parse_object(r#"{"a":1,"a":2}"#).unwrap()["a"], 2);
    }

    #[test]
    fn integers_follow_the_spec() {
        assert_eq!(uint(&serde_json::json!(5)), Some(5));
        assert_eq!(uint(&serde_json::json!(9007199254740991u64)), Some(MAX_SAFE_INT));
        assert_eq!(uint(&serde_json::json!(9007199254740992u64)), None);
        assert_eq!(uint(&serde_json::json!(-1)), None);
        assert_eq!(uint(&serde_json::json!(1.0)), None);
        assert_eq!(uint(&serde_json::json!("1")), None);
    }
}
