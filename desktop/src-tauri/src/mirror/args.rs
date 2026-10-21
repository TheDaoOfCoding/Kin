//! The strings a toast carries for a click: `kin=1&d=<phone short ID>&n=<nid>&a=<action>[&i=<index>]` (design 5.3, vectors in `notif.json`:
//! `toastIds[]` and `toastArguments[]`). They hold **no content**, only identifiers, because Windows keeps them with the toast.
//!
//! They are read back **strictly**: the exact keys in this order, `kin` = 1, an 8-character short ID, a 32-hex `nid`, one of four actions, an index
//! (0 to 31, no sign, no leading zero) only for the two actions that need one, and at most 200 characters. Anything else is refused: the COM class
//! can be called by any program of the same user, so it believes nothing it can't check.
use super::wire::is_hex32;

/// The longest arguments string that is read.
pub const MAX_ARGS_CHARS: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastAction {
    /// A click on the toast itself.
    Tap,
    /// The reply box's Send button.
    Reply,
    /// The "Copy <code>" button.
    Copy,
    /// A button of the phone's (its index in the phone's list).
    Button,
}

impl ToastAction {
    fn word(self) -> &'static str {
        match self {
            ToastAction::Tap => "tap",
            ToastAction::Reply => "reply",
            ToastAction::Copy => "copy",
            ToastAction::Button => "btn",
        }
    }

    fn needs_index(self) -> bool {
        matches!(self, ToastAction::Reply | ToastAction::Button)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToastArgs {
    /// The phone's 8-character short ID.
    pub device: String,
    pub nid: String,
    pub action: ToastAction,
    pub index: Option<u8>,
}

impl ToastArgs {
    pub fn new(device: &str, nid: &str, action: ToastAction, index: Option<u8>) -> ToastArgs {
        ToastArgs { device: device.to_string(), nid: nid.to_string(), action, index }
    }

    /// The string for a toast's `launch` or a button's `arguments`.
    pub fn to_arguments(&self) -> String {
        let mut s = format!("kin=1&d={}&n={}&a={}", self.device, self.nid, self.action.word());
        if let Some(i) = self.index {
            s.push_str(&format!("&i={i}"));
        }
        s
    }
}

/// The tag of a toast of this `nid` (Windows allows 64 characters, the `nid` has 32).
pub fn tag_of(nid: &str) -> String {
    nid.to_string()
}

/// The group of the toasts of one phone: `kin-` and its short ID, so they can be listed and removed together.
pub fn group_of(device_short_id: &str) -> String {
    format!("kin-{device_short_id}")
}

fn is_short_id(s: &str) -> bool {
    s.len() == 8 && s.bytes().all(|b| matches!(b, b'A'..=b'Z' | b'2'..=b'7'))
}

/// Reads the arguments of a click, or `None` for anything that is not exactly what [`ToastArgs::to_arguments`] writes.
pub fn parse(arguments: &str) -> Option<ToastArgs> {
    if arguments.chars().count() > MAX_ARGS_CHARS {
        return None;
    }
    let parts: Vec<&str> = arguments.split('&').collect();
    if parts.len() != 4 && parts.len() != 5 {
        return None;
    }
    const KEYS: [&str; 5] = ["kin", "d", "n", "a", "i"];
    let mut values = [""; 5];
    for (k, part) in parts.iter().enumerate() {
        let (key, value) = part.split_once('=')?;
        if key != KEYS[k] {
            return None;
        }
        values[k] = value;
    }
    let [version, device, nid, action, index] = values;
    if version != "1" || !is_short_id(device) || !is_hex32(nid) {
        return None;
    }
    let action = match action {
        "tap" => ToastAction::Tap,
        "reply" => ToastAction::Reply,
        "copy" => ToastAction::Copy,
        "btn" => ToastAction::Button,
        _ => return None,
    };
    if action.needs_index() != (parts.len() == 5) {
        return None;
    }
    let index = if action.needs_index() {
        if index.is_empty() || !index.bytes().all(|b| b.is_ascii_digit()) || (index.len() > 1 && index.starts_with('0')) {
            return None;
        }
        Some(index.parse::<u8>().ok().filter(|i| *i <= 31)?)
    } else {
        None
    };
    Some(ToastArgs { device: device.to_string(), nid: nid.to_string(), action, index })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NID: &str = "54892d1f759b11f6762ff1bd98cda7df";

    #[test]
    fn what_is_written_is_read_back() {
        for (action, index) in
            [(ToastAction::Tap, None), (ToastAction::Copy, None), (ToastAction::Reply, Some(1)), (ToastAction::Button, Some(31))]
        {
            let args = ToastArgs::new("ABCD2345", NID, action, index);
            assert_eq!(parse(&args.to_arguments()), Some(args));
        }
        assert_eq!(ToastArgs::new("ABCD2345", NID, ToastAction::Tap, None).to_arguments(), format!("kin=1&d=ABCD2345&n={NID}&a=tap"));
        assert_eq!(group_of("ABCD2345"), "kin-ABCD2345");
        assert_eq!(tag_of(NID), NID);
    }

    #[test]
    fn anything_that_is_not_exactly_that_is_refused() {
        for bad in [
            "".to_string(),
            format!("kin=2&d=ABCD2345&n={NID}&a=tap"),
            format!("d=ABCD2345&kin=1&n={NID}&a=tap"),
            format!("kin=1&d=ABCD2345&n={NID}&a=tap&x=1"),
            format!("kin=1&d=ABCD2345&n={NID}&a=TAP"),
            format!("kin=1&d=ABCD2345&n={NID}&a=reply"),
            format!("kin=1&d=ABCD2345&n={NID}&a=tap&i=1"),
            format!("kin=1&d=ABCD2345&n={NID}&a=btn&i=32"),
            format!("kin=1&d=ABCD2345&n={NID}&a=btn&i=01"),
            format!("kin=1&d=ABCD2345&n={NID}&a=btn&i=-1"),
            format!("kin=1&d=ABCD2345&n={NID}&a=btn&i=%2B1"),
            format!("kin=1&d=ABCD2345&n={NID}&a=btn&i=\u{ff11}"),
            format!("kin=1&d=abcd2345&n={NID}&a=tap"),
            format!("kin=1&d=ABCD2341&n={NID}&a=tap"),
            format!("kin=1&d=ABCD2345&n={}&a=tap", NID.to_uppercase()),
            format!("kin=1&d=ABCD2345&n={NID}&a=%74ap"),
            format!("kin=1&d=ABCD2345&n={NID}&a=tap&"),
            format!(" kin=1&d=ABCD2345&n={NID}&a=tap"),
            format!("kin=1&d=ABCD2345&n={NID}&a=tap&x={}", "y".repeat(200)),
            "https://example.com/?kin=1".to_string(),
        ] {
            assert_eq!(parse(&bad), None, "{bad:?}");
        }
    }
}
