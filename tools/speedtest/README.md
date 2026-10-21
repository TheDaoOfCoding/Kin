# tools/speedtest

Helpers for the real-hardware speed test, Kin vs LocalSend. Windows PowerShell 5.1 + Node 22, adb, a real phone on Wi-Fi.

**They inject no input on the PC.** The PC side only starts `Kin.exe --send` (a command), reads the Ethernet byte counters and the files on disk, and makes outgoing connections. Everything that needs a tap is a phone tap through adb (`phoneui.ps1`). Do not add PC UI automation here: input injection can break a game with an anti-cheat that happens to be running.

Work folder (not in the repo): `%LOCALAPPDATA%\Temp\kin-speedtest` with `data\`, `photos\`, `results\` and a `phone.ip` file (the phone's Wi-Fi address, one line). Set `ANDROID_SERIAL` to the phone's adb serial first.

| Script | What it does |
|---|---|
| `make-files.ps1` | One 2 GiB random file and 50 random `.jpg` of about 3.4 MB, all named `kin-speedtest-*`. |
| `speedmon.ps1` | Passive monitor for one case. Samples the Ethernet counters (50 to 250 ms), polls the receiver until every file is at its final name and full size, optionally samples the phone's `top`. Writes one JSON. Every loop has a timeout. |
| `run-kin-out.ps1` | One Kin PC to phone run: monitor + `Kin.exe --send`, then Kin's own start and finish from `status.json`. Pass `-PhoneDirs /sdcard/Download/Kin` for files and `/sdcard/Pictures/Kin` for photos (Kin puts images in the gallery folder). Deletes the phone's test files afterwards. |
| `run-kin-in.ps1` | One Kin phone to PC run. The file(s) must already be selected in the phone's file browser (DocumentsUI); the script taps Share, then Kin in the share sheet, and times it. (An `am start ... ShareActivity` with a `content:` URI does not work: Kin gets no read grant and skips the file.) |
| `ls-send.mjs` | A **scripted LocalSend v2 sender** (`prepare-upload` and `upload` on port 53317) that talks to the phone's LocalSend app. Needs a client certificate (LocalSend asks for one): make a throwaway one with `openssl req -x509 -newkey rsa:2048 -nodes -keyout k.key -out k.crt -days 2 -subj /CN=speedtest-script` and set `LS_CLIENT_CERT` / `LS_CLIENT_KEY` to the two files. `node ls-send.mjs <phone-ip> --info` only reads `/info`. Never read LocalSend's own `settings.json` (it holds its private key). |
| `ls-run-out.ps1` | One LocalSend PC to phone run with that sender, with the same monitor. Photos land in `/sdcard/Pictures`, other files in `/sdcard/Download`. |
| `phoneui.ps1` | `uiautomator dump` + `input tap` helpers (`Get-UiNodes`, `Find-Node`, `Tap-Node`, `Show-Ui`). |

Things that bit the first run: the monitor refuses to start if the phone folder already has a `kin-speedtest*` entry (a leftover folder counts); the wire window uses the PC's whole NIC counters, so other traffic (a game download) shows up in a phone to PC window; LocalSend's Receive tab animates, so `uiautomator dump` fails there (tap the tab bar by coordinates).
