# Testing Kin Relay on real hardware

A guide for the person who tests: no programming needed. It takes you from "I have two files" to "I have a report". Every step is short. If a step says what you should see and you see something else, that is a result worth writing down: see [Reporting a problem](#8-reporting-a-problem).

What you need: a Windows 10 or 11 PC (x64) and an Android phone (Android 10 or newer), both on the **same Wi-Fi**. For the soak test (part 7), 3 days of normal use.

Parts: [1 Install on the PC](#1-install-the-pc-app) · [2 Install on the phone](#2-install-the-phone-app) · [3 Before you start](#3-before-you-start-phase-0-leftovers) · [4 Pair](#4-pair-the-phone-and-the-pc) · [5 Set up the phone](#5-set-up-the-phone-so-android-leaves-kin-running) · [6 The test matrix](#6-the-test-matrix) · [7 The 3-day soak](#7-the-3-day-soak) · [8 Reporting](#8-reporting-a-problem) · [9 Uninstall](#9-uninstall)

---

## 1. Install the PC app

You get one file, `Kin_0.1.0_x64-setup.exe` (about 3.6 MB). In the project it is at `desktop\src-tauri\target\release\bundle\nsis\Kin_0.1.0_x64-setup.exe`.

**On the PC where Kin was developed:** developer runs left a folder `%APPDATA%\Kin`. Before installing, rename it to `Kin.old` (paste `%APPDATA%` in Explorer's address bar). Otherwise the installed Kin reuses that old profile, and **Start with Windows stays off** (it is turned on only for a new profile). On any other PC there is nothing to do.

1. **Double-click it.** Windows shows a blue box, **"Windows protected your PC"** (SmartScreen). That is expected: the installer is **not signed** yet (a certificate costs money and the MVP has none), so Windows doesn't know who made it.
   - Click **More info**, then **Run anyway**.
   - If the file came by chat or e-mail and nothing happens: right-click it, **Properties**, tick **Unblock**, OK, and try again.
2. A **"Do you want to allow this app to make changes?"** box appears (the administrator prompt). Click **Yes**. The installer needs it to add a Windows Firewall rule so that your phone can reach the PC.
3. Click through the installer. Leave the box **Run Kin** ticked on the last page.
4. Kin starts in the **tray** (the small icons next to the clock; click the **^** if you don't see it). It has a **K** icon.
5. **Check the install.** Open **PowerShell** (Start menu → type `powershell` → Enter; a normal window, not "as administrator") and run:

   ```
   powershell -ExecutionPolicy Bypass -File "<folder>\verify-install.ps1" -Toast
   ```

   `<folder>` is where you have the file (in the project: `desktop`). It prints one line per check: **PASS**, **FAIL** or **WARN**, and a notification appears: its title should say **Kin** with the Kin icon (not "Windows PowerShell").
   - **FAIL** on a line: copy everything it printed into your report.
   - **WARN: no "Send to" shortcut / no Run value yet:** Kin makes those for you the first time it runs. Wait 10 seconds and run the check again.

What the installer does: adds the firewall rules ("Kin Relay"), a Start menu entry "Kin" and an uninstaller. When Kin runs it adds **Kin to Explorer's "Send to" menu** and **starts with Windows** (you can turn that off in Kin's Settings).

**Updating** (a newer installer over an old one): run it the same way. It offers to uninstall the old version first: accept, and leave **"Delete the application data" unticked**, so your pairing stays.

## 2. Install the phone app

You get one file, **`Kin-0.1.0.apk`** (about 4.5 MB, version 0.1.0, for Android 10 or newer). In the project it is `android\app\build\outputs\apk\release\app-release.apk`: copy it and rename the copy to `Kin-0.1.0.apk`. It is signed with the project's own release key (`%USERPROFILE%\.kinrelay\release.jks` on the build PC); its certificate fingerprint (SHA-256) is `83fd45c8402978c061c784d1f5dd5f7f1cc91c6165faa89a767c5bc19ec7d419`. Because it doesn't come from the Play Store, Play Protect may warn about it (step 4 below).

> **Back up the folder `%USERPROFILE%\.kinrelay` now** (both files in it) to somewhere private, such as an encrypted USB stick or a password manager's file storage. If that key is lost, no later version of Kin can be installed over this one: every phone would have to uninstall Kin first, which deletes its pairing and history. Never put the folder in the project, a chat or an e-mail.

**Method A: from the phone** (the usual one)

1. Get the APK onto the phone (cable, chat, cloud drive, or download it).
2. Open it from the **Files** app (or the browser that downloaded it).
3. Android says **"For your security, your phone is not allowed to install unknown apps from this source"**. Tap **Settings**, switch on **Allow from this source** (it is for that one app, such as Files or Chrome; you can switch it off again afterwards), go back and tap **Install**.
4. If **Play Protect** says it blocked or doesn't recognise the app: tap **More details**, then **Install anyway** (the app isn't on the Play Store yet). If the box only offers **OK**: open the **Play Store** → your profile picture → **Play Protect** → the gear icon → turn off **Scan apps with Play Protect**, install, and turn it back on afterwards (or use Method B).
5. Tap **Open**.

If Android says **"App not installed"** and the phone already has a Kin from a developer build, uninstall that one first (the two are signed differently).

**Method B: with a USB cable** (`adb`, for people who have it)

1. On the phone: **Settings → About phone**, tap **Build number** 7 times, then **Settings → System → Developer options → USB debugging** on. Connect the cable and accept the prompt on the phone.
2. On the PC: `adb install -r Kin-0.1.0.apk` (use the real file name).

USB debugging is **optional**: you only need it for Method B and for the measurements with `adb` that a developer may ask for. Switch it off afterwards.

## 3. Before you start (Phase 0 leftovers)

- **USB debugging** on the phone is optional, as above.
- **The network check** (`tools/net-check.ts`) is only for when pairing fails and you want to know whether the phone can reach the PC at all. Skip it unless part 4 doesn't work. How to run it is in [tools/README.md](../tools/README.md); the short version: `bun tools/net-check.ts` on the PC, then open the printed address in the phone's browser. If the page doesn't load, the cause is the network, not Kin (a guest Wi-Fi, "AP isolation" or "client isolation" in the router, or the firewall).
- **Write down your devices** (for the report, and for the "Reference devices" table in [targets.md](targets.md)): the PC model and Windows version, whether it is on Wi-Fi or a cable; the phone's model, **Android version** and brand skin (One UI, MIUI/HyperOS, ColorOS, …); the router and the Wi-Fi band (2.4 or 5 GHz).

## 4. Pair the phone and the PC

You do this once.

1. **On the PC:** **right-click** the Kin **tray icon** → **Pair new phone**. (A left click opens the window for sending files.) A window shows a **QR code** and counts down from 5 minutes.
2. **On the phone:** open **Kin** (the app is called Kin) and tap **Scan code**.
3. The phone asks to use the **camera**: tap **While using the app** (or **Allow**). (The scanner is built into the app, so Android asks for the camera the first time. Nothing is recorded.)
   **If scanning fails** (the camera doesn't work, or you refused the camera permission): in the PC window click **Copy pairing link**, send the link to your phone (for example by message to yourself), and paste it in Kin's first-run screen. It works once, for 5 minutes.
4. Point the phone at the QR code. Kin reads it and pairs straight away: tapping **Scan code** was your confirmation, so there is no extra question. (A pasted link pairs straight away too. Only a link **tapped outside Kin**, for example in a chat, first shows **"Pair with this PC?"** with the PC's name and short code: check that the name is yours, then confirm.)
5. Both sides show success: the PC shows **"New device paired"**, the phone's Home shows **Connected to <PC name>**, and the tray icon gets a **green dot**.
6. If your phone is on **Android 17**, Android asks for the **Nearby devices** permission (permission to find and connect to devices on your local network): tap **Allow**. Without it the phone cannot reach the PC at all, and Home says "Nearby devices permission needed".
7. **Time yourself:** from opening the installer to "Connected" should be **under 1 minute** for someone who hasn't seen the app (it is one of the targets). Write down the time.

**If it doesn't connect:** are both on the same Wi-Fi (not a guest network, not mobile data)? Right-click the tray icon → **Settings** → **Debug…** shows the PC's **network profile**; Windows calls a new network **Public** by default, which the installer's firewall rule covers. Run `verify-install.ps1` again and look for FAIL lines. Then try the network check (part 3).

## 5. Set up the phone so Android leaves Kin running

Android tries to save battery by stopping apps that run in the background, and many phone makers are strict about it. Kin has to stay connected to be useful. After the first pairing Kin opens a **setup screen** (you can open it again any time: **Settings → Fix connection reliability**). Go through every row until it shows **done**:

1. **Notifications** (Android 13 or newer): tap **Allow**. Kin shows one quiet notification ("Connected to <PC name>") while it runs, and notifies you of what arrives.
2. **Nearby devices** (Android 17): **Allow** (see part 4).
3. **Battery:** open the battery list and set **Kin** to **Not optimised** / **Unrestricted** / **No restrictions**.
4. **Your phone's brand.** The setup screen shows the steps for your phone and opens the right settings page. In short:

   | Phone | What to set |
   |---|---|
   | **Samsung** | Settings → Battery → Background usage limits: add Kin to **Never sleeping apps**; make sure it is not in "Sleeping apps" or "Deep sleeping apps". Battery → Kin → **Unrestricted** |
   | **Xiaomi / Redmi / POCO** (MIUI, HyperOS) | Settings → Apps → Manage apps → Kin → **Autostart: on**; **Battery saver: No restrictions**; in the Recents screen, press and hold the Kin card and **lock** it |
   | **Oppo / Realme / OnePlus** (ColorOS, OxygenOS) | Settings → Battery → (App battery management) → Kin → **Allow background activity**; allow **auto-launch / startup** |
   | **Vivo / iQOO** | Settings → Battery → **Background power consumption** → Kin → allow; **Autostart** on |
   | **Huawei / Honor** | Settings → Apps → App launch → Kin → **Manage manually**, all three switches on |
   | **Google Pixel and most others** | Settings → Apps → Kin → App battery usage → **Unrestricted** |

   The setup screen's own text wins if it differs: phone makers move these screens around.

When all rows are **done**, send yourself a test: copy some text on the PC, and look at the phone. It should appear in the phone's clipboard within a second.

## 6. The test matrix

This is the list from [targets.md](targets.md). Do each **payload** in each **situation**, **in both directions**, and write down **pass or fail and the time**. Use a stopwatch or your phone's.

### How to send each payload

| | Phone → PC | PC → phone |
|---|---|---|
| **Short text, a link** | Select text → **Send to PC** in the selection menu; or the **Share** button → Kin (the PC shows in the top row) | Copy it on the PC; it goes to the phone's clipboard by itself (automatic clipboard sync). Or press the hotkey **Ctrl+Alt+Shift+K**, or tray → **Send clipboard to phone** |
| **Files, photos, video** | Open it in Photos/Files → **Share** → Kin | Tray icon → click (the **drop window**) and drag files into it; or right-click files in Explorer → **Send to → Kin** (on Windows 11: **Show more options** first) |
| **The phone's clipboard** | The **quick tile** or the notification's **Send clipboard** button | (the automatic sync above) |

**What "arrives" looks like:** on the PC a **notification** (click it to open the file or the link; text and images are already on the clipboard: try **Ctrl+V**); on the phone a **notification**, text on the clipboard, photos and videos in the **gallery**, other files in **Download/Kin**. Received files on the PC are in **Downloads\Kin**.

### The payloads

1. A short text
2. A link
3. 100 KB of text (a long document pasted in)
4. One 3 MB photo
5. A batch of 50 photos
6. A 2 GB video

### The situations

Run **every payload** in the first situation, then the ones that matter most in each of the others (at least 1, 2, 4 and 5: the short text and the 3 MB photo) if you don't have time for all.

| # | Situation | How |
|---|---|---|
| 1 | **Both awake** | Phone unlocked, PC in use |
| 2 | **Phone's screen has been off for over an hour** | Lock the phone, leave it for an hour, then send from the PC. Write down how long the text takes |
| 3 | **Phone just switched Wi-Fi networks** | Join another network (or switch between the 2.4 and 5 GHz names), then send |
| 4 | **PC just woke from sleep** | Put the PC to sleep, wake it, send straight away |
| 5 | **Wi-Fi dropped mid-transfer, then came back** | Start the 2 GB video, turn the phone's Wi-Fi off for about 10 seconds, turn it on. The transfer should **continue**, not start over |
| 6 | **The phone rebooted** | Restart the phone, wait for it to unlock, send |
| 7 | **The PC rebooted** | Restart the PC and sign in. Kin should start by itself (tray icon), then send |
| 8 | **The app was swiped away from Recents** | Swipe Kin away on the phone, then send from the PC. (On some phones this stops the app: write down if it does) |
| 9 | **Battery saver is on** | Turn **Battery saver** on, then send |

Also try **the queue**: turn the phone's Wi-Fi off, send something from the PC (the tray says "Phone offline … 1 queued"), turn Wi-Fi on. It should arrive **within a few seconds**, and the same the other way round.

### What "good" looks like

| Check | Good |
|---|---|
| **Reliability** | At least **99 of 100 sends work the first time** on your normal home Wi-Fi. A failure shows an error in History (and the Debug screen counts them) |
| **Arrival time** | Text and clipboard in **under 1 second**. A 3 MB photo in **under 3 seconds** |
| **Big files** | The 2 GB video arrives complete and opens. The speed should be **at least as good as LocalSend** with the same files, phones and positions (try LocalSend with the same 2 GB file if you want to compare; the number is in MB/s) |
| **Resume** | After the Wi-Fi drop (situation 5) it carries on from where it stopped |
| **PC footprint** | With every window closed, Kin uses **under 30 MB** and about 0% CPU: Task Manager → Details → **Kin.exe** → look at **Memory (active private working set)**; add the Details column if it isn't there. Look after an hour |
| **Sizes** | The installer and the APK are each about **10 MB or less** |
| **Setup time** | **Under 1 minute** from opening the installer to "Connected" |
| **Nothing odd** | No crash, no error you can't explain, no file that arrives empty or twice |

### A results sheet

Copy this into a note and fill it in:

```
PC: <model, Windows version, Wi-Fi or cable>      Phone: <model, Android version, skin>      Router/band: <...>
Setup time (installer to Connected): __ s
                          sit.1   sit.2   sit.3   sit.4   sit.5   sit.6   sit.7   sit.8   sit.9
short text  phone->PC     ok/__s
short text  PC->phone
link        both
100 KB text both
3 MB photo  phone->PC
3 MB photo  PC->phone
50 photos   phone->PC
50 photos   PC->phone
2 GB video  phone->PC
2 GB video  PC->phone
Queue (Wi-Fi off, then on): arrived after __ s
Kin memory after 1 h idle: __ MB
```

### Phone notifications on the PC (0.2.0): a limit of Android 10 and 11

On **Android 10 and 11** the phone can't tell a notification button that opens an app (an activity) from one that does something in the background (Android only added the question in 12), so it offers both. A button such as **"Open"** therefore **appears on the PC's toast, and pressing it shows "Sent to <phone>"** (the phone really did fire it and said so), **but Android silently blocks the app from starting**, so nothing happens on the phone. This is Android's rule for apps started from the background, not a fault in Kin, and the PC cannot know. On Android 12 and later such buttons are not offered at all. Note it in the results sheet if you see it; don't count it as a failed send.

### Media controls on a real phone (0.2.0): what an emulator can't show

The emulator has no secure lock screen, no headset, no car and no real players, so these three checks are only possible on your phone (Android 13 to 16; a Pixel and a Samsung if you have both). In Kin's Settings → **Media**, turn on **Show your PC's media on this phone** (and on the PC, share its media), and play something on the PC so the card is on your phone.

1. **Headset or car buttons while the PC session is `playing`.** With the PC's media playing, press play/pause, next and previous on a Bluetooth headset or the car's controls. Write down which ones reach the PC and whether any of them starts a **local** player on the phone instead. (Kin's session reports `playing` while the PC plays, so the phone should hand the buttons to it.)
2. **Lock-screen exposure, "hide sensitive content" on and off.** Lock the phone with the PC's media showing. In Android's lock-screen settings set **Notifications / "Hide sensitive content"** (Samsung: **Lock screen → Notifications → Hide content**) first **on**, then **off**, and each time with Kin's **Show PC media on the lock screen** switch off and then on. Write down, for each of the four combinations, what the lock screen shows: nothing, only "Media from your PC", or the title and artist. Kin marks the card private when its own switch is off, but whether a phone still draws the media controls is the phone's own choice: this is the check that says which phones do. Do it on a Pixel and on a Samsung, because they differ.
3. **Volume keys switch while a local player plays.** Turn on **Use the phone's volume keys for the PC**. Start music on the phone itself, then start the PC's media. Press the phone's volume keys: they should change the **PC's** volume while the PC is playing (the on-screen volume dialog says "Kin"), and the phone's own volume again when the PC pauses or stops. Write down if the keys ever stay on the wrong device.

### Screenshots to the PC on a real phone (0.2.0): what an emulator can't show

In Kin's Settings → **Screenshots**, turn on **Send screenshots to your PC** (allow the photo permission; give **All photos** access). These two checks need your phone.

1. **The hold notification on its low-importance channel: it must be seen within 5 seconds.** The "Sending screenshot to <PC> in 5 s" notification (with **Cancel**) is on the **File transfers** channel, which is low importance: it appears in the shade and the status bar, **not as a pop-up**. Take a screenshot, and **open the notification shade within 5 seconds** to press **Cancel** (on Samsung, check the same with the shade on the lock screen). Write down whether you could reach Cancel in time, whether the notification shows at all on your phone (Samsung and Xiaomi sometimes hide low-importance notifications), and whether the screenshot reached the PC when you did nothing. Then in Android's settings turn off Kin's notifications (or just the **File transfers** channel) and take another screenshot: it **must not be sent** and Kin's Settings must say **"Needs notifications"**.
2. **Android 14+: "Select photos" and a withdrawn grant.** When Android asks for photo access, choose **Select photos** (not "Allow all"): Kin must show **"Needs access to all photos"** and watch nothing (a session-only grant can't work in the background). Then grant **Allow all**, turn auto-send on, and later withdraw it in Android's settings (Apps → Kin → Permissions → Photos and videos → Don't allow) **while Kin is running**. Android may **kill Kin's process** when a photo grant is withdrawn: write down whether it did, whether Kin came back by itself (the connection to the PC returns without opening Kin), and that **no screenshot is sent** after the withdrawal and the switch shows "Needs access to all photos".

## 7. The 3-day soak

The soak test asks: **does an always-on connection survive on a real phone without hurting the battery?** It is the biggest open question of the project. For **at least 3 days**, use the phone and the PC normally (carry the phone around, let it sleep overnight, leave the PC on or put it to sleep as you always do). **Don't** open Kin to "keep it alive": that spoils the test.

**Each evening, write down:**

1. **Battery used by Kin.** Settings → **Battery** → **Battery usage** (or **App battery usage**; on Samsung: Battery → Battery usage → *Today / Last 7 days*) → find **Kin**, and write down its **percentage since the last full charge**, and for how many hours that is. Good is **under 1% per day**. Compare with a day without Kin if you can (stop Kin: Settings → Apps → Kin → Force stop, and note the battery drain of the day).
2. **The phone's Debug screen:** open **Kin → Settings → Debug**. It shows **Connects**, **Disconnects** and **Service kills** (how many times Android stopped Kin), plus the success rate. Write them down; they only go up.
3. **The PC's Debug window:** right-click the tray icon → **Settings** → **Debug…**, then **Connection history**: how many times the phone went away today and why.
4. **One timed test after the screen was off for over an hour:** send a text from the PC to the phone and note how long it takes to arrive. Good is **under 3 seconds**.

```
         battery % (hours)   connects   disconnects   service kills   screen-off test (s)
day 1
day 2
day 3
```

Good: **under 1% per day**, a service that **survives** (kills stay at 0 or very low), and a screen-off text in **under 3 s**. If Android kills the service often, or the battery use is high, **say so**: those are the results that decide how the connection is built next. Also write down anything that stopped working and **when**.

**Optional, for a developer with a cable:** `adb shell dumpsys batterystats --charged app.kinrelay.android` gives the exact number.

## 8. Reporting a problem

When something fails, or at the end of the soak, send **both logs** and **a short description**.

1. **Export the PC's logs:** right-click the tray icon → **Settings** → **Debug…** → **Export logs…**. Choose where to save the `.zip`.
2. **Export the phone's logs:** **Kin → Settings → Debug → Export logs**. A share sheet opens: send the file to yourself or straight to the person collecting the reports.
3. **Write a few lines:**
   - **What you did** (for example: "shared a 3 MB photo from Photos to the PC").
   - **What you expected, and what happened** (including any message you saw).
   - **When** (the time, so it can be found in the logs).
   - **Your devices** (the note from part 3) and whether it happens **every time**.
   - A **screenshot** of the error if there is one.
   - If you ran `verify-install.ps1`, its output.
4. Send the two zip files and the text to the person collecting the reports (for the first test on your own devices, keep them together in one folder).

**What is in the logs, and what is not:** the logs say what connected and when, errors and timings. Before the file is made the app takes out your Windows user name, your PC's name and your phone's name, IP addresses, Wi-Fi network names and file names; it never writes the text you copied or your files at all. You can open the zip and look.

## 9. Uninstall

**The PC**

1. **Settings → Apps → Installed apps → Kin → Uninstall** (Windows 10: Apps & features). Accept the administrator prompt.
2. On the uninstall page there is a box **"Delete the application data"**. **Unticked** keeps your pairing and history (what you want if you will install again). **Ticked** removes them too, and the phone has to be paired again. It is unticked by default.
3. The uninstaller removes the firewall rules, the Start menu entry, the **Send to** entry and the start-with-Windows entry.
4. **Check it:** `powershell -ExecutionPolicy Bypass -File "<folder>\verify-uninstall.ps1"` (add `-ExpectDataDeleted` if you ticked the box). All lines should be **PASS**. A **FAIL** line says what is left and how to remove it.

**The phone**

1. Optional first: **Kin → Settings → Unpair**, so the PC forgets the phone.
2. **Settings → Apps → Kin → Uninstall.**
3. Switch **Allow from this source** off again for the Files app or browser you used (Settings → Apps → Special app access → Install unknown apps), and USB debugging if you turned it on.
