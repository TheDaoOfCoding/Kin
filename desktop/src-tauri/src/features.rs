//! The per-feature settings model (design section 4): every feature is switched by **one function**, `enabled(feature, peer)` = the per-peer override if there is
//! one, else the device's default (today's flat key of `settings.json`). Today only the defaults have a UI; the override store exists, is empty and has no
//! UI, so the several devices of 2D need a screen and nothing else (no data migration). Unlinking a peer deletes its overrides.
//!
//! A feature acting on a connection passes **that connection's peer ID**; one that acts on nothing in particular passes none and gets the default.
use std::collections::BTreeMap;

use serde_json::{Map, Value};

/// `settings.json`: `"featureOverrides": { "<peer device id>": { "<feature id>": true|false } }`. Device IDs are not secret.
pub type Overrides = BTreeMap<String, BTreeMap<String, bool>>;

// The feature ids (design section 4). They are stable strings: a later screen and the Android side use the same ones.
pub const CLIPBOARD_SYNC: &str = "clipboard.sync";
pub const LINKS_AUTO_OPEN: &str = "links.autoOpen";
pub const NOTIF_SHOW: &str = "notif.show";
pub const MEDIA_SHARE: &str = "media.share";
pub const MEDIA_SHOW: &str = "media.show";
pub const PRESENCE_SHARE: &str = "presence.share";
pub const PRESENCE_SHOW: &str = "presence.show";
pub const PRESENCE_NUDGE: &str = "presence.nudge";
/// 2B.5: tell the phone when this PC is in Focus (`presence.focus`).
pub const FOCUS_SHARE: &str = "focus.share";
/// 2B.5: hold Kin's own optional toasts while the phone is in Do Not Disturb.
pub const DND_HOLD_TOASTS: &str = "dnd.holdToasts";

/// Every feature id this PC has a switch for.
pub const ALL: [&str; 10] = [
    CLIPBOARD_SYNC,
    LINKS_AUTO_OPEN,
    NOTIF_SHOW,
    MEDIA_SHARE,
    MEDIA_SHOW,
    PRESENCE_SHARE,
    PRESENCE_SHOW,
    PRESENCE_NUDGE,
    FOCUS_SHARE,
    DND_HOLD_TOASTS,
];

/// The resolver: an override for this peer if it is a boolean, else the default.
pub fn resolve(feature: &str, peer: Option<&str>, default: bool, overrides: &Overrides) -> bool {
    peer.and_then(|p| overrides.get(p)).and_then(|m| m.get(feature)).copied().unwrap_or(default)
}

/// The overrides as `settings.json` keeps them. A wrong shape is dropped entry by entry (a non-boolean value is not an override).
pub fn overrides_from_json(v: &Value) -> Overrides {
    let mut out = Overrides::new();
    let Some(peers) = v.as_object() else {
        return out;
    };
    for (peer, features) in peers {
        let Some(features) = features.as_object() else {
            continue;
        };
        let kept: BTreeMap<String, bool> = features.iter().filter_map(|(f, b)| b.as_bool().map(|b| (f.clone(), b))).collect();
        if !kept.is_empty() {
            out.insert(peer.clone(), kept);
        }
    }
    out
}

pub fn overrides_to_json(o: &Overrides) -> Value {
    let mut peers = Map::new();
    for (peer, features) in o {
        let mut m = Map::new();
        for (f, b) in features {
            m.insert(f.clone(), Value::Bool(*b));
        }
        peers.insert(peer.clone(), Value::Object(m));
    }
    Value::Object(peers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_override_beats_the_default_and_only_for_its_own_peer_and_feature() {
        let mut o = Overrides::new();
        o.entry("dev-a".into()).or_default().insert(CLIPBOARD_SYNC.into(), false);
        assert!(!resolve(CLIPBOARD_SYNC, Some("dev-a"), true, &o));
        assert!(resolve(CLIPBOARD_SYNC, Some("dev-b"), true, &o), "another peer");
        assert!(resolve(CLIPBOARD_SYNC, None, true, &o), "no peer: the default");
        assert!(resolve(NOTIF_SHOW, Some("dev-a"), true, &o), "another feature");
        o.get_mut("dev-a").unwrap().insert(NOTIF_SHOW.into(), true);
        assert!(resolve(NOTIF_SHOW, Some("dev-a"), false, &o), "on beats a default of off");
    }

    #[test]
    fn the_overrides_round_trip_and_a_non_boolean_is_not_an_override() {
        let v = json!({ "dev-a": { "clipboard.sync": false, "media.show": "no" }, "dev-b": "x", "dev-c": {} });
        let o = overrides_from_json(&v);
        assert_eq!(o.len(), 1);
        assert_eq!(o["dev-a"].len(), 1);
        assert_eq!(overrides_from_json(&overrides_to_json(&o)), o);
        assert_eq!(overrides_from_json(&json!(5)), Overrides::new());
    }

    /// The vectors of `features[]`.
    #[test]
    fn the_vectors_of_the_resolver() {
        let root: Value = serde_json::from_str(
            &std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../protocol/presence.json")).unwrap(),
        )
        .unwrap();
        let cases = root["features"].as_array().unwrap();
        assert!(cases.len() >= 10);
        for c in cases {
            let feature = c["feature"].as_str().unwrap();
            let peer = c["peer"].as_str();
            let default = c["defaults"].get(feature).and_then(Value::as_bool).unwrap_or(false);
            // The vectors keep overrides that are not booleans too: `overrides_from_json` drops them, and `resolve` never sees them.
            let overrides = overrides_from_json(&c["overrides"]);
            assert_eq!(resolve(feature, peer, default, &overrides), c["expected"].as_bool().unwrap(), "{}", c["name"]);
        }
    }

    /// Design section 4: "no feature code reads its flat key directly". The switches are read through `Core::feature_enabled` (which goes through `resolve`),
    /// so a later screen of per-peer overrides can't be bypassed. Only `settings.rs` (the file format) and the code that writes a switch may name the fields.
    #[test]
    fn no_feature_code_reads_its_flat_key_directly() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let flat = [
            "settings).auto_clipboard_sync",
            "settings).open_links_automatically",
            "settings).mirror_notifications",
            "settings).media_share",
            "settings).media_show",
            "settings).presence_share_battery",
            "settings).presence_show_phone",
            "settings).low_battery_nudge",
        ];
        let mut offenders = Vec::new();
        let mut stack = vec![src];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") && path.file_name().is_some_and(|n| n != "features.rs") {
                    let text = std::fs::read_to_string(&path).unwrap();
                    for (n, line) in text.lines().enumerate() {
                        // A write (`lock(&self.settings).x = true`) is the setting being changed, not a feature reading its switch.
                        let reads = flat
                            .iter()
                            .any(|k| line.contains(k) && !line.contains(&format!("{k} =")) && !line.trim_start().starts_with("//"));
                        if reads {
                            offenders.push(format!("{}:{}", path.file_name().unwrap().to_string_lossy(), n + 1));
                        }
                    }
                }
            }
        }
        assert!(offenders.is_empty(), "these lines read a switch without the resolver: {offenders:?}");
    }
}
