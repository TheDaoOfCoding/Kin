//! Tells the app when Windows wakes from sleep (2B.4 review round 1, item 8).
//!
//! The pause is a wall-clock comparison, but what **notices** that it ended is a timer, and a timer does not count the time a PC spent asleep: after a resume the grey
//! dot could stay up until the old wake-up time came round. So a resume wakes the presence pump and redraws the tray at once.
//!
//! `PowerRegisterSuspendResumeNotification` with a callback: no window, no message loop, no permission. Windows calls the callback on a thread of its own with
//! `PBT_APMRESUMEAUTOMATIC` (every wake) and `PBT_APMRESUMESUSPEND` (a wake the user caused); either one means "look again". The registration lives as long as the
//! process (it is made once, at start), so the callback and its context are never freed.
use std::sync::Arc;

/// Calls `on_resume` each time Windows resumes from sleep or hibernation. Nothing happens off Windows. A failed registration is logged and the app carries on:
/// the pause then ends at the next timer, as before.
pub fn watch(on_resume: Arc<dyn Fn() + Send + Sync>) {
    #[cfg(windows)]
    imp::watch(on_resume);
    #[cfg(not(windows))]
    let _ = on_resume;
}

/// Whether a `PBT_*` code from the callback is a resume.
pub fn is_resume_event(code: u32) -> bool {
    const PBT_APMRESUMESUSPEND: u32 = 7;
    const PBT_APMRESUMEAUTOMATIC: u32 = 18;
    code == PBT_APMRESUMEAUTOMATIC || code == PBT_APMRESUMESUSPEND
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::sync::Arc;

    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Power::{DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS, PowerRegisterSuspendResumeNotification};
    use windows::Win32::UI::WindowsAndMessaging::DEVICE_NOTIFY_CALLBACK;

    type Callback = Arc<dyn Fn() + Send + Sync>;

    unsafe extern "system" fn on_event(context: *const c_void, code: u32, _setting: *const c_void) -> u32 {
        if super::is_resume_event(code) && !context.is_null() {
            // SAFETY: `context` is the `Box<Callback>` leaked by `watch`, never freed and only read.
            let callback = unsafe { &*(context as *const Callback) };
            callback();
        }
        0
    }

    pub fn watch(on_resume: Callback) {
        // Leaked on purpose: the registration lasts as long as the process.
        let context = Box::into_raw(Box::new(on_resume)) as *mut c_void;
        let params = Box::into_raw(Box::new(DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS { Callback: Some(on_event), Context: context }));
        let mut registration: *mut c_void = std::ptr::null_mut();
        // SAFETY: `params` points to a valid, never-freed `DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS`; `registration` is a valid out-pointer.
        let result =
            unsafe { PowerRegisterSuspendResumeNotification(DEVICE_NOTIFY_CALLBACK, HANDLE(params as *mut c_void), &mut registration) };
        if result.0 != 0 {
            log::warn!(
                "watching for the PC waking from sleep failed (error {}): a pause that ends in its sleep is noticed at the next timer",
                result.0
            );
        } else {
            log::info!("watching for the PC waking from sleep");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_two_resume_codes_count() {
        assert!(is_resume_event(18) && is_resume_event(7));
        // PBT_APMSUSPEND (4), PBT_APMPOWERSTATUSCHANGE (10), PBT_POWERSETTINGCHANGE (32787): not a wake.
        assert!(!is_resume_event(4) && !is_resume_event(10) && !is_resume_event(32787));
    }
}
