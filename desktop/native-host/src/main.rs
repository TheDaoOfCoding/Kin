// A native messaging host is started by the browser with its stdin and stdout redirected to the browser. Built for the Windows GUI subsystem
// so that no console window can flash when the browser starts it; stdin and stdout still work (the browser hands them over), and stderr is
// simply absent.
#![windows_subsystem = "windows"]

use kin_native_host::config::Config;
use kin_native_host::host::{self, Ended};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let origin = kin_native_host::origin_from_args(&args);
    let cfg = Config::from_build();
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let ended = host::run(&cfg, &origin, &mut stdin.lock(), &mut stdout.lock());
    // The browser doesn't look at the exit code; a script can.
    std::process::exit(match ended {
        Ended::InputClosed => 0,
        Ended::TooLarge => 3,
        Ended::InputBroken | Ended::OutputBroken => 2,
    });
}
