# Spike S1 (2B.2): toasts and COM activation for an unpackaged Kin

Run 2026-10-17 on Windows 11 Pro 10.0.26300 (the dev PC), by the research sub-agent. Code: [`spikes/toast-activation/`](../../spikes/toast-activation/) (own Cargo project, `windows` 0.62.2, own target dir; nothing in `desktop/`, `android/`, `extension/`, `protocol/`, `tools/` was touched). Design it answers: [phase-2b-2-design.md](../phases/phase-2b-2-design.md) section 5.2 and section 9 step 1.

## Verdict

**GO with the COM activator.** Everything that a script can check works. The one link no script can check (the shell itself calling our class when a person clicks) rests on Microsoft's documentation and on the same registration that WARP, Wondershare and MSI Center use on this PC. **A 1-minute human check is in the last section.** Until a person has done it, R21 stays open, and the desktop coder should build `toast_xml` so that the actions and the input box can be left out (the design's text-only fallback) with one switch.

| Capability | Verdict | Evidence (log lines are in [`evidence/full-run.log`](../../spikes/toast-activation/evidence/full-run.log)) |
|---|---|---|
| Show toasts under an own AUMID, registered only under HKCU, no shortcut, no MSIX | **Works** | `Show` ok, toast in `GetHistoryWithId`, popup stayed about 6.8 s (6.76 s in two earlier runs, whose logs were overwritten; the logged run's popup of this AUMID was replaced and stayed 22 s) and then moved to the Action Center. **Caveat:** an *unregistered* AUMID did exactly the same on this build (popup 6.51 s, stored in history), so this does not prove the registry values were read. What they add (name, icon, activator) is only visible to a person |
| The registered COM class resolves through `HKCU\...\LocalServer32` (path with a space, quoted) | **Works** | `CoCreateInstance` ok, 4 to 5 ms (running) and 376 to 499 ms (not running); the exe lived in `...\out\with space\` |
| `Activate` while the app is running: button args and reply text | **Works** | one `ACTIVATE aumid="app.kinrelay.spike.toast" args="kin=1&n=t1&a=reply" count=1 input[reply]="hello from the test client"`, a second one with `count=0` for a button; both reached the **same process** that showed the toasts (REGCLS_MULTIPLEUSE) |
| `Activate` while the app is **not** running | **Works** | no spike process alive, the client's `CoCreateInstance` made COM start the exe: `start mode="--toast-activated", "-Embedding"`, the class was registered 4 ms later, and `ACTIVATE ... input[reply]="hello cold start"` arrived; the process ended itself 1 s after |
| Toast still there after the process that showed it has gone (Action Center) | **Works** | a fresh process `GetHistoryWithId` listed both toasts with tag, group and the content XML, `<input type="text">` included |
| The same, with the Windows toast in the Action Center and a click | **Not provable without a person** | see "What this proves" |
| Update by tag + group | **Works** | `Show` twice with tag `t1`, group `spike-phone`: one toast in history, text of the second. **No `Dismissed` on the old object** (suppressed and visible-popup case) |
| `RemoveGroupedTagWithId` | **Works**, also **from a fresh process** (a toast made by a dead process) | `t1` gone, `t2` left. It raises **no** `Dismissed` |
| `ExpirationTime` | **Works** | `+15 s` toast gone from history after 14.6 s |
| `SuppressPopup` | **Works** | in history, and `Dismissed(TimedOut)` came **within 23 to 60 ms** instead of after about 6.6 s, so Windows treated it as never shown. Not seen with eyes |
| `Dismissed` reasons | `ApplicationHidden`: **works** (`notifier.Hide`). `TimedOut`: **works** (popup, about 6.5 to 6.8 s, then in the Action Center). `UserCanceled`: **not testable** (needs the X) | log lines `EVENT Dismissed ... reason=` |
| `NotificationMirroring`, `ExpiresOnReboot` | Settable without error; **effect not verified** | |
| App name and icon from `DisplayName` / `IconUri` | **Not verified** | a read-only UI Automation probe found no element with the toast's title in the top-level windows of the shell's processes; the probe was deleted |

## What this proves, and what only a click can

**How activation was tested without input.** `toast-spike.exe client <args> key=value` does what the shell does for a click: `CoCreateInstance(CLSID, CLSCTX_LOCAL_SERVER)` for the class named in the AUMID's `CustomActivator`, then `INotificationActivationCallback::Activate(aumid, invokedArgs, NOTIFICATION_USER_INPUT_DATA[])`. It is a separate process that reaches the class only through the registry, as the shell's process would.

**Proven:** the HKCU registration is valid; COM finds the class and starts the exe with the quoted command line (plus `-Embedding`); `Activate` receives the exact `arguments` string and each input as `Key`/`Value` (the reply box id `reply` came through as the key); it works with the app running (same process, 5 ms) and not running (0.4 to 0.5 s); the toasts, their tags, groups and content survive the process; update, removal, expiry, `Hide`, `SuppressPopup` do what the design assumes.

**Not proven (a person must click):**

1. That the shell calls **our** CLSID for a toast of this AUMID (it reads `CustomActivator` from `HKCU\Software\Classes\AppUserModelId\<aumid>`). Documented by Microsoft for the shortcut form (`ToastActivatorCLSID`) and used by three other products on this PC in the registry form; not exercised here.
2. What a real **button** puts in `invokedArgs` and a real **reply** in the user input (documented as the `arguments` attribute and the `<input id>`; the test client passed the same shapes).
3. Whether a click on the **popup** reaches the COM class only, or **also** the in-process `Activated` handler (design 5.2, "not known [U]"). The spike registered both and logs both.
4. `UserCanceled` from the X on a popup and on an Action Center entry (design 5.4's "known gap").
5. How the toast **looks** (name, icon, five buttons, wrapping) and that `NotificationMirroring` does anything.
6. Windows 10. Only this Windows 11 build was used.

## Exact registration recipe (all HKCU, written by the app at each start)

For Kin: AUMID `app.kinrelay.desktop.phone`, a fixed random CLSID of its own. The spike used `app.kinrelay.spike.toast` and `{D4F1A6B2-7C3E-4B58-9A10-5E6C0B7A21F3}`.

```
HKCU\Software\Classes\CLSID\{CLSID}                        (Default) = Kin phone notifications activator
HKCU\Software\Classes\CLSID\{CLSID}\LocalServer32          (Default) = "C:\Program Files\Kin\Kin.exe" --toast-activated
HKCU\Software\Classes\AppUserModelId\app.kinrelay.desktop.phone
        DisplayName      REG_SZ   Kin: phone notifications
        IconUri          REG_SZ   C:\Users\<user>\AppData\Roaming\Kin\icon.png
        CustomActivator  REG_SZ   {CLSID}
```

No Start Menu shortcut, no `ToastActivatorCLSID`. The `LocalServer32` value must be **quoted** (the spike's path contained a space) and COM appends `-Embedding`, so the process sees `Kin.exe --toast-activated -Embedding`.

**Cargo** (`desktop/src-tauri/Cargo.toml`): add to the existing `windows` features `UI_Notifications`, `Data_Xml_Dom`, `Foundation`, `Foundation_Collections`, `Win32_UI_Notifications` (`Win32_System_Com` and `Win32_UI_Shell` are already there). **And a direct dependency `windows-core = "=0.62.2"`** (see gotchas, the `implement` macro).

**The class** (`spikes/toast-activation/src/main.rs`, about 40 lines):

```rust
#[implement(INotificationActivationCallback)]
struct Activator;
impl INotificationActivationCallback_Impl for Activator_Impl {
    fn Activate(&self, appusermodelid: &PCWSTR, invokedargs: &PCWSTR,
                data: *const NOTIFICATION_USER_INPUT_DATA, count: u32) -> Result<()> {
        // read invokedargs; for each d in from_raw_parts(data, count): d.Key / d.Value (PCWSTR)
        // hand off to the async runtime and RETURN AT ONCE
        Ok(())
    }
}
#[implement(IClassFactory)]
struct Factory;
impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(&self, outer: Ref<'_, IUnknown>, riid: *const GUID, out: *mut *mut c_void) -> Result<()> {
        if outer.is_some() { return Err(CLASS_E_NOAGGREGATION.into()); }
        let a: IUnknown = Activator.into();
        unsafe { a.query(riid, out).ok() }
    }
    fn LockServer(&self, _: BOOL) -> Result<()> { Ok(()) }
}
// once the process is the primary instance, in an MTA thread:
let f: IUnknown = IClassFactory::from(Factory).cast()?;
let cookie = unsafe { CoRegisterClassObject(&CLSID, &f, CLSCTX_LOCAL_SERVER, REGCLS_MULTIPLEUSE)? };
```

**The toast:** `<toast launch="..." activationType="background">` with `<input id="reply" type="text" placeHolderContent="Reply"/>`, `<action ... hint-inputId="reply" activationType="background" arguments="..."/>` and `<audio silent="true"/>` (as in design 5.3), `CreateToastNotifierWithId(aumid)`, then `SetTag`, `SetGroup`, `SetSuppressPopup`, `SetExpiresOnReboot`, `SetExpirationTime(IReference<DateTime>)` (from `PropertyValue::CreateDateTime(..).cast()`; the time is 100 ns ticks since 1601), `SetNotificationMirroring(NotificationMirroring::Disabled)`.

## Gotchas (for the 2B.2 desktop coder)

1. **`#[implement]` needs `windows-core` as a direct dependency** (it expands to paths in a crate named `windows_core`; `windows::core::implement` alone fails with "could not find `windows_core`"). Pin it to the same version as `windows`.
2. **The enum is `NotificationMirroring::Disabled`**, not `Disallowed` (design 5.3 and T3 use the second name).
3. **Replacing a toast raises no `Dismissed` on the old object** (suppressed toast, and a visible popup replaced after 1.5 s: no event for the old one, ever). Design 5.4 assumed it does. The generation counter is still cheap insurance but is not needed for replacement. **`RemoveGroupedTagWithId` raises none either**, so the app's own removals cannot look like a `UserCanceled`. `ApplicationHidden` comes only from `notifier.Hide()`.
4. **With `SuppressPopup = true`, `Dismissed(TimedOut)` fires at once** (23 to 60 ms), not when "the popup timed out". The design only acts on `UserCanceled`, so this is harmless, but nothing may treat `TimedOut` as "now in the Action Center".
5. **Any AUMID string is accepted by `Show`** on this build, registered or not (same history, same 6.5 s popup). A missing registration therefore does **not** fail loudly: the toast is shown under a generic name and, presumably, with no activator. Only the first real click tells. Nothing can be inferred from `Show` returning `Ok`.
6. **COM adds `-Embedding`** after the registered arguments. A `--toast-activated` parser must accept it.
7. **Windows writes its own HKCU keys** for every AUMID that shows a toast: `Software\Microsoft\Windows\CurrentVersion\Notifications\Settings\<aumid>` at once, and `...\PushNotifications\Backup\<aumid>` **later and not predictably** (found after the first full run only, removed by the next run's clean-up; not there after the later runs, so Windows may add it minutes after the toasts, and a re-check some time after a test is worthwhile). A test that registers a throwaway AUMID must remove all of them (the spike's `unregister` does; the first version missed the second and left two keys, which a later run removed). Kin's uninstaller should remove its own `AppUserModelId` and `CLSID` keys and may leave the two OS-owned ones.
8. **`notifier.Setting()` returned "Element not found" (0x80070490) once**, before the first toast of a freshly registered AUMID, and `Enabled` otherwise. Never treat an error from it as fatal.
9. **COM start-up cost is 0.4 to 0.5 s** from `CoCreateInstance` to `Activate` returning. Kin as the COM-launched process must register its class object within the first moments; keep `Activate` to a hand-off.
10. **Apartment (not tested here, the spike has no Tauri):** the spike registered the class from an **MTA** thread, so `Activate` runs on an RPC thread with no pump. Tauri's main thread is STA; registering from there would need its message loop to dispatch the call. Do the registration on a **dedicated thread that does `CoInitializeEx(MTA)`, registers, then parks until shutdown** and revokes there. Test this inside Kin first.
11. **Single instance:** if Kin is running, the second `CoCreateInstance` is routed to the running instance's registered class (REGCLS_MULTIPLEUSE, seen in the log: both calls reached one pid) and no second process starts. A second process starts only if the class is **not** registered (the setting is off, or Kin is not running); then it is the COM-launched one and must register the class **and** do the hand-off, as design 5.2 says.
12. One popup stayed **22 s** (the visible-popup replacement test) against 6.5 to 6.8 s for every other one. Not explained (the cursor over it, or the replacement); not investigated.
13. The toast's XML as read back from history keeps the `<input>` and the buttons, which is what `notif.sync` needs after a restart.

## Recommendation for the 2B.2 desktop coder

1. **Go with the COM activator**, as in design 5.2: the registration recipe above, `--toast-activated` handled like the spike, one `on_activation(args, input)` fed by both the COM class and the in-process events.
2. Keep the **text-only fallback** one switch away until the human check (below) has passed once, and again if a Windows update ever changes it. Do not build a UI on "the activator is surely called".
3. Take the design corrections above into `phase-2b-2-desktop.md`: `Disabled`; replacement and removal raise no `Dismissed`; `SuppressPopup` raises `TimedOut` at once; `windows-core` as a direct dependency; `-Embedding`; the two OS-owned keys in test clean-up; the MTA registration thread.
4. The dev/test hygiene of design 5.10 works as written: this spike removed its toasts (`ClearWithId`) and every key in a `finally`, verified by a `reg query` of all of `HKCU\Software` for the AUMID name (0 matches).

## The 1-minute human check (5 steps, run it later, on the PC)

From `spikes\toast-activation`, in PowerShell (it needs the build once: `cargo build --release`; the script does it). Everything it does is spike-only and silent.

1. `.\manual-check.ps1 -Step Show` registers the spike keys and shows two toasts, **"Kin spike m1"** (a popup) and **"Kin spike m2"** (Action Center only), each with a reply box, **Send** and **Copy**. No process stays running afterwards.
2. Type a word in m1's box and click **Send** while the popup is up (about 7 s), then open the Action Center (`Win+N`), type a word in m2's box and click **Send**, and click **Copy** on one of them.
3. `.\manual-check.ps1 -Step Check` must show one `ACTIVATE aumid=... args="kin=1&n=m1&a=reply" ... input[reply]="<your word>"` per click, each from a `start mode="--toast-activated" ... "-Embedding"` process (COM started the app for the click). Look at the toasts too: the name should read **"Kin spike (toast activation test)"** with the icon.
4. `.\manual-check.ps1 -Step Show -Stay 60`, click **Send** on the popup and on m2 within 60 s, and press the **X** of one toast, then `-Step Check` again: the `ACTIVATE` lines now come from the process that showed the toasts; the lines `EVENT Activated (in-process)` say whether a popup click also reaches the in-process handler; `EVENT Dismissed ... reason=UserCanceled` is the X.
5. `.\manual-check.ps1 -Step Cleanup` removes the toasts and every spike key (it ends with `0 match(es) found`).

If step 3 shows no `ACTIVATE` for a click, the shell is not calling the registry-form activator on that Windows build: use the text-only fallback and say so in `phase-2b-2-desktop.md` and R21.

## Hygiene and cost

- **Registered and removed:** `HKCU\Software\Classes\CLSID\{D4F1A6B2-...}`, `...\AppUserModelId\app.kinrelay.spike.toast`, and the two keys Windows made for the spike's two AUMIDs (`Notifications\Settings\`, `PushNotifications\Backup\`). No Start Menu shortcut was made. The final `reg query HKCU\Software /s /f app.kinrelay.spike` found nothing, the `CLSID` key is gone, no `toast-spike` process remains, and the installed Kin's `app.kinrelay.desktop` was never touched (nor its registry, Run key, SendTo, Downloads or firewall).
- **Toasts:** all silent (`<audio silent="true"/>`), every one removed again (`ClearWithId`). The exe refuses to show anything unless `SHQueryUserNotificationState` says notifications are accepted (it did: 5), and the driver stops if an anti-cheat or game process is running (only the Steam launcher was). **About nine silent popups appeared in all**, more than hoped: five of them for the final results, four for an abandoned read-only UI Automation probe.
- **Runs:** one full run (after a first run that stopped at the spike's own `notifier.Setting()` bug, which still produced the two COM results), one more full run with the controls added, two probe runs, one smoke test of `manual-check.ps1`. No input was injected at any point; the probe only read accessibility names in shell processes' windows and was deleted.
