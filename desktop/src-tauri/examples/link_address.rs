//! What the PC's toast shows for a web address, and whether it has a warning, read from the real code (`links::link_address`, protocol.md "Showing the
//! address of a link"): `cargo run --example link_address -- <file>`. The file has one URL per line as the hex of its UTF-8 bytes; each answer line is
//! `<url hex>|<shown hex>|<warn>` (hex again, so no console code page can change a character). For `tools/e2e-2b-1.ps1`, which compares it with what a
//! browser and the phone show.
fn unhex(s: &str) -> Option<String> {
    let bytes: Option<Vec<u8>> = (0..s.len() / 2).map(|i| u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()).collect();
    String::from_utf8(bytes?).ok()
}

fn hex(s: &str) -> String {
    s.bytes().map(|b| format!("{b:02x}")).collect()
}

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: link_address <file with one hex-encoded URL per line>");
        std::process::exit(2);
    };
    let text = std::fs::read_to_string(&path).expect("the input file is readable");
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let Some(url) = unhex(line) else {
            println!("{line}||error");
            continue;
        };
        let address = kin_lib::links::link_address(&url);
        println!("{line}|{}|{}", hex(&address.shown()), address.warn);
    }
}
