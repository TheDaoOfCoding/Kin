//! A developer tool for the phone notifications of a running Kin (2B.2), with no input injection:
//!
//! ```text
//! cargo run --example toast_probe -- history <aumid>                      what Windows holds for an app identity: tag, group, and the buttons of each toast
//! cargo run --example toast_probe -- clear <aumid>                        removes every toast of that identity
//! cargo run --example toast_probe -- activate <clsid> <aumid> <arguments> [key=value ...]
//!                                                                         does what the shell does for a click: COM creates the class, then `Activate`
//!                                                                         with the button's arguments and the input boxes (`reply=hello`)
//! cargo run --example toast_probe -- registry <aumid> <clsid>             which HKCU keys of that identity exist (including the two Windows makes)
//! ```
//!
//! It works on the identity it is given and never on another: refuse `app.kinrelay.desktop` and `app.kinrelay.desktop.phone` (the installed Kin's).
use kin_lib::mirror::activator::call_class;
use kin_lib::mirror::registration::RELEASE_AUMID;
use windows::UI::Notifications::ToastNotificationManager;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::core::HSTRING;

fn refuse_installed(aumid: &str) {
    if aumid == "app.kinrelay.desktop" || aumid == RELEASE_AUMID {
        eprintln!("refused: {aumid} is the installed Kin's identity");
        std::process::exit(3);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // SAFETY: COM for this thread, in the multithreaded apartment.
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    match args.first().map(String::as_str) {
        Some("history") => {
            let aumid = args.get(1).expect("an AUMID");
            refuse_installed(aumid);
            let list =
                ToastNotificationManager::History().and_then(|h| h.GetHistoryWithId(&HSTRING::from(aumid.as_str()))).expect("the history");
            let mut n = 0;
            for t in list {
                n += 1;
                let tag = t.Tag().map(|s| s.to_string()).unwrap_or_default();
                let group = t.Group().map(|s| s.to_string()).unwrap_or_default();
                let xml = t.Content().and_then(|c| c.GetXml()).map(|s| s.to_string()).unwrap_or_default();
                // Only the structure, never the text: the buttons by their `content` and whether there is an input box.
                let buttons = xml.matches("<action ").count();
                let input = xml.contains("<input ");
                let sensitive_form = xml.contains("Private notification");
                println!("toast tag={tag} group={group} bytes={} buttons={buttons} input={input} private={sensitive_form}", xml.len());
                if args.iter().any(|a| a == "--xml") {
                    println!("{xml}");
                }
            }
            println!("{n} toast(s) for {aumid}");
        }
        Some("clear") => {
            let aumid = args.get(1).expect("an AUMID");
            refuse_installed(aumid);
            ToastNotificationManager::History().and_then(|h| h.ClearWithId(&HSTRING::from(aumid.as_str()))).expect("clear");
            println!("cleared {aumid}");
        }
        Some("activate") => {
            let (clsid, aumid, arguments) =
                (args.get(1).expect("a CLSID"), args.get(2).expect("an AUMID"), args.get(3).expect("the arguments"));
            refuse_installed(aumid);
            let hex: String = clsid.chars().filter(char::is_ascii_hexdigit).collect();
            let clsid = u128::from_str_radix(&hex, 16).expect("a CLSID");
            let inputs: Vec<(&str, &str)> = args[4..].iter().filter_map(|kv| kv.split_once('=')).collect();
            let started = std::time::Instant::now();
            match call_class(clsid, aumid, arguments, &inputs) {
                Ok(()) => println!("Activate returned in {} ms", started.elapsed().as_millis()),
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
        Some("registry") => {
            let (aumid, clsid) = (args.get(1).expect("an AUMID"), args.get(2).expect("a CLSID"));
            refuse_installed(aumid);
            for key in [
                format!(r"Software\Classes\CLSID\{clsid}"),
                format!(r"Software\Classes\AppUserModelId\{aumid}"),
                format!(r"Software\Microsoft\Windows\CurrentVersion\Notifications\Settings\{aumid}"),
                format!(r"Software\Microsoft\Windows\CurrentVersion\PushNotifications\Backup\{aumid}"),
            ] {
                let output = std::process::Command::new("reg").args(["query", &format!(r"HKCU\{key}")]).output().expect("reg.exe");
                println!("{} {key}", if output.status.success() { "EXISTS " } else { "absent " });
            }
        }
        _ => eprintln!("usage: toast_probe history|clear|activate|registry ... (see the top of the file)"),
    }
}
