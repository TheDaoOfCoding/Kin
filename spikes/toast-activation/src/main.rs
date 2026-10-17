//! Spike S1 (2B.2): can an UNPACKAGED Win32 app show toasts under its own AUMID, registered only
//! under HKCU, and be activated through a COM class (`INotificationActivationCallback`)?
//!
//! One exe, several modes (see `usage`). Everything it registers is spike-only and `unregister`
//! removes it. It never shows a toast unless Windows says it accepts notifications (no
//! full-screen game, no presentation, no quiet time).

use std::ffi::c_void;
use std::io::Write;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use windows::core::{implement, Ref, Result, GUID, HSTRING, IUnknown, Interface, BOOL, PCWSTR};
use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::{DateTime, IReference, PropertyValue, TypedEventHandler};
use windows::UI::Notifications::{
    NotificationMirroring, ToastActivatedEventArgs, ToastDismissalReason, ToastDismissedEventArgs,
    ToastFailedEventArgs, ToastNotification, ToastNotificationManager,
};
use windows::Win32::Foundation::{CLASS_E_NOAGGREGATION, E_FAIL};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoRegisterClassObject, CoRevokeClassObject, IClassFactory,
    IClassFactory_Impl, CLSCTX_LOCAL_SERVER, COINIT_MULTITHREADED, REGCLS_MULTIPLEUSE,
};
use windows::Win32::UI::Notifications::{
    INotificationActivationCallback, INotificationActivationCallback_Impl,
    NOTIFICATION_USER_INPUT_DATA,
};
use windows::Win32::UI::Shell::SHQueryUserNotificationState;
use windows_registry::CURRENT_USER;

// Spike-only identities. NEVER the installed Kin's (`app.kinrelay.desktop`).
const AUMID: &str = "app.kinrelay.spike.toast";
const AUMID_UNREGISTERED: &str = "app.kinrelay.spike.toast.unregistered";
const CLSID: GUID = GUID::from_u128(0xd4f1a6b2_7c3e_4b58_9a10_5e6c0b7a21f3);
const CLSID_STR: &str = "{D4F1A6B2-7C3E-4B58-9A10-5E6C0B7A21F3}";
const GROUP: &str = "spike-phone";

static ACTIVATIONS: AtomicU32 = AtomicU32::new(0);
static START: Mutex<Option<Instant>> = Mutex::new(None);

// ---------------------------------------------------------------- logging

fn out_dir() -> std::path::PathBuf {
    // <spike>/out, found from the exe's place: <spike>/out/<anything>/toast-spike.exe
    let exe = std::env::current_exe().unwrap();
    let mut p = exe.parent().unwrap().to_path_buf();
    while !p.ends_with("out") {
        if !p.pop() {
            return std::env::temp_dir();
        }
    }
    p
}

fn log(msg: &str) {
    let ms = START.lock().unwrap().map(|s| s.elapsed().as_millis()).unwrap_or(0);
    let line = format!("[{:>6}ms pid {:>6}] {}", ms, std::process::id(), msg);
    println!("{line}");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(out_dir().join("spike.log")) {
        let _ = writeln!(f, "{line}");
    }
}

// ---------------------------------------------------------------- the COM class

#[implement(INotificationActivationCallback)]
struct Activator;

impl INotificationActivationCallback_Impl for Activator_Impl {
    fn Activate(
        &self,
        appusermodelid: &PCWSTR,
        invokedargs: &PCWSTR,
        data: *const NOTIFICATION_USER_INPUT_DATA,
        count: u32,
    ) -> Result<()> {
        let aumid = unsafe { appusermodelid.to_string() }.unwrap_or_default();
        let args = unsafe { invokedargs.to_string() }.unwrap_or_default();
        let mut inputs = String::new();
        if !data.is_null() {
            for d in unsafe { std::slice::from_raw_parts(data, count as usize) } {
                let k = unsafe { d.Key.to_string() }.unwrap_or_default();
                let v = unsafe { d.Value.to_string() }.unwrap_or_default();
                inputs.push_str(&format!(" input[{k}]={v:?}"));
            }
        }
        log(&format!("ACTIVATE aumid={aumid:?} args={args:?} count={count}{inputs}"));
        ACTIVATIONS.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(&self, outer: Ref<'_, IUnknown>, riid: *const GUID, out: *mut *mut c_void) -> Result<()> {
        if outer.is_some() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let a: IUnknown = Activator.into();
        unsafe { a.query(riid, out).ok() }
    }
    fn LockServer(&self, _lock: BOOL) -> Result<()> {
        Ok(())
    }
}

fn register_class() -> Result<u32> {
    let f: IUnknown = IClassFactory::from(Factory).cast()?;
    unsafe { CoRegisterClassObject(&CLSID, &f, CLSCTX_LOCAL_SERVER, REGCLS_MULTIPLEUSE) }
}

// ---------------------------------------------------------------- registration (HKCU only)

fn register(exe: &str, icon: &str) -> Result<()> {
    // 1. the COM class: a quoted path (so a space in "Program Files" works) + our own switch
    let clsid_key = CURRENT_USER.create(format!(r"Software\Classes\CLSID\{CLSID_STR}"))?;
    clsid_key.set_string("", "Kin toast spike activator")?;
    let ls = CURRENT_USER.create(format!(r"Software\Classes\CLSID\{CLSID_STR}\LocalServer32"))?;
    ls.set_string("", format!("\"{exe}\" --toast-activated"))?;
    // 2. the identity: no shortcut, only these values
    let id = CURRENT_USER.create(format!(r"Software\Classes\AppUserModelId\{AUMID}"))?;
    id.set_string("DisplayName", "Kin spike (toast activation test)")?;
    id.set_string("IconUri", icon)?;
    id.set_string("CustomActivator", CLSID_STR)?;
    log(&format!("registered AUMID {AUMID} + CLSID {CLSID_STR} -> {exe}"));
    Ok(())
}

fn unregister() {
    for p in [
        format!(r"Software\Classes\CLSID\{CLSID_STR}"),
        format!(r"Software\Classes\AppUserModelId\{AUMID}"),
        // Windows creates these itself the first time it shows a toast for an AUMID
        format!(r"Software\Microsoft\Windows\CurrentVersion\Notifications\Settings\{AUMID}"),
        format!(r"Software\Microsoft\Windows\CurrentVersion\Notifications\Settings\{AUMID_UNREGISTERED}"),
        format!(r"Software\Microsoft\Windows\CurrentVersion\PushNotifications\Backup\{AUMID}"),
        format!(r"Software\Microsoft\Windows\CurrentVersion\PushNotifications\Backup\{AUMID_UNREGISTERED}"),
    ] {
        match CURRENT_USER.remove_tree(&p) {
            Ok(()) => log(&format!("removed HKCU\\{p}")),
            Err(e) => log(&format!("HKCU\\{p}: not removed ({})", e.message().trim())),
        }
    }
}

// ---------------------------------------------------------------- toasts

/// Only show anything if Windows accepts notifications (no D3D full screen, presentation, quiet time).
fn guard() -> bool {
    match unsafe { SHQueryUserNotificationState() } {
        Ok(s) if s.0 == 5 => true,
        Ok(s) => {
            log(&format!("REFUSED: SHQueryUserNotificationState={} (not QUNS_ACCEPTS_NOTIFICATIONS)", s.0));
            false
        }
        Err(e) => {
            log(&format!("REFUSED: SHQueryUserNotificationState failed: {e}"));
            false
        }
    }
}

fn toast_xml(tag: &str, body: &str, with_input: bool) -> Result<XmlDocument> {
    let actions = if with_input {
        format!(
            r#"<actions>
  <input id="reply" type="text" placeHolderContent="Reply"/>
  <action content="Send" hint-inputId="reply" activationType="background" arguments="kin=1&amp;n={tag}&amp;a=reply"/>
  <action content="Copy" activationType="background" arguments="kin=1&amp;n={tag}&amp;a=copy"/>
</actions>"#
        )
    } else {
        String::new()
    };
    let xml = format!(
        r#"<toast launch="kin=1&amp;n={tag}&amp;a=tap" activationType="background">
<visual><binding template="ToastGeneric"><text>Kin spike {tag}</text><text>{body}</text><text placement="attribution">spike, safe to ignore</text></binding></visual>
{actions}
<audio silent="true"/>
</toast>"#
    );
    let doc = XmlDocument::new()?;
    doc.LoadXml(&HSTRING::from(xml))?;
    Ok(doc)
}

fn filetime_in(secs: u64) -> DateTime {
    let unix = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() + secs;
    DateTime { UniversalTime: ((unix + 11_644_473_600) * 10_000_000) as i64 }
}

struct Opts {
    suppress: bool,
    expire_secs: Option<u64>,
}

fn make_toast(tag: &str, body: &str, with_input: bool, o: &Opts, events: &Arc<Events>) -> Result<ToastNotification> {
    let t = ToastNotification::CreateToastNotification(&toast_xml(tag, body, with_input)?)?;
    t.SetTag(&HSTRING::from(tag))?;
    t.SetGroup(&HSTRING::from(GROUP))?;
    t.SetSuppressPopup(o.suppress)?;
    t.SetExpiresOnReboot(true)?;
    t.SetNotificationMirroring(NotificationMirroring::Disabled)?;
    if let Some(s) = o.expire_secs {
        let r: IReference<DateTime> = PropertyValue::CreateDateTime(filetime_in(s))?.cast()?;
        t.SetExpirationTime(&r)?;
    }
    let (ev, tg, label) = (events.clone(), tag.to_string(), body.to_string());
    t.Dismissed(&TypedEventHandler::<ToastNotification, ToastDismissedEventArgs>::new(move |_, a| {
        let reason = match a.as_ref().map(|a| a.Reason()) {
            Some(Ok(r)) if r == ToastDismissalReason::UserCanceled => "UserCanceled",
            Some(Ok(r)) if r == ToastDismissalReason::ApplicationHidden => "ApplicationHidden",
            Some(Ok(r)) if r == ToastDismissalReason::TimedOut => "TimedOut",
            _ => "?",
        };
        log(&format!("EVENT Dismissed tag={tg} ({label}) reason={reason}"));
        ev.dismissed.lock().unwrap().push((tg.clone(), label.clone(), reason.to_string()));
        Ok(())
    }))?;
    let tg = tag.to_string();
    t.Activated(&TypedEventHandler::<ToastNotification, windows::core::IInspectable>::new(move |_, a| {
        let args = a
            .as_ref()
            .and_then(|a| a.cast::<ToastActivatedEventArgs>().ok())
            .and_then(|a| a.Arguments().ok())
            .map(|s| s.to_string())
            .unwrap_or_default();
        log(&format!("EVENT Activated (in-process) tag={tg} args={args:?}"));
        Ok(())
    }))?;
    let tg = tag.to_string();
    t.Failed(&TypedEventHandler::<ToastNotification, ToastFailedEventArgs>::new(move |_, a| {
        let code = a.as_ref().and_then(|a| a.ErrorCode().ok()).map(|c| c.0).unwrap_or(0);
        log(&format!("EVENT Failed tag={tg} hr=0x{code:08X}"));
        Ok(())
    }))?;
    Ok(t)
}

#[derive(Default)]
struct Events {
    dismissed: Mutex<Vec<(String, String, String)>>,
}

impl Events {
    fn reasons_for(&self, tag: &str) -> Vec<String> {
        self.dismissed.lock().unwrap().iter().filter(|d| d.0 == tag).map(|d| format!("{}:{}", d.1, d.2)).collect()
    }
}

/// (count, tags with their body text) of what Windows holds for an AUMID
fn history(aumid: &str) -> Vec<(String, String, bool)> {
    let h = match ToastNotificationManager::History().and_then(|h| h.GetHistoryWithId(&HSTRING::from(aumid))) {
        Ok(h) => h,
        Err(e) => {
            log(&format!("history({aumid}) failed: {e}"));
            return vec![];
        }
    };
    h.into_iter()
        .map(|t| {
            let tag = t.Tag().map(|s| s.to_string()).unwrap_or_default();
            let group = t.Group().map(|s| s.to_string()).unwrap_or_default();
            let xml = t.Content().and_then(|c| c.GetXml()).map(|s| s.to_string()).unwrap_or_default();
            let body = xml
                .split("<text>")
                .nth(2)
                .and_then(|s| s.split("</text>").next())
                .unwrap_or("")
                .to_string();
            (format!("{group}/{tag}"), body, xml.contains(r#"type="text""#))
        })
        .collect()
}

fn dump_history(label: &str, aumid: &str) -> usize {
    let h = history(aumid);
    log(&format!(
        "HISTORY[{label}] {aumid}: {} toast(s): {}",
        h.len(),
        h.iter().map(|(t, b, i)| format!("{t} body={b:?} input={i}")).collect::<Vec<_>>().join(" | ")
    ));
    h.len()
}

fn sleep(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

fn wait_for(mut cond: impl FnMut() -> bool, max: Duration) -> Option<Duration> {
    let t = Instant::now();
    while t.elapsed() < max {
        if cond() {
            return Some(t.elapsed());
        }
        sleep(250);
    }
    None
}

// ---------------------------------------------------------------- the scenario (in-process)

fn scenario() -> Result<()> {
    let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID))?;
    let hist = ToastNotificationManager::History()?;
    let ev = Arc::new(Events::default());
    let quiet = Opts { suppress: true, expire_secs: None };
    let aumid = HSTRING::from(AUMID);
    let group = HSTRING::from(GROUP);

    // Q1: is the setting visible? (NotificationSetting::Enabled == 0)
    log(&format!("notifier.Setting() = {:?}", notifier.Setting()));

    // CONTROL: a toast under an AUMID that is NOT registered
    {
        let n = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID_UNREGISTERED))?;
        let t = make_toast("ctl", "control, unregistered AUMID", false, &quiet, &ev)?;
        let r = n.Show(&t);
        log(&format!("CONTROL show under unregistered AUMID -> {r:?}; setting={:?}", n.Setting()));
        sleep(800);
        dump_history("control", AUMID_UNREGISTERED);
        // the real discriminator: does a POPUP for an unregistered AUMID stay up like a registered one (~6 s)?
        if guard() {
            let t = make_toast("ctl2", "control popup, unregistered AUMID", false, &Opts { suppress: false, expire_secs: None }, &ev)?;
            let shown = Instant::now();
            n.Show(&t)?;
            let w = wait_for(|| !ev.reasons_for("ctl2").is_empty(), Duration::from_secs(12));
            log(&format!(
                "CONTROL popup under UNREGISTERED AUMID: Dismissed after {:?}s -> {:?}",
                w.map(|_| shown.elapsed().as_secs_f32()),
                ev.reasons_for("ctl2")
            ));
        }
        let _ = hist.ClearWithId(&HSTRING::from(AUMID_UNREGISTERED));
    }

    // 1. toast with input + buttons, suppressed popup: lands in the history (Action Center)
    let t1 = make_toast("t1", "v1 (input + buttons)", true, &quiet, &ev)?;
    notifier.Show(&t1)?;
    sleep(800);
    dump_history("after show t1", AUMID);

    // 2. update: same tag + group, new text
    let t1b = make_toast("t1", "v2 (updated in place)", true, &quiet, &ev)?;
    notifier.Show(&t1b)?;
    sleep(1500);
    let n = dump_history("after update t1", AUMID);
    log(&format!("UPDATE: {n} toast with tag t1 after two Show() calls (1 = replaced, 2 = duplicated); old object's Dismissed events: {:?}", ev.reasons_for("t1")));

    // 3. ExpirationTime: a toast that must disappear by itself (watched on a thread)
    let t5 = make_toast("t5", "expires after 15 s", false, &Opts { suppress: true, expire_secs: Some(15) }, &ev)?;
    notifier.Show(&t5)?;
    let t5_shown = Instant::now();
    let watcher = std::thread::spawn(move || {
        let gone = wait_for(|| !history(AUMID).iter().any(|h| h.0.ends_with("/t5")), Duration::from_secs(120));
        match gone {
            Some(_) => log(&format!("EXPIRY: t5 (ExpirationTime=+15s) gone from history after {:.1}s", t5_shown.elapsed().as_secs_f32())),
            None => log("EXPIRY: t5 still in history after 120s (ExpirationTime NOT honoured)"),
        }
    });

    // 4. Hide() = "the app hid it"
    let t3 = make_toast("t3", "will be hidden by the app", false, &quiet, &ev)?;
    notifier.Show(&t3)?;
    sleep(600);
    notifier.Hide(&t3)?;
    sleep(1500);
    let present = history(AUMID).iter().any(|h| h.0.ends_with("/t3"));
    log(&format!("HIDE: after notifier.Hide(t3): in history={present}; Dismissed={:?}", ev.reasons_for("t3")));

    // 5. RemoveGroupedTagWithId = what notif.remove will use
    let t4 = make_toast("t4", "will be removed from history", false, &quiet, &ev)?;
    notifier.Show(&t4)?;
    sleep(600);
    hist.RemoveGroupedTagWithId(&HSTRING::from("t4"), &group, &aumid)?;
    sleep(1500);
    let present = history(AUMID).iter().any(|h| h.0.ends_with("/t4"));
    log(&format!("REMOVE: after RemoveGroupedTagWithId(t4): in history={present}; Dismissed={:?}", ev.reasons_for("t4")));

    // 6. A real popup (not suppressed), silent: replace it while visible, then let it time out into the Action Center
    if guard() {
        let o = Opts { suppress: false, expire_secs: None };
        let t2a = make_toast("t2", "popup v1", true, &o, &ev)?;
        notifier.Show(&t2a)?;
        sleep(1500);
        let t2b = make_toast("t2", "popup v2 replaced while visible", true, &o, &ev)?;
        let shown = Instant::now();
        notifier.Show(&t2b)?;
        sleep(300);
        log(&format!("POPUP REPLACE: events of the old popup object right after replacing it: {:?}", ev.reasons_for("t2")));
        let waited = wait_for(|| ev.reasons_for("t2").iter().any(|r| r.starts_with("popup v2")), Duration::from_secs(30));
        log(&format!(
            "POPUP: v2 Dismissed after {:?}s; all events for tag t2 -> {:?}",
            waited.map(|_| shown.elapsed().as_secs_f32()),
            ev.reasons_for("t2")
        ));
        sleep(500);
        dump_history("after popup timed out", AUMID);
    }

    let _ = watcher.join();
    dump_history("end of scenario", AUMID);
    // t1 (suppressed, in the Action Center) and t2 (a popup that moved there) are deliberately kept
    Ok(())
}

// ---------------------------------------------------------------- modes

fn usage() {
    println!("modes: register | unregister | serve <max-seconds> <expected-activations> | client <args> [key=value ...]\n       history | clear | --toast-activated");
}

fn main() -> Result<()> {
    *START.lock().unwrap() = Some(Instant::now());
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };
    let a: Vec<String> = std::env::args().collect();
    let mode = a.get(1).map(String::as_str).unwrap_or("");
    log(&format!("start mode={mode:?} full command line={:?}", a));
    match mode {
        "register" => {
            let exe = std::env::current_exe().unwrap();
            let icon = out_dir().join("icon.png");
            register(&exe.to_string_lossy(), &icon.to_string_lossy())?;
        }
        "unregister" => unregister(),
        "history" => {
            dump_history("fresh process", AUMID);
        }
        "manual-show" => {
            // For the human checklist: m1 = a popup, m2 = Action Center only; both with a reply box + Send + Copy.
            // stay = 0: this process exits at once (so a click can only work through COM start-up);
            // stay > 0: the class object stays registered for that many seconds (the "app is running" case).
            let stay: u64 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
            if !guard() {
                std::process::exit(3);
            }
            let cookie = if stay > 0 { Some(register_class()?) } else { None };
            let ev = Arc::new(Events::default());
            let n = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID))?;
            for (tag, suppress) in [("m1", false), ("m2", true)] {
                let o = Opts { suppress, expire_secs: Some(1800) };
                n.Show(&make_toast(tag, "manual check: type a word, press Send", true, &o, &ev)?)?;
            }
            log(&format!("MANUAL-SHOW m1 (popup) and m2 (Action Center only) shown; process stays {stay} s"));
            if let Some(c) = cookie {
                sleep(stay * 1000);
                log("MANUAL-SHOW process ending");
                unsafe { CoRevokeClassObject(c)? };
            }
        }
        "remove" => {
            // what notif.remove / notif.sync will do after a restart: remove a toast made by another (dead) process
            let tag = a.get(2).cloned().unwrap_or_default();
            ToastNotificationManager::History()?.RemoveGroupedTagWithId(&HSTRING::from(tag.as_str()), &HSTRING::from(GROUP), &HSTRING::from(AUMID))?;
            dump_history(&format!("after RemoveGroupedTagWithId({tag}) from a fresh process"), AUMID);
        }
        "clear" => {
            let h = ToastNotificationManager::History()?;
            h.ClearWithId(&HSTRING::from(AUMID))?;
            h.ClearWithId(&HSTRING::from(AUMID_UNREGISTERED))?;
            dump_history("after clear", AUMID);
        }
        "serve" => {
            let max: u64 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(60);
            let expect: u32 = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
            if !guard() {
                std::process::exit(3);
            }
            let cookie = register_class()?;
            log("class object registered (REGCLS_MULTIPLEUSE); app is RUNNING");
            let r = scenario();
            log(&format!("SCENARIO_DONE {r:?}"));
            wait_for(|| ACTIVATIONS.load(Ordering::SeqCst) >= expect, Duration::from_secs(max));
            sleep(500);
            log(&format!("serve ending, activations received={}", ACTIVATIONS.load(Ordering::SeqCst)));
            unsafe { CoRevokeClassObject(cookie)? };
        }
        "--toast-activated" => {
            // COM started us because a client asked for the class and nobody had registered it.
            let cookie = register_class()?;
            log("COM-LAUNCHED: class object registered, waiting for the call");
            wait_for(|| ACTIVATIONS.load(Ordering::SeqCst) >= 1, Duration::from_secs(30));
            sleep(1000);
            log(&format!("COM-LAUNCHED process ending, activations received={}", ACTIVATIONS.load(Ordering::SeqCst)));
            unsafe { CoRevokeClassObject(cookie)? };
        }
        "client" => {
            // What the shell does for a click: CoCreateInstance(CustomActivator CLSID) then Activate().
            let args = a.get(2).cloned().unwrap_or_default();
            let kv: Vec<(HSTRING, HSTRING)> = a[3.min(a.len())..]
                .iter()
                .filter_map(|s| s.split_once('='))
                .map(|(k, v)| (HSTRING::from(k), HSTRING::from(v)))
                .collect();
            let data: Vec<NOTIFICATION_USER_INPUT_DATA> = kv
                .iter()
                .map(|(k, v)| NOTIFICATION_USER_INPUT_DATA { Key: PCWSTR(k.as_ptr()), Value: PCWSTR(v.as_ptr()) })
                .collect();
            let t = Instant::now();
            let cb: INotificationActivationCallback = unsafe { CoCreateInstance(&CLSID, None, CLSCTX_LOCAL_SERVER) }
                .map_err(|e| {
                    log(&format!("CLIENT CoCreateInstance FAILED: {e}"));
                    e
                })?;
            log(&format!("CLIENT CoCreateInstance ok in {} ms", t.elapsed().as_millis()));
            let r = unsafe { cb.Activate(&HSTRING::from(AUMID), &HSTRING::from(args), &data) };
            log(&format!("CLIENT Activate -> {r:?} (total {} ms)", t.elapsed().as_millis()));
            r.map_err(|_| windows::core::Error::from(E_FAIL))?;
        }
        _ => usage(),
    }
    Ok(())
}
