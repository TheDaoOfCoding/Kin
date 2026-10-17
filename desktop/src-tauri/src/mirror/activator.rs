//! The COM class that Windows calls when someone clicks a button, types a reply or clicks the body of one of Kin's phone toasts (design 5.2, spike S1).
//!
//! The shell reads `CustomActivator` from `HKCU\Software\Classes\AppUserModelId\<AUMID>` (`registration.rs`), asks COM for that class, and calls
//! `INotificationActivationCallback::Activate(aumid, invokedArgs, inputs[], count)` with the `arguments` of the button and each input box as a key and
//! a value. If Kin is not running COM **starts** it (`Kin.exe --toast-activated -Embedding`); if it is, the call goes to the running process
//! (`REGCLS_MULTIPLEUSE`).
//!
//! * **The class is registered from a thread of its own** that joins the **MTA**, registers, and parks until Kin stops (spike gotcha 10): `Activate` then
//!   runs on an RPC thread with no message loop needed. Tauri's main thread is an STA and would need to pump messages.
//! * **`Activate` only hands off**: it copies the strings into an [`Activation`] and returns at once (the shell may wait for it).
//! * **Anything can call it**: any program of the same user can ask COM for this class. What the call may do is checked by the receiver (strict
//!   arguments, a paired phone that lists `notif`, a toast that exists, a limit), not here.
use std::ffi::c_void;
use std::sync::Arc;
use std::sync::mpsc;

use windows::Win32::Foundation::CLASS_E_NOAGGREGATION;
use windows::Win32::System::Com::{
    CLSCTX_LOCAL_SERVER, COINIT_MULTITHREADED, CoInitializeEx, CoRegisterClassObject, CoRevokeClassObject, IClassFactory,
    IClassFactory_Impl, REGCLS_MULTIPLEUSE,
};
use windows::Win32::UI::Notifications::{
    INotificationActivationCallback, INotificationActivationCallback_Impl, NOTIFICATION_USER_INPUT_DATA,
};
use windows::core::{BOOL, GUID, IUnknown, Interface, PCWSTR, Ref, Result, implement};

/// How an activation reached Kin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// The shell called the COM class.
    Com,
    /// The toast object's own `Activated` event (a click on a popup of a toast this process showed).
    InProcess,
    /// A developer command (`--dev-toast`): the same function, no Windows.
    Dev,
}

/// What a click said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Activation {
    pub source: Source,
    /// The button's `arguments` (or the toast's `launch`), as Windows passed them. Parsed strictly by `args::parse`.
    pub arguments: String,
    /// The input boxes: `(id, value)`. The reply box has the id `reply`.
    pub inputs: Vec<(String, String)>,
}

/// Where a call of the class goes. It must not block.
pub type Sink = Arc<dyn Fn(Activation) + Send + Sync>;

#[implement(INotificationActivationCallback)]
struct Activator {
    sink: Sink,
}

impl INotificationActivationCallback_Impl for Activator_Impl {
    fn Activate(
        &self,
        _appusermodelid: &PCWSTR,
        invokedargs: &PCWSTR,
        data: *const NOTIFICATION_USER_INPUT_DATA,
        count: u32,
    ) -> Result<()> {
        // SAFETY: the shell passes NUL-terminated strings and `count` entries at `data` (or null with a count of 0), valid for the call.
        let arguments = unsafe { invokedargs.to_string() }.unwrap_or_default();
        let mut inputs = Vec::new();
        if !data.is_null() {
            for d in unsafe { std::slice::from_raw_parts(data, (count as usize).min(16)) } {
                let key = unsafe { d.Key.to_string() }.unwrap_or_default();
                let value = unsafe { d.Value.to_string() }.unwrap_or_default();
                inputs.push((key, value));
            }
        }
        (self.sink)(Activation { source: Source::Com, arguments, inputs });
        Ok(())
    }
}

#[implement(IClassFactory)]
struct Factory {
    sink: Sink,
}

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(&self, outer: Ref<'_, IUnknown>, riid: *const GUID, out: *mut *mut c_void) -> Result<()> {
        if outer.is_some() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let activator: IUnknown = Activator { sink: self.sink.clone() }.into();
        // SAFETY: `riid` and `out` are what COM passed for this call.
        unsafe { activator.query(riid, out).ok() }
    }

    fn LockServer(&self, _lock: BOOL) -> Result<()> {
        Ok(())
    }
}

/// The registered class. Dropping it (or [`ClassRegistration::stop`]) revokes it.
pub struct ClassRegistration {
    stop: Option<mpsc::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ClassRegistration {
    /// Registers the class `clsid` with COM from a thread of its own (MTA). `sink` gets every call.
    pub fn start(clsid: u128, sink: Sink) -> std::result::Result<ClassRegistration, String> {
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::channel::<std::result::Result<(), String>>();
        let thread = std::thread::Builder::new()
            .name("toast-activator".into())
            .spawn(move || {
                // SAFETY: COM for this thread, in the multithreaded apartment; the class is registered and revoked on this same thread.
                unsafe {
                    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                    let registered = (|| -> Result<u32> {
                        let factory: IUnknown = IClassFactory::from(Factory { sink }).cast()?;
                        CoRegisterClassObject(&GUID::from_u128(clsid), &factory, CLSCTX_LOCAL_SERVER, REGCLS_MULTIPLEUSE)
                    })();
                    match registered {
                        Ok(cookie) => {
                            let _ = ready_tx.send(Ok(()));
                            // Parked until Kin stops (the sender is dropped or sends).
                            let _ = stop_rx.recv();
                            let _ = CoRevokeClassObject(cookie);
                        }
                        Err(e) => {
                            let _ = ready_tx.send(Err(format!("the activator class can't be registered: {e}")));
                        }
                    }
                }
            })
            .map_err(|e| format!("the activator thread can't start: {e}"))?;
        ready_rx.recv().map_err(|_| "the activator thread ended at once".to_string())??;
        Ok(ClassRegistration { stop: Some(stop_tx), thread: Some(thread) })
    }

    pub fn stop(&mut self) {
        if let Some(tx) = self.stop.take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for ClassRegistration {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Calls a registered class the way the shell does for a click, from this process: `CoCreateInstance(clsid, LOCAL_SERVER)` and `Activate`. For tests and
/// the developer tool; it needs the class registered in some process (this one counts).
pub fn call_class(clsid: u128, aumid: &str, arguments: &str, inputs: &[(&str, &str)]) -> std::result::Result<(), String> {
    use windows::Win32::System::Com::CoCreateInstance;
    use windows::core::HSTRING;
    // SAFETY: COM for this thread (MTA); the wide strings live until the call returns; `Activate` is called as the shell calls it.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let keys: Vec<HSTRING> = inputs.iter().map(|(k, _)| HSTRING::from(*k)).collect();
        let values: Vec<HSTRING> = inputs.iter().map(|(_, v)| HSTRING::from(*v)).collect();
        let data: Vec<NOTIFICATION_USER_INPUT_DATA> = keys
            .iter()
            .zip(&values)
            .map(|(k, v)| NOTIFICATION_USER_INPUT_DATA { Key: PCWSTR(k.as_ptr()), Value: PCWSTR(v.as_ptr()) })
            .collect();
        let callback: INotificationActivationCallback = CoCreateInstance(&GUID::from_u128(clsid), None, CLSCTX_LOCAL_SERVER)
            .map_err(|e| format!("CoCreateInstance of the activator failed: {e}"))?;
        callback.Activate(&HSTRING::from(aumid), &HSTRING::from(arguments), &data).map_err(|e| format!("Activate failed: {e}"))?;
    }
    Ok(())
}
