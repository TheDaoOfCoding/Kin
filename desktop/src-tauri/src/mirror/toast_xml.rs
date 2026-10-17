//! A `notif.post` as the XML of a Windows toast (design 5.3). Pure: no Windows.
//!
//! **Every value from the phone goes through one escaping function** and is only ever placed as the text or the attribute of a node of a small
//! document tree (`Node`), never spliced into a string of markup: a title like `</text><action .../>` is just text. The result is also cut to fit
//! 4,800 bytes (the documented limit of a toast is 5 KB): text is shortened, older messages go, the structure never changes.
//!
//! **What a toast holds**, at most: the title, two body lines, the attribution (`<app> · <phone>`), the app's icon and up to five buttons (the reply
//! box with Send, `Copy <code>`, then the phone's own). A `sensitive` or `redacted` post, or the user's "hide content" setting, shows the app and a
//! fixed sentence and no buttons. `launch` and the `arguments` of the buttons are identifiers only (`args.rs`).
use std::path::Path;

use super::args::{ToastAction, ToastArgs};
use super::wire::Post;

/// The most bytes of XML (UTF-8) a toast may have. The limit of Windows is 5 KB, and it is not documented in bytes or in characters.
pub const MAX_XML_BYTES: usize = 4800;
/// Windows shows at most five buttons.
pub const MAX_BUTTONS: usize = 5;
pub const PRIVATE_BODY: &str = "Private notification";
pub const REDACTED_BODY: &str = "Code hidden by Android";
/// The placeholder of the reply box when the phone gave no hint: "Reply" and an invisible mark, so that a toast read back from Windows' history after a
/// restart of Kin can tell "no hint" (the phone's label is empty) from a phone whose hint really is "Reply".
pub const NO_HINT_PLACEHOLDER: &str = "Reply\u{200b}";

/// Where a toast comes from and how it is drawn (the PC's own settings and the phone's name).
pub struct ToastContext<'a> {
    pub phone_name: &'a str,
    /// The phone's 8-character short ID (for the arguments).
    pub device: &'a str,
    /// The cached icon of the app, if there is one.
    pub icon: Option<&'a Path>,
    /// "Hide notification content": every post is shown in the private form.
    pub hide_content: bool,
    /// "Play a sound for phone notifications": off by default, the phone has rung already.
    pub sound: bool,
    /// The fallback when the activator doesn't work: only text, no box and no buttons.
    pub text_only: bool,
}

/// What the toast offers, so the ledger can check a click against it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Offered {
    /// The reply box: the phone's index of the reply action and the hint that was shown.
    pub reply: Option<(u8, String)>,
    pub copy_code: Option<String>,
    /// The phone's buttons that are on the toast: index and label.
    pub buttons: Vec<(u8, String)>,
}

pub struct Built {
    pub xml: String,
    pub offered: Offered,
    /// The text of the first line, for tests and the status of the shown count. Never logged.
    pub private: bool,
}

// ----- a tiny document tree ------------------------------------------------------------------------------------------

struct Node {
    name: &'static str,
    attrs: Vec<(&'static str, String)>,
    text: Option<String>,
    children: Vec<Node>,
}

impl Node {
    fn new(name: &'static str) -> Node {
        Node { name, attrs: Vec::new(), text: None, children: Vec::new() }
    }

    fn attr(mut self, key: &'static str, value: impl Into<String>) -> Node {
        self.attrs.push((key, value.into()));
        self
    }

    fn text(mut self, text: impl Into<String>) -> Node {
        self.text = Some(text.into());
        self
    }

    fn child(mut self, child: Node) -> Node {
        self.children.push(child);
        self
    }

    fn write(&self, out: &mut String) {
        out.push('<');
        out.push_str(self.name);
        for (k, v) in &self.attrs {
            out.push(' ');
            out.push_str(k);
            out.push_str("=\"");
            escape_into(v, out, true);
            out.push('"');
        }
        if self.text.is_none() && self.children.is_empty() {
            out.push_str("/>");
            return;
        }
        out.push('>');
        if let Some(t) = &self.text {
            escape_into(t, out, false);
        }
        for c in &self.children {
            c.write(out);
        }
        out.push_str("</");
        out.push_str(self.name);
        out.push('>');
    }
}

/// Whether a character may appear in an XML 1.0 document at all.
fn xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}')
}

/// A label (the hint of the reply box, the name of a button) in the form it has **on the toast**, which is also the form that goes back to the phone in a
/// `notif.action` and that the phone compares: U+2028, U+2029 and the characters XML can't hold (U+FFFE, U+FFFF) become a space, and the ends are
/// trimmed. Windows hands the toast back from its history with those characters as spaces, so a label kept as the phone wrote it would not match
/// after a restart of Kin. The phone applies the same function to its own label before it compares (protocol.md, `notif.action`; `toastLabel[]`).
pub fn toast_label(s: &str) -> String {
    let mapped: String = s.chars().map(|c| if matches!(c, '\u{2028}' | '\u{2029}' | '\u{fffe}' | '\u{ffff}') { ' ' } else { c }).collect();
    mapped.trim_matches(' ').to_string()
}

/// The one place text becomes markup: `& < >` always, quotes inside an attribute, and a character XML can't hold (or that draws as a line break
/// of its own, U+2028 and U+2029) becomes a space.
fn escape_into(s: &str, out: &mut String, attribute: bool) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if attribute => out.push_str("&quot;"),
            '\'' if attribute => out.push_str("&apos;"),
            '\u{2028}' | '\u{2029}' => out.push(' '),
            '\n' | '\r' | '\t' if attribute => out.push(' '),
            c if !xml_char(c) => out.push(' '),
            c => out.push(c),
        }
    }
}

// ----- what a toast says ----------------------------------------------------------------------------------------------

/// What the toast says, before any cutting: the title and up to two body lines.
fn lines_of(post: &Post, private: bool) -> (String, Vec<String>) {
    if private {
        return (post.app.clone(), vec![PRIVATE_BODY.to_string()]);
    }
    let title = post.title.clone().or_else(|| post.conversation.clone()).unwrap_or_else(|| post.app.clone());
    if post.redacted {
        return (title, vec![REDACTED_BODY.to_string()]);
    }
    let mut body = Vec::new();
    if !post.messages.is_empty() {
        // The last two messages. A group shows who wrote each; a chat of two shows "You: " for the user's own and nothing for the other.
        let skip = post.messages.len().saturating_sub(2);
        for m in &post.messages[skip..] {
            body.push(match (&m.sender, post.group) {
                (Some(s), true) => format!("{s}: {}", m.text),
                (Some(_), false) => m.text.clone(),
                (None, _) => format!("You: {}", m.text),
            });
        }
    } else if let Some(text) = &post.big_text {
        // The long form replaces the short one when it is longer.
        let longer = post.text.as_ref().is_none_or(|t| text.chars().count() > t.chars().count());
        body.push(if longer { text.clone() } else { post.text.clone().unwrap_or_default() });
    } else if let Some(text) = &post.text {
        body.push(text.clone());
    } else {
        body.extend(post.lines.iter().take(2).cloned());
    }
    (title, body)
}

fn cut(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('\u{2026}');
    out
}

/// The toast for a post. `private` is true for a `sensitive` post and for the user's "hide content" setting.
pub fn build(post: &Post, ctx: &ToastContext<'_>) -> Built {
    let private = post.sensitive || ctx.hide_content;
    // Shorten the text step by step until the XML fits. The structure and the buttons stay as they are.
    let caps: [(usize, usize); 9] = [(500, 150), (300, 150), (200, 120), (150, 100), (100, 80), (70, 60), (50, 40), (30, 25), (15, 15)];
    let mut last = None;
    for (body_cap, title_cap) in caps {
        let built = build_with(post, ctx, private, body_cap, title_cap);
        if built.xml.len() <= MAX_XML_BYTES {
            return built;
        }
        last = Some(built);
    }
    last.expect("the list of caps is not empty")
}

fn build_with(post: &Post, ctx: &ToastContext<'_>, private: bool, body_cap: usize, title_cap: usize) -> Built {
    let (title, body) = lines_of(post, private);
    let tap = ToastArgs::new(ctx.device, &post.nid, ToastAction::Tap, None).to_arguments();
    let mut binding = Node::new("binding").attr("template", "ToastGeneric");
    if let Some(icon) = ctx.icon {
        binding =
            binding.child(Node::new("image").attr("placement", "appLogoOverride").attr("hint-crop", "circle").attr("src", file_uri(icon)));
    }
    binding = binding.child(Node::new("text").text(cut(&title, title_cap)));
    for line in &body {
        binding = binding.child(Node::new("text").text(cut(line, body_cap)));
    }
    binding = binding.child(Node::new("text").attr("placement", "attribution").text(format!("{} \u{b7} {}", post.app, ctx.phone_name)));

    let mut toast = Node::new("toast").attr("launch", tap).attr("activationType", "background");
    toast = toast.child(Node::new("visual").child(binding));

    let mut offered = Offered::default();
    // A sensitive or redacted post, the user's "hide content" setting and the text-only fallback show no box and no buttons.
    if !private && !post.redacted && !ctx.text_only {
        let mut actions = Node::new("actions");
        let mut count = 0;
        if let Some(r) = &post.reply {
            let hint = toast_label(r.hint.as_deref().unwrap_or_default());
            let placeholder = if hint.is_empty() { NO_HINT_PLACEHOLDER.to_string() } else { cut(&hint, 60) };
            actions =
                actions.child(Node::new("input").attr("id", "reply").attr("type", "text").attr("placeHolderContent", placeholder)).child(
                    Node::new("action")
                        .attr("content", "Send")
                        .attr("hint-inputId", "reply")
                        .attr("activationType", "background")
                        .attr("arguments", ToastArgs::new(ctx.device, &post.nid, ToastAction::Reply, Some(r.i)).to_arguments()),
                );
            offered.reply = Some((r.i, hint));
            count += 1;
        }
        if let Some(code) = &post.code {
            actions = actions.child(
                Node::new("action")
                    .attr("content", format!("Copy {code}"))
                    .attr("activationType", "background")
                    .attr("arguments", ToastArgs::new(ctx.device, &post.nid, ToastAction::Copy, None).to_arguments()),
            );
            offered.copy_code = Some(code.clone());
            count += 1;
        }
        for a in &post.actions {
            if count >= MAX_BUTTONS {
                break;
            }
            let label = toast_label(&a.label);
            if label.is_empty() {
                continue;
            }
            actions = actions.child(
                Node::new("action")
                    .attr("content", label.clone())
                    .attr("activationType", "background")
                    .attr("arguments", ToastArgs::new(ctx.device, &post.nid, ToastAction::Button, Some(a.i)).to_arguments()),
            );
            offered.buttons.push((a.i, label));
            count += 1;
        }
        if count > 0 {
            toast = toast.child(actions);
        }
    }
    if !ctx.sound {
        toast = toast.child(Node::new("audio").attr("silent", "true"));
    }
    let mut xml = String::new();
    toast.write(&mut xml);
    Built { xml, offered, private }
}

// ----- reading a toast back from Windows' history ---------------------------------------------------------------------------

/// The attributes of every `<name ...>` element of a piece of XML, entities decoded. A small scanner for the toasts this module writes (and that Windows
/// hands back): it respects quotes, so a `>` inside a value doesn't end the tag.
fn elements(xml: &str, name: &str) -> Vec<Vec<(String, String)>> {
    let open = format!("<{name}");
    let bytes = xml.as_bytes();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(found) = xml[from..].find(&open) {
        let mut i = from + found + open.len();
        from = i;
        // `<action` must be followed by whitespace, `/` or `>` (not `<actions`).
        if !matches!(bytes.get(i), Some(b' ' | b'\t' | b'\r' | b'\n' | b'/' | b'>')) {
            continue;
        }
        let mut attrs = Vec::new();
        loop {
            while bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
                i += 1;
            }
            if matches!(bytes.get(i), None | Some(b'>') | Some(b'/')) {
                break;
            }
            let start = i;
            while bytes.get(i).is_some_and(|b| !b.is_ascii_whitespace() && !matches!(b, b'=' | b'>' | b'/')) {
                i += 1;
            }
            let key = xml[start..i].to_string();
            while bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
                i += 1;
            }
            if bytes.get(i) != Some(&b'=') {
                break;
            }
            i += 1;
            while bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
                i += 1;
            }
            let Some(&quote) = bytes.get(i).filter(|q| matches!(q, b'"' | b'\'')) else {
                break;
            };
            i += 1;
            let value_start = i;
            while bytes.get(i).is_some_and(|b| *b != quote) {
                i += 1;
            }
            attrs.push((key, unescape(&xml[value_start..i.min(xml.len())])));
            i += 1;
        }
        out.push(attrs);
        from = from.max(i.min(xml.len()));
    }
    out
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

fn attr<'a>(attrs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

/// What a toast of `nid` offers, read back from its XML (Windows keeps it in the history across a restart of Kin): the reply box, the code to copy and the
/// phone's buttons. Only elements whose arguments are well-formed and name this `nid` count.
pub fn offered_from_xml(xml: &str, nid: &str) -> Offered {
    let mut offered = Offered::default();
    let hint = elements(xml, "input")
        .iter()
        .find(|a| attr(a, "id") == Some("reply"))
        .and_then(|a| attr(a, "placeHolderContent").map(str::to_string));
    for a in elements(xml, "action") {
        let (Some(args), Some(content)) = (attr(&a, "arguments").and_then(super::args::parse), attr(&a, "content")) else {
            continue;
        };
        if args.nid != nid {
            continue;
        }
        match (args.action, args.index) {
            (ToastAction::Reply, Some(i)) => {
                let hint = hint.clone().unwrap_or_default();
                offered.reply = Some((i, if hint == NO_HINT_PLACEHOLDER { String::new() } else { hint }));
            }
            (ToastAction::Copy, _) => offered.copy_code = content.strip_prefix("Copy ").map(str::to_string),
            (ToastAction::Button, Some(i)) => offered.buttons.push((i, content.to_string())),
            _ => {}
        }
    }
    offered
}

/// `file:///C:/Users/.../x.png` for a path of the icon cache. The name of an icon is hex, but the folder is the user's, so it is percent-encoded.
fn file_uri(path: &Path) -> String {
    let mut out = String::from("file:///");
    for c in path.to_string_lossy().replace('\\', "/").chars() {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '.' | '_' | '~' | '/' | ':' => out.push(c),
            c => {
                let mut buf = [0u8; 4];
                for b in c.encode_utf8(&mut buf).bytes() {
                    out.push_str(&format!("%{b:02X}"));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mirror::wire::{ChatLine, PhoneAction, Reply};

    const NID: &str = "54892d1f759b11f6762ff1bd98cda7df";

    fn post() -> Post {
        Post {
            nid: NID.into(),
            app: "Chat".into(),
            icon_hash: None,
            title: Some("Alice".into()),
            text: Some("Are you coming tonight?".into()),
            big_text: None,
            sub_text: None,
            conversation: None,
            group: false,
            messages: vec![],
            lines: vec![],
            importance: 3,
            silent: false,
            sensitive: false,
            redacted: false,
            code: None,
            reply: None,
            actions: vec![],
        }
    }

    fn ctx() -> ToastContext<'static> {
        ToastContext { phone_name: "Pixel 8", device: "ABCD2345", icon: None, hide_content: false, sound: false, text_only: false }
    }

    #[test]
    fn a_plain_toast_has_a_title_a_line_an_attribution_and_is_silent() {
        let b = build(&post(), &ctx());
        assert_eq!(
            b.xml,
            format!(
                "<toast launch=\"kin=1&amp;d=ABCD2345&amp;n={NID}&amp;a=tap\" activationType=\"background\"><visual><binding template=\"ToastGeneric\">\
                 <text>Alice</text><text>Are you coming tonight?</text><text placement=\"attribution\">Chat \u{b7} Pixel 8</text></binding></visual>\
                 <audio silent=\"true\"/></toast>"
            )
        );
        assert_eq!(b.offered, Offered::default());
    }

    #[test]
    fn the_reply_box_the_code_and_the_buttons_in_that_order_and_at_most_five() {
        let mut p = post();
        p.reply = Some(Reply { i: 1, hint: Some("Reply to Alice".into()) });
        p.code = Some("123456".into());
        p.actions = (0..3).map(|i| PhoneAction { i, label: format!("Act {i}") }).collect();
        let b = build(&p, &ctx());
        let send = b.xml.find("content=\"Send\"").unwrap();
        let copy = b.xml.find("content=\"Copy 123456\"").unwrap();
        let first = b.xml.find("content=\"Act 0\"").unwrap();
        assert!(send < copy && copy < first);
        assert!(b.xml.contains("<input id=\"reply\" type=\"text\" placeHolderContent=\"Reply to Alice\"/>"));
        assert!(b.xml.contains("hint-inputId=\"reply\""));
        assert_eq!(b.xml.matches("<action ").count(), 5, "Send, Copy and three buttons");
        assert_eq!(b.offered.reply, Some((1, "Reply to Alice".into())));
        assert_eq!(b.offered.copy_code.as_deref(), Some("123456"));
        assert_eq!(b.offered.buttons.len(), 3);
        // A fourth phone button would be a sixth: only five fit, in priority order.
        p.actions = (0..3).map(|i| PhoneAction { i, label: format!("Act {i}") }).collect();
        p.reply = None;
        p.code = None;
        assert_eq!(build(&p, &ctx()).xml.matches("<action ").count(), 3);
    }

    #[test]
    fn a_sensitive_post_the_hide_setting_and_the_fallback_show_no_content_and_no_buttons() {
        let mut p = post();
        p.reply = Some(Reply { i: 1, hint: None });
        p.code = Some("123456".into());
        p.actions = vec![PhoneAction { i: 0, label: "Mark as read".into() }];
        p.sensitive = true;
        let b = build(&p, &ctx());
        assert!(b.private && b.xml.contains(">Chat</text><text>Private notification</text>"));
        assert!(
            !b.xml.contains("Alice")
                && !b.xml.contains("tonight")
                && !b.xml.contains("123456")
                && !b.xml.contains("<action")
                && !b.xml.contains("<input")
        );
        p.sensitive = false;
        let mut c = ctx();
        c.hide_content = true;
        assert!(build(&p, &c).xml.contains("Private notification"));
        let mut c = ctx();
        c.text_only = true;
        let b = build(&p, &c);
        assert!(b.xml.contains("Alice") && !b.xml.contains("<action") && !b.xml.contains("<input"));
        assert_eq!(b.offered, Offered::default());
    }

    #[test]
    fn a_redacted_post_says_the_code_is_hidden_by_android() {
        let mut p = post();
        p.title = Some("Bank".into());
        p.text = None;
        p.redacted = true;
        p.code = Some("123456".into());
        p.reply = Some(Reply { i: 0, hint: None });
        let b = build(&p, &ctx());
        assert!(b.xml.contains("<text>Bank</text><text>Code hidden by Android</text>"));
        assert!(!b.xml.contains("<action") && !b.xml.contains("123456"));
    }

    #[test]
    fn messages_show_the_last_two_with_the_sender_in_a_group_and_you_for_the_users_own() {
        let mut p = post();
        p.text = None;
        p.messages = vec![
            ChatLine { sender: Some("Mum".into()), text: "one".into() },
            ChatLine { sender: Some("Mum".into()), text: "two".into() },
            ChatLine { sender: None, text: "three".into() },
        ];
        let b = build(&p, &ctx());
        assert!(b.xml.contains("<text>two</text><text>You: three</text>"), "{}", b.xml);
        p.group = true;
        assert!(build(&p, &ctx()).xml.contains("<text>Mum: two</text><text>You: three</text>"));
        p.conversation = Some("Family".into());
        p.title = None;
        assert!(build(&p, &ctx()).xml.contains("<text>Family</text>"), "the conversation is the title when there is none");
    }

    #[test]
    fn the_long_text_replaces_the_short_one_only_when_it_is_longer() {
        let mut p = post();
        p.big_text = Some("Are you coming tonight? Bring the dessert please.".into());
        assert!(build(&p, &ctx()).xml.contains("Bring the dessert"));
        p.big_text = Some("short".into());
        let x = build(&p, &ctx()).xml;
        assert!(x.contains("Are you coming tonight?") && !x.contains(">short<"));
        let mut p = post();
        p.text = None;
        p.lines = vec!["a".into(), "b".into(), "c".into()];
        assert!(build(&p, &ctx()).xml.contains("<text>a</text><text>b</text><text placement"), "two lines at most");
    }

    #[test]
    fn hostile_text_can_not_change_the_structure() {
        let mut p = post();
        p.title = Some("</text><action content=\"Pwn\" arguments=\"x\"/><text>".into());
        p.text = Some("a & b <b>bold</b> \"q\" 'q' \u{2028}x\u{2029}y \u{0}z \u{ffff}".into());
        p.app = "A\"><image src=\"x\"/>".into();
        let mut c = ctx();
        c.phone_name = "P\"><x/>";
        let b = build(&p, &c);
        // Parse what we made with a strict reader of the few things that matter: only the nodes we wrote are there.
        for bad in ["<action content=\"Pwn\"", "<b>", "<image src=\"x\"", "<x/>", "\u{2028}", "\u{2029}", "\u{0}", "\u{ffff}"] {
            assert!(!b.xml.contains(bad), "{bad:?} in {}", b.xml);
        }
        assert!(b.xml.contains("&lt;/text&gt;&lt;action content=\"Pwn\" arguments=\"x\"/&gt;&lt;text&gt;"));
        assert!(b.xml.contains("a &amp; b &lt;b&gt;bold&lt;/b&gt; \"q\" 'q'  x y"));
        assert_eq!(b.xml.matches("<text").count(), 3, "title, line and attribution");
        assert_eq!(b.xml.matches("<toast ").count(), 1);
    }

    #[test]
    fn the_xml_is_cut_to_fit_whatever_the_phone_sends() {
        let mut p = post();
        p.title = Some("<".repeat(150));
        p.text = Some("&".repeat(500));
        p.big_text = Some("<".repeat(1000));
        p.messages = (0..5).map(|_| ChatLine { sender: Some("&".repeat(60)), text: "<".repeat(300) }).collect();
        p.group = true;
        p.app = "<".repeat(40);
        p.reply = Some(Reply { i: 31, hint: Some("<".repeat(60)) });
        p.code = Some("123456".into());
        p.actions = (0..3).map(|i| PhoneAction { i, label: "&".repeat(30) }).collect();
        let mut c = ctx();
        let long_name = "<".repeat(64);
        c.phone_name = &long_name;
        let b = build(&p, &c);
        assert!(b.xml.len() <= MAX_XML_BYTES, "{} bytes", b.xml.len());
        assert!(b.xml.contains("<action") && b.xml.contains("<input"), "the buttons are kept, the text is what shrinks");
        // And a post that already fits is not touched.
        assert!(!build(&post(), &ctx()).xml.contains('\u{2026}'));
    }

    #[test]
    fn a_label_with_characters_windows_turns_into_spaces_is_the_same_before_and_after_a_restart() {
        assert_eq!(toast_label("To\u{2028}Alice\u{2029}"), "To Alice");
        assert_eq!(toast_label("a\u{fffe}\u{ffff}b"), "a  b");
        assert_eq!(toast_label("\u{2028}"), "");
        assert_eq!(toast_label("plain \u{200b}x"), "plain \u{200b}x");
        assert_eq!(toast_label(&toast_label("x\u{2028}y")), "x y", "idempotent");
        let mut p = post();
        p.reply = Some(Reply { i: 1, hint: Some("To\u{2028}Alice\u{ffff}".into()) });
        p.actions = vec![
            PhoneAction { i: 2, label: "Mu\u{2029}te".into() },
            PhoneAction { i: 3, label: "\u{2028}".into() },
            PhoneAction { i: 4, label: "Done".into() },
        ];
        let b = build(&p, &ctx());
        assert_eq!(b.offered.reply, Some((1, "To Alice".into())));
        assert_eq!(b.offered.buttons, [(2, "Mu te".to_string()), (4, "Done".to_string())], "a button with no visible name is not offered");
        // Windows keeps the XML the way this module wrote it, so reading it back gives exactly what the ledger held.
        assert_eq!(offered_from_xml(&b.xml, NID), b.offered);
    }

    #[test]
    fn what_a_toast_offers_is_read_back_from_its_xml() {
        let mut p = post();
        p.reply = Some(Reply { i: 4, hint: Some("Reply to \"Alice\" & co".into()) });
        p.code = Some("AB12-CD".into());
        p.actions = vec![PhoneAction { i: 2, label: "Mark <read>".into() }, PhoneAction { i: 7, label: "Mute".into() }];
        let b = build(&p, &ctx());
        assert_eq!(offered_from_xml(&b.xml, NID), b.offered);
        assert_eq!(
            offered_from_xml(&b.xml, "00000000000000000000000000000000"),
            Offered::default(),
            "another nid's toast offers nothing for this one"
        );
        // No hint: the invisible mark says so.
        p.reply = Some(Reply { i: 1, hint: None });
        let b = build(&p, &ctx());
        assert_eq!(b.offered.reply, Some((1, String::new())));
        assert_eq!(offered_from_xml(&b.xml, NID).reply, Some((1, String::new())));
        // A hint that is really "Reply" is a hint.
        p.reply = Some(Reply { i: 1, hint: Some("Reply".into()) });
        assert_eq!(offered_from_xml(&build(&p, &ctx()).xml, NID).reply, Some((1, "Reply".into())));
        // Single quotes and spaces around the equals sign, as another serializer may write them.
        let other = format!(
            "<toast><actions><input id='reply' placeHolderContent = 'Hi'/><action content='Send' arguments='kin=1&amp;d=ABCD2345&amp;n={NID}&amp;a=reply&amp;i=3'/></actions></toast>"
        );
        assert_eq!(offered_from_xml(&other, NID).reply, Some((3, "Hi".into())));
        assert_eq!(offered_from_xml("<toast><visual/></toast>", NID), Offered::default());
        assert_eq!(
            offered_from_xml("<action content=\"x", NID),
            Offered::default(),
            "a broken document is not an error, it offers nothing"
        );
    }

    #[test]
    fn the_icon_is_a_file_uri_and_the_sound_is_off_unless_asked_for() {
        let icon = Path::new("C:\\Users\\Jos\u{e9} B\\AppData\\Roaming\\Kin\\notif-icons\\0123456789abcdef0123456789abcdef.png");
        let mut c = ctx();
        c.icon = Some(icon);
        c.sound = true;
        let b = build(&post(), &c);
        assert!(b.xml.contains("<image placement=\"appLogoOverride\" hint-crop=\"circle\" src=\"file:///C:/Users/Jos%C3%A9%20B/AppData/Roaming/Kin/notif-icons/0123456789abcdef0123456789abcdef.png\"/>"), "{}", b.xml);
        assert!(!b.xml.contains("<audio"));
    }
}
