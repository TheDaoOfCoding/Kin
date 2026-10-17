//! A `link` from the phone (milestone 2B.1, protocol.md "Links: `link`"): the PC opens it in the default browser, or offers to.
//!
//! * **"Open links from phone automatically"** (a setting, on by default): the link opens at once (`ShellExecute` of a validated http or https URL
//!   only, `links::ShellOpener`) and a toast says "Opened link from phone" with a **Copy** button.
//! * The setting off: the toast says "Link from phone" with **Open** and **Copy**, and nothing opens.
//! * **A link with a warning** (`links::link_address`: a look-alike or international host, an IP address, a host a browser refuses) is *never* opened by
//!   itself, whatever the setting: the toast shows the real address with a warning sign, and Open and Copy.
//! * **A fullscreen game, a presentation, quiet time, or a locked or away user** (`quiet::UserState::holds_back_windows`): the link is *not*
//!   opened even with the setting on. A browser that appears over a game steals its focus, and one that opens on a locked PC lands where nobody
//!   sees it. The toast then has Open and Copy, and the history keeps the link.
//!
//! Either way the link is stored in the history (like a text) and never goes to the clipboard by itself. The URL is never logged: a log line has
//! its length and what was done with it.
use crate::core::{Core, Event, lock, short};
use crate::encoding::unix_ms;
use crate::links::link_address;
use crate::quiet::UserState;
use crate::sync::ToastTarget;

/// What to do with a link that arrived.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkDecision {
    /// Open it now.
    Open,
    /// Show a toast with Open and Copy. `why` is for the log.
    Offer { why: &'static str },
}

/// The one decision: the address, the setting, and what the user is doing. (`protocol`-independent; unit tested without a game.)
pub fn decide(open_automatically: bool, state: UserState, warn: bool) -> LinkDecision {
    if warn {
        LinkDecision::Offer { why: "the address has a warning" }
    } else if !open_automatically {
        LinkDecision::Offer { why: "the setting is off" }
    } else if state == UserState::NotPresent {
        LinkDecision::Offer { why: "the PC is locked or the user is away" }
    } else if state.holds_back_windows() {
        LinkDecision::Offer { why: "the user is in a fullscreen program, a presentation or quiet time" }
    } else {
        LinkDecision::Open
    }
}

/// What a toast for a link says: the title (if any) and the real address under it, the way a browser reads it, with a warning sign in front when
/// there is a warning (the title is not to be trusted, the address is what the user needs to see).
pub fn toast_body(title: Option<&str>, url: &str) -> String {
    let host = link_address(url).shown();
    match title {
        Some(t) => format!("{t}\n{host}"),
        None => host,
    }
}

impl Core {
    /// A `link` from the phone (already acked): stored in the history, then opened or offered (see the module comment).
    pub async fn receive_link(&self, device_id: &str, id: &str, url: String, title: Option<String>, source_app: Option<String>) {
        // The ID is the message's, so a repeat (a retry after a lost `ack`) is the same row and is not shown again: the receiver acked it from its
        // memory of IDs and never gets here twice, and the history refuses a second row with the same ID.
        if let Err(e) = self.db.insert_link_item(
            id,
            device_id,
            "in",
            "done",
            unix_ms() as i64,
            None,
            &url,
            title.as_deref(),
            source_app.as_deref(),
            None,
        ) {
            log::debug!("the link {id} is not added to the history again: {e}");
        }
        let open_automatically = lock(&self.settings).open_links_automatically;
        let state = self.user_state().state();
        let decision = decide(open_automatically, state, link_address(&url).warn);
        let mut opened = false;
        let mut note = String::new();
        if decision == LinkDecision::Open {
            let (opener, target) = (self.opener.clone(), url.clone());
            // `ShellExecute` can take a moment (it may start the browser): not on the connection's task.
            match tokio::task::spawn_blocking(move || opener.open(&target)).await {
                Ok(Ok(())) => opened = true,
                Ok(Err(e)) => note = format!("opening it failed: {e}"),
                Err(e) => note = format!("opening it failed: {e}"),
            }
        }
        log::info!(
            "link {id} from device {}: {} characters, {}{}",
            short(device_id),
            url.chars().count(),
            match (opened, decision) {
                (true, _) => "opened in the default browser".to_string(),
                (false, LinkDecision::Offer { why }) => format!("not opened ({why}; user state {}), offered in a toast", state.name()),
                (false, LinkDecision::Open) => "not opened, offered in a toast".to_string(),
            },
            if note.is_empty() { String::new() } else { format!(" ({note})") }
        );
        let toast_id = self.next_toast_id.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        let body = toast_body(title.as_deref(), &url);
        self.register_toast_target(toast_id, if opened { ToastTarget::LinkOpened(url) } else { ToastTarget::LinkOffer(url) });
        self.emit(Event::LinkToast {
            id: toast_id,
            title: if opened { "Opened link from phone" } else { "Link from phone" }.to_string(),
            body,
            opened,
        });
        self.emit(Event::StateChanged);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_opens_when_the_setting_is_on_and_the_user_is_free() {
        for free in [UserState::AcceptsNotifications, UserState::RunningWindowsStoreApp, UserState::Unknown] {
            assert_eq!(decide(true, free, false), LinkDecision::Open, "{free:?}");
        }
    }

    #[test]
    fn a_link_never_opens_over_a_fullscreen_program_a_presentation_or_quiet_time() {
        for held in [UserState::Busy, UserState::RunningD3dFullScreen, UserState::PresentationMode, UserState::QuietTime] {
            assert!(matches!(decide(true, held, false), LinkDecision::Offer { .. }), "{held:?}");
        }
    }

    #[test]
    fn a_link_is_offered_not_opened_while_the_pc_is_locked_or_the_user_is_away() {
        assert_eq!(decide(true, UserState::NotPresent, false), LinkDecision::Offer { why: "the PC is locked or the user is away" });
        assert_eq!(decide(false, UserState::NotPresent, false), LinkDecision::Offer { why: "the setting is off" });
    }

    #[test]
    fn with_the_setting_off_nothing_opens_whatever_the_state() {
        for any in [UserState::AcceptsNotifications, UserState::Busy, UserState::Unknown] {
            assert!(matches!(decide(false, any, false), LinkDecision::Offer { .. }), "{any:?}");
        }
    }

    #[test]
    fn a_link_with_a_warning_is_never_opened_by_itself() {
        for state in [UserState::AcceptsNotifications, UserState::NotPresent, UserState::Unknown, UserState::Busy] {
            for setting in [true, false] {
                assert_eq!(decide(setting, state, true), LinkDecision::Offer { why: "the address has a warning" }, "{setting} {state:?}");
            }
        }
    }

    #[test]
    fn the_toast_shows_the_real_host_not_the_title_alone() {
        assert_eq!(toast_body(Some("A video"), "https://youtu.be/abc"), "A video\nyoutu.be");
        assert_eq!(toast_body(None, "https://youtu.be/abc"), "youtu.be");
        assert_eq!(toast_body(None, "https://example.com:8443/x"), "example.com:8443");
        // A title can pose as another site: the host is still there on the line below it.
        assert!(toast_body(Some("paypal.com - sign in"), "https://evil.example/").ends_with("\nevil.example"));
    }

    #[test]
    fn the_toast_shows_what_a_browser_opens_with_a_warning_sign() {
        // B2: a homograph, a full-width host, an IDN and an odd IP are shown the way the browser reads them, marked.
        assert_eq!(toast_body(None, "https://\u{430}\u{440}\u{440}\u{4cf}\u{435}.com/"), "\u{26a0} xn--80ak6aa92e.com");
        assert_eq!(toast_body(None, "https://\u{ff47}\u{ff4f}\u{ff4f}\u{ff47}\u{ff4c}\u{ff45}.com/"), "\u{26a0} google.com");
        assert_eq!(toast_body(Some("t"), "https://b\u{fc}cher.example/"), "t\n\u{26a0} xn--bcher-kva.example");
        assert_eq!(toast_body(None, "https://0x7f.1/"), "\u{26a0} 127.0.0.1");
        assert_eq!(toast_body(None, "http://[::ffff:127.0.0.1]/"), "\u{26a0} [::ffff:7f00:1]");
    }

    #[test]
    fn the_backslash_trick_is_not_openable_so_it_never_reaches_the_toast() {
        // B1: a browser goes to evil.example; the rule refuses the URL, and the address function reads it like a browser if it is asked anyway.
        let url = r"https://evil.example\@google.com";
        assert!(!crate::links::is_link_url(url));
        assert!(!crate::links::is_openable_url(url));
        assert_eq!(link_address(url), crate::links::LinkAddress { display: "evil.example".into(), warn: true });
    }
}
