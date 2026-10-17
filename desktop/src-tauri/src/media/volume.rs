//! The PC's master volume behind a trait (protocol.md "Volume"): the real Core Audio endpoint ([`WinVolume`]) and a fake ([`FakeVolume`]) that remembers a number.
//! **The real `Set*` calls are reachable only through `guard::Mode::real_volume`**: a test run gets the fake (`for_mode`).
//!
//! Windows' own slider has steps (51 of 2 % on the dev PC). A `media.cmd` `volume` with a `level` sets the level as it is, a `delta` moves **at least one native
//! step** in its direction, a `mute` sets the mute flag. `protocol/media.json` (`volumeLevel[]`, `volumeSet[]`) pins the arithmetic.
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use windows::Win32::Media::Audio::Endpoints::{IAudioEndpointVolume, IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl};
use windows::Win32::Media::Audio::{AUDIO_VOLUME_NOTIFICATION_DATA, IMMDeviceEnumerator, MMDeviceEnumerator, eMultimedia, eRender};
use windows::Win32::System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize};
use windows::core::{GUID, implement};

use super::guard::Mode;
use super::wire::VolumeOp;

/// The event context Kin passes with every write, so its own changes can be told from the user's in the change notification.
pub const KIN_CONTEXT: GUID = GUID::from_u128(0x4b1e_7a52_93c1_4d0e_8f61_2a7c_5d9b_3e10);

/// What the master volume is now: a percentage and the mute flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VolumeState {
    pub level: u8,
    pub muted: bool,
}

/// Rounds `a / b` half up, for non-negative integers.
fn div_round(a: u64, b: u64) -> u64 {
    (2 * a + b) / (2 * b)
}

/// A scalar from 0 to 1 as a percentage.
pub fn level_of_scalar(scalar: f32) -> u8 {
    (scalar.clamp(0.0, 1.0) * 100.0).round() as u8
}

/// What a `volume` command does to a level (not to the mute flag): `Level` sets it, `Delta` moves **at least one native step** (the endpoint has `steps` steps, so
/// `steps - 1` intervals) in its direction and stops at both ends. `None` for a mute.
pub fn target_level(steps: u32, current: u8, op: VolumeOp) -> Option<u8> {
    match op {
        VolumeOp::Mute(_) => None,
        VolumeOp::Level(l) => Some(l.min(100)),
        VolumeOp::Delta(d) => {
            let n = u64::from(steps.saturating_sub(1).max(1));
            let step = div_round(u64::from(current) * n, 100);
            let moved = div_round(d.unsigned_abs() * n, 100).max(1);
            let next = if d > 0 { (step + moved).min(n) } else { step.saturating_sub(moved) };
            Some(div_round(next * 100, n).min(100) as u8)
        }
    }
}

pub trait VolumeEndpoint: Send + Sync {
    fn get(&self) -> Result<VolumeState, String>;
    /// How many steps Windows' slider has (51 on the dev PC).
    fn steps(&self) -> u32;
    fn set_level(&self, level: u8) -> Result<(), String>;
    fn set_mute(&self, muted: bool) -> Result<(), String>;
}

/// Does what a command says, and returns what the volume is after it.
pub fn apply(endpoint: &dyn VolumeEndpoint, op: VolumeOp) -> Result<VolumeState, String> {
    match op {
        VolumeOp::Mute(m) => endpoint.set_mute(m)?,
        other => {
            let now = endpoint.get()?;
            let level = target_level(endpoint.steps(), now.level, other).expect("a level command has a target");
            endpoint.set_level(level)?;
        }
    }
    endpoint.get()
}

// ----- the fake ---------------------------------------------------------------------------------------------------------

/// A volume that is a number in memory. A test run uses it, so a command from a phone can never change the PC's sound.
pub struct FakeVolume {
    state: Mutex<VolumeState>,
    steps: u32,
    writes: Mutex<Vec<VolumeOp>>,
}

impl FakeVolume {
    pub fn new(level: u8, steps: u32) -> FakeVolume {
        FakeVolume { state: Mutex::new(VolumeState { level, muted: false }), steps, writes: Mutex::new(Vec::new()) }
    }

    /// The writes it got, as commands (for tests).
    pub fn writes(&self) -> Vec<VolumeOp> {
        self.writes.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

impl VolumeEndpoint for FakeVolume {
    fn get(&self) -> Result<VolumeState, String> {
        Ok(*self.state.lock().unwrap_or_else(|p| p.into_inner()))
    }

    fn steps(&self) -> u32 {
        self.steps
    }

    fn set_level(&self, level: u8) -> Result<(), String> {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).level = level;
        self.writes.lock().unwrap_or_else(|p| p.into_inner()).push(VolumeOp::Level(level));
        Ok(())
    }

    fn set_mute(&self, muted: bool) -> Result<(), String> {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).muted = muted;
        self.writes.lock().unwrap_or_else(|p| p.into_inner()).push(VolumeOp::Mute(muted));
        Ok(())
    }
}

/// The volume a run gets: the real endpoint for the installed Kin (and for a test run with `--dev-media-real-volume`), the fake for everything else.
pub fn for_mode(mode: &Mode) -> Arc<dyn VolumeEndpoint> {
    if mode.real_volume() { Arc::new(WinVolume) } else { Arc::new(FakeVolume::new(50, 51)) }
}

// ----- Core Audio -------------------------------------------------------------------------------------------------------

/// COM for the calling thread, for as long as the guard lives. It is undone on drop **only if it was started here**: a call from a blocking-pool thread must leave
/// the thread as it found it, or the single-threaded file pickers that later run on the same thread fail with `RPC_E_CHANGED_MODE`. A thread that already has COM in
/// another mode keeps the mode it has (the init fails, nothing is undone). Declare it before any COM object, so the objects are released first.
struct Com(bool);

impl Com {
    fn init() -> Com {
        // SAFETY: balanced by `CoUninitialize` in `Drop` when this succeeded.
        Com(unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok())
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: balances the successful `CoInitializeEx` of `init`.
            unsafe { CoUninitialize() };
        }
    }
}

fn default_endpoint() -> windows::core::Result<IAudioEndpointVolume> {
    // SAFETY: COM calls on a thread that has a `Com` guard; the objects are used on this thread only.
    unsafe {
        let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device = enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia)?;
        device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None)
    }
}

/// The real master volume of the default playback device. Every call finds the default device again (headphones can be plugged in at any time).
pub struct WinVolume;

fn text(e: windows::core::Error) -> String {
    format!("the volume is not available: {e}")
}

impl VolumeEndpoint for WinVolume {
    fn get(&self) -> Result<VolumeState, String> {
        let _com = Com::init();
        let ep = default_endpoint().map_err(text)?;
        // SAFETY: read-only calls on an endpoint made on this thread.
        let (scalar, muted) = unsafe { (ep.GetMasterVolumeLevelScalar().map_err(text)?, ep.GetMute().map_err(text)?.as_bool()) };
        Ok(VolumeState { level: level_of_scalar(scalar), muted })
    }

    fn steps(&self) -> u32 {
        let _com = Com::init();
        let Ok(ep) = default_endpoint() else { return 51 };
        let (mut step, mut count) = (0u32, 0u32);
        // SAFETY: `GetVolumeStepInfo` writes two integers.
        match unsafe { ep.GetVolumeStepInfo(&mut step, &mut count) } {
            Ok(()) if count > 1 => count,
            _ => 51,
        }
    }

    fn set_level(&self, level: u8) -> Result<(), String> {
        let _com = Com::init();
        let ep = default_endpoint().map_err(text)?;
        // SAFETY: the one place the real master volume is written (guarded by `Mode::real_volume` in `for_mode`).
        unsafe { ep.SetMasterVolumeLevelScalar(f32::from(level.min(100)) / 100.0, &KIN_CONTEXT).map_err(text) }
    }

    fn set_mute(&self, muted: bool) -> Result<(), String> {
        let _com = Com::init();
        let ep = default_endpoint().map_err(text)?;
        // SAFETY: as in `set_level`.
        unsafe { ep.SetMute(muted, &KIN_CONTEXT).map_err(text) }
    }
}

// ----- change notifications ---------------------------------------------------------------------------------------------

/// Where a change of the real volume (by the user, with the slider or the keys) is reported. It must not block.
pub type OnChange = Arc<dyn Fn(VolumeState) + Send + Sync>;

#[implement(IAudioEndpointVolumeCallback)]
struct Listener {
    on_change: OnChange,
}

impl IAudioEndpointVolumeCallback_Impl for Listener_Impl {
    fn OnNotify(&self, pnotify: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> windows::core::Result<()> {
        // SAFETY: Windows passes a valid notification for the duration of the call.
        if let Some(data) = unsafe { pnotify.as_ref() } {
            (self.on_change)(VolumeState { level: level_of_scalar(data.fMasterVolume), muted: data.bMuted.as_bool() });
        }
        Ok(())
    }
}

/// A watcher of the real volume: a thread that registers a callback on the default endpoint and parks until it is stopped (nothing runs in between: Windows calls
/// the callback when the volume changes).
pub struct Watch {
    stop: mpsc::Sender<()>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Watch {
    pub fn start(on_change: OnChange) -> Result<Watch, String> {
        let (stop, parked) = mpsc::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let thread = std::thread::Builder::new()
            .name("media-volume-watch".into())
            .spawn(move || {
                let _com = Com::init();
                let ep = match default_endpoint() {
                    Ok(e) => e,
                    Err(e) => {
                        let _ = ready_tx.send(Err(text(e)));
                        return;
                    }
                };
                let callback: IAudioEndpointVolumeCallback = Listener { on_change }.into();
                // SAFETY: the callback object lives until it is unregistered below.
                if let Err(e) = unsafe { ep.RegisterControlChangeNotify(&callback) } {
                    let _ = ready_tx.send(Err(text(e)));
                    return;
                }
                let _ = ready_tx.send(Ok(()));
                let _ = parked.recv();
                // SAFETY: the same callback that was registered.
                let _ = unsafe { ep.UnregisterControlChangeNotify(&callback) };
            })
            .map_err(|e| format!("the volume watcher can't start: {e}"))?;
        ready_rx.recv().map_err(|_| "the volume watcher ended at once".to_string())??;
        Ok(Watch { stop, thread: Some(thread) })
    }

    pub fn stop(&mut self) {
        let _ = self.stop.send(());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Com::{
        APTTYPE, APTTYPE_CURRENT, APTTYPE_STA, APTTYPEQUALIFIER_IMPLICIT_MTA, APTTYPEQUALIFIER_NONE, COINIT_APARTMENTTHREADED,
        CoGetApartmentType,
    };

    #[test]
    fn a_scalar_is_a_percentage() {
        assert_eq!([1.0, 0.0, 0.5, 0.37, 0.994].map(level_of_scalar), [100, 0, 50, 37, 99]);
        assert_eq!(level_of_scalar(1.5), 100);
        assert_eq!(level_of_scalar(-1.0), 0);
    }

    #[test]
    fn a_delta_moves_at_least_one_step_and_stops_at_the_ends() {
        let d = |current, delta| target_level(51, current, VolumeOp::Delta(delta));
        assert_eq!(d(50, 5), Some(56));
        assert_eq!(d(50, -5), Some(44));
        assert_eq!(d(50, 1), Some(52), "one percent is less than a step of 2 %: one step");
        assert_eq!(d(50, -1), Some(48));
        assert_eq!(d(98, 100), Some(100));
        assert_eq!(d(4, -100), Some(0));
        assert_eq!(d(100, 5), Some(100));
        assert_eq!(target_level(51, 10, VolumeOp::Level(37)), Some(37));
        assert_eq!(target_level(51, 10, VolumeOp::Level(0)), Some(0));
        assert_eq!(target_level(51, 10, VolumeOp::Mute(true)), None);
        // A device with 101 steps moves by the percentage.
        assert_eq!(target_level(101, 50, VolumeOp::Delta(5)), Some(55));
    }

    #[test]
    fn the_fake_remembers_what_it_was_told_and_apply_returns_the_new_state() {
        let fake = FakeVolume::new(50, 51);
        assert_eq!(apply(&fake, VolumeOp::Delta(5)).unwrap(), VolumeState { level: 56, muted: false });
        assert_eq!(apply(&fake, VolumeOp::Level(10)).unwrap().level, 10);
        assert_eq!(apply(&fake, VolumeOp::Mute(true)).unwrap(), VolumeState { level: 10, muted: true });
        assert_eq!(fake.writes(), [VolumeOp::Level(56), VolumeOp::Level(10), VolumeOp::Mute(true)]);
    }

    #[test]
    fn a_test_run_gets_the_fake_and_never_the_real_endpoint() {
        let test = Mode::Test { aumid: None, real_volume: false };
        let v = for_mode(&test);
        // The fake starts at 50 % with 51 steps, whatever the PC's real volume is; a write changes only the fake.
        assert_eq!(v.get().unwrap(), VolumeState { level: 50, muted: false });
        assert_eq!(v.steps(), 51);
        v.set_level(20).unwrap();
        assert_eq!(v.get().unwrap().level, 20);
    }

    /// The apartment the calling thread started **itself** (`None`: it never called `CoInitializeEx`). A thread with no COM in a process that has an MTA is in the
    /// implicit MTA, which Windows reports as an MTA with the qualifier `IMPLICIT_MTA`: that is not a thread that joined, and it does not block an STA.
    fn apartment_of_this_thread() -> Option<APTTYPE> {
        let (mut kind, mut qualifier) = (APTTYPE_CURRENT, APTTYPEQUALIFIER_NONE);
        // SAFETY: a read-only question about the calling thread; it writes two integers.
        match unsafe { CoGetApartmentType(&mut kind, &mut qualifier) } {
            Ok(()) if qualifier != APTTYPEQUALIFIER_IMPLICIT_MTA => Some(kind),
            _ => None,
        }
    }

    #[test]
    fn a_volume_call_leaves_a_thread_with_no_com_as_it_found_it_so_a_picker_can_start_its_own_apartment_there() {
        // A blocking-pool thread runs the volume call and later a file picker (a single-threaded apartment): the call must not leave the thread in the MTA.
        // Read-only calls on the real endpoint only (`get` and `steps`): nothing here ever writes the volume.
        std::thread::spawn(|| {
            assert_eq!(apartment_of_this_thread(), None, "a fresh thread has no COM");
            let _ = WinVolume.get();
            assert_eq!(apartment_of_this_thread(), None, "after get");
            let _ = WinVolume.steps();
            assert_eq!(apartment_of_this_thread(), None, "after steps");
            // SAFETY: what the pickers do on the same kind of thread; balanced below.
            let sta = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
            assert!(sta.is_ok(), "the single-threaded apartment can still be started after a volume call: {sta:?}");
            // SAFETY: balances the successful call above.
            unsafe { CoUninitialize() };
        })
        .join()
        .unwrap();
    }

    #[test]
    fn a_volume_call_on_a_thread_that_already_has_an_apartment_leaves_that_apartment_alone() {
        std::thread::spawn(|| {
            // SAFETY: balanced at the end of this thread.
            assert!(unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok());
            let _ = WinVolume.get();
            let _ = WinVolume.steps();
            assert_eq!(apartment_of_this_thread(), Some(APTTYPE_STA), "the thread's own apartment is untouched");
            // SAFETY: balances the call above (and nothing else: the volume calls did not start COM here).
            unsafe { CoUninitialize() };
            assert_eq!(apartment_of_this_thread(), None, "and not left started by the volume call either");
        })
        .join()
        .unwrap();
    }

    #[test]
    fn volume_read_only_check_on_the_real_endpoint_does_not_write() {
        // Reads the real endpoint (no `Set*` anywhere in this test) when the PC has a playback device, and says so when it has none.
        match WinVolume.get() {
            Ok(s) => assert!(s.level <= 100),
            Err(e) => eprintln!("no playback device here: {e}"),
        }
    }
}
