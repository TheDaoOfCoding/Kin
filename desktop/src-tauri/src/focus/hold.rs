//! Which of Kin's own toasts are held while the phone is in Do Not Disturb (design 3.6). Pure: `protocol/alerts.json` `holdToasts[]` drives it.
//!
//! With the setting "Hold Kin's own toasts while my phone is in Do Not Disturb" on (default on) and a `dnd` word in the phone's `presence`, Kin does not show **its own
//! optional** toasts: the low-battery nudge, and the informational toasts that are not the answer to something the user did. **Not held:** the result of a user action (a
//! send, a ring, "Copied"), a mirrored phone notification (the phone already marks a post `silent` when its own filter blocked it and lets through what the user's
//! exceptions allow, so a blunt rule on the PC would be worse), the clipboard-clear of 2B.2 and a link offer.
use crate::presence::wire::Dnd;

/// The kind of a toast, for this decision only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    /// "Pixel 8 is at 14% and not charging".
    LowBattery,
    /// An informational toast that no action of the user asked for ("Sent 2 queued items to Pixel 8").
    Info,
    /// The answer to something the user did.
    Result,
    /// A phone notification shown on the PC.
    Mirrored,
    /// The 2B.2 "Code copied, it leaves the clipboard in a minute" and its clear.
    ClipboardClear,
    /// A link offered by the phone.
    Link,
}

impl ToastKind {
    /// The word of the vectors (`holdToasts[].kind`).
    pub fn from_word(word: &str) -> Option<ToastKind> {
        Some(match word {
            "low_battery" => ToastKind::LowBattery,
            "info" => ToastKind::Info,
            "result" => ToastKind::Result,
            "mirrored" => ToastKind::Mirrored,
            "clipboard_clear" => ToastKind::ClipboardClear,
            "link" => ToastKind::Link,
            _ => return None,
        })
    }
}

/// Whether the toast is held now. `dnd`: the phone's Do Not Disturb word, `None` when it is off or not known (the phone's `dnd` is forgotten when its connection closes).
pub fn is_held(dnd: Option<Dnd>, setting: bool, kind: ToastKind) -> bool {
    setting && dnd.is_some() && matches!(kind, ToastKind::LowBattery | ToastKind::Info)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_two_optional_kinds_are_held_and_only_with_the_setting_and_a_dnd_word() {
        for kind in [ToastKind::LowBattery, ToastKind::Info] {
            assert!(is_held(Some(Dnd::Priority), true, kind));
            assert!(is_held(Some(Dnd::Total), true, kind));
            assert!(!is_held(None, true, kind), "no dnd");
            assert!(!is_held(Some(Dnd::Alarms), false, kind), "setting off");
        }
        for kind in [ToastKind::Result, ToastKind::Mirrored, ToastKind::ClipboardClear, ToastKind::Link] {
            assert!(!is_held(Some(Dnd::Total), true, kind), "{kind:?}");
        }
    }
}
