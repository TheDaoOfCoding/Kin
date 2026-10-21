# tools/emulator

Android emulator helpers, so scripts can install and drive the Kin app without a phone. Windows PowerShell scripts. They use the SDK's `adb` by full path (`%ANDROID_HOME%\platform-tools\adb.exe`), never the one on `PATH`.

Setup: install the emulator package and the two system images below with `sdkmanager`, then create the two AVDs with `avdmanager` (names `kin_api37` and `kin_api29`; the scripts and the e2e checks look for exactly these names).

| AVD | Image | Use |
|---|---|---|
| `kin_api37` | `system-images;android-37.0;google_apis;x86_64` | the app's target (Android 17) |
| `kin_api29` | `system-images;android-29;google_apis;x86_64` | minSdk (Android 10) behaviour |

Both use a Pixel 6 profile (1080x2400), swiftshader graphics, cold boot, no snapshots, a 4 GB data partition. The AVD config is in `%USERPROFILE%\.android\avd\<name>.avd\config.ini`.

## Usage

Run from anywhere. Open a new terminal if `ANDROID_HOME` is missing. The scripts fall back to `%LOCALAPPDATA%\Android\Sdk`.

```
tools\emulator\start.ps1                    # headless kin_api37, waits for boot, prints "emulator-5554"
tools\emulator\start.ps1 -Avd kin_api29     # the API 29 AVD
tools\emulator\start.ps1 -Window            # with a visible window
tools\emulator\install.ps1                  # installs android\app\build\outputs\apk\debug\app-debug.apk and launches Kin
tools\emulator\install.ps1 -Apk path\to.apk
tools\emulator\start.ps1 -WipeData          # factory-reset the data partition first (repair only: adb "offline" in a loop after a hard kill)
tools\emulator\screenshot.ps1               # saves tools\emulator\screenshot-<time>.png and prints the path
tools\emulator\screenshot.ps1 -Out C:\temp\a.png
tools\emulator\stop.ps1                     # stops the Kin emulators (AVDs named kin_*) and, if none is left, the adb server
tools\emulator\stop.ps1 -All                # stops every emulator, also ones of other AVDs
```

- Build the APK first if it is missing: `cd android; gradlew.bat assembleDebug`.
- The emulator always uses serial `emulator-5554` (port 5554). Only one emulator runs at a time. `start.ps1` refuses to start a second AVD while one is running, and reuses a running one if it is the same AVD.
- Always run `stop.ps1` when you are done. The emulator uses about 4 to 5 GB of RAM.
- `stop.ps1` finds the AVD of each running emulator with `adb -s <serial> emu avd name`, and only stops `kin_*` ones. An emulator of another AVD that you started yourself keeps running (use `-All` to stop it too). It also deletes the multi-GB `snapshots` folder that the emulator writes on exit, **only** in `kin_*.avd` folders, never in other AVDs.
- Drive the device with adb directly: `& "$env:ANDROID_HOME\platform-tools\adb.exe" -s emulator-5554 shell input tap 540 1200`, `... shell input text hello`, `... shell uiautomator dump`, `... logcat -d`.
- `start.ps1` sets animations to 0 and keeps the screen on, so scripted taps are repeatable.
- Logs go to `tools\emulator\emulator-<avd>.log` (gitignored). Read them if a boot fails.

## Measured on one development PC (for scale only; 32 GB RAM, Windows Hypervisor Platform)

| | kin_api37 | kin_api29 |
|---|---|---|
| Cold boot to ready | 26 s (57 s on the first boot after the data partition was created) | 22 to 24 s |
| Host RAM (qemu working set, app running) | about 4.9 GB | about 3.7 GB |
| Guest RAM | 4096 MB (see below) | 2560 MB |
| Disk, AVD after use | about 1.3 GB for both | |

The API 37 image raises the guest RAM to 4096 MB by itself ("Increasing RAM size to 4096MB" in the log), whatever `hw.ramSize` says. API 29 respects the 2.5 GB setting.

## adb and Unity

The e2e, smoke and bench scripts (`tools\kin-lock.ps1`) set `ANDROID_ADB_SERVER_PORT=5038`, so they and the emulator they start use an adb server of their own and never meet Unity's or an IDE's on 5037 (two adb servers on one emulator take it "offline" every few seconds); with that variable set, `Initialize-Adb` does not stop other adb processes. The text below is for the scripts run by hand without it.

Unity ships its own `adb.exe` (under `<Unity install>\...\PlaybackEngines\AndroidPlayer\SDK\platform-tools`), and a different version. Two adb versions fight over port 5037. Every script here calls `Initialize-Adb` in `_common.ps1`, which kills any adb server that is not ours and starts our own.

- Unity's Android tooling may restart its own server later and take the port back. Our adb then replaces it again the next time a script runs. If you call adb by hand and see "adb server version doesn't match", run `adb kill-server` using our full path, or run any script here.
- `stop.ps1` also stops the adb server, so nothing is left running.

## Networking

- **Host loopback is `10.0.2.2`.** From inside the emulator, `10.0.2.2` is the PC's `127.0.0.1`. A desktop server bound to `127.0.0.1` is reachable from the app with no Windows Firewall prompt, because the traffic arrives on loopback. Verified: a PC listener on `127.0.0.1:47999` answered `nc 10.0.2.2 47999` from inside the emulator, and `ping 10.0.2.2` works.
- A server bound to `0.0.0.0` is reachable too, but then Windows may show the firewall prompt for that program.
- To reach an emulator service from the PC, use `adb forward tcp:HOST_PORT tcp:DEVICE_PORT`.
- The emulator is on its own virtual network `10.0.2.0/24` behind NAT. The emulator's own IP is `10.0.2.15`. It is not on the PC's LAN, and the LAN cannot reach it.
- **mDNS / multicast from the emulator does not reach the host LAN.** Discovery by mDNS or UDP broadcast cannot be tested here. Use a fixed address (`10.0.2.2:<port>`) for emulator-to-desktop tests, and test discovery on the real phone and Wi-Fi.
- DNS inside the emulator is forwarded to the host's DNS, so internet access works.

## Known limits

- No OEM battery killing (Samsung, Xiaomi and so on). Aggressive background limits cannot be reproduced here. Test foreground service survival on the real phone.
- Battery numbers are meaningless. The emulator reports a fake battery, so Doze and battery stats are not representative.
- It is not on the real Wi-Fi: no Wi-Fi sleep, no roaming, no router quirks, no AP isolation, no captive portals.
- Google APIs images, no Play Store. Play services behaviour is partial.
- Swiftshader software graphics: slower than a phone, and not a reference for animation smoothness or performance.
- The emulator is x86_64, so native ARM-only libraries will not run on it.
- Sensors, camera, Bluetooth and a real USB or NFC stack are fake or missing.
