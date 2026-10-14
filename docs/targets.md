# Targets and how we measure them

## Priority order

When two goals conflict, the higher one wins. These are targets we chose; none have been measured yet. Results go in `docs/results/`.

| # | Goal | MVP target | How we measure it |
|---|---|---|---|
| 1 | Reliability | At least 99% of sends succeed on the first try on a normal home network. Every failure is logged with an error code | Every transfer's outcome is stored. The debug screen shows the success rate and recent errors. Friends export their logs |
| 2 | Arrival time | Text and clipboard in under 1 s. A 3 MB phone photo in under 3 s | The sender records time from the tap to the receiver's `ack`. That includes the return trip, so it slightly overstates the real time, which is fine. Shown on the debug screen |
| 3 | Phone battery | Under 1% per day while idle and connected | After 24 h: Settings → Battery → app usage, and `adb shell dumpsys batterystats --charged app.kinrelay.android`. Compare with a day without the app |
| 4 | Transfer speed | Large files at least as fast as LocalSend on the same network | A 2 GB file, 5 runs each way, median MB/s. Then LocalSend with the same file, devices and positions |
| 5 | PC footprint | When idle: under 30 MB of RAM and near 0% CPU | After 1 h idle with all windows closed: Task Manager's "Memory (active private working set)", including any leftover WebView2 processes (there should be none). CPU from `Get-Process` sampled over 10 minutes |
| 6 | App size | APK and installer each about 10 MB or less | Build output sizes |
| 7 | Setup time | Under 1 minute from opening the installer to "Paired" | Stopwatch, with someone who hasn't seen the app before |

Not a target in the MVP: how the UI looks, and support for more than one phone and one PC.

## Reference devices

Fill this in during Phase 0.

| Device | Model | OS / skin | Notes |
|---|---|---|---|
| PC | | Windows | Wi-Fi or Ethernet? |
| Phone 1 (yours) | | | |
| Phone 2 | | | |
| Phone 3 | | | |
| Router | | | Wi-Fi standard, band |

## Test matrix

Run every payload in every situation, in both directions. Record pass or fail and the time taken.

**Payloads:**

- Short text
- A link
- 100 KB of text
- One 3 MB photo
- A batch of 50 photos
- A 2 GB video

**Situations:**

1. Both devices awake
2. The phone's screen has been off for over an hour
3. The phone just switched Wi-Fi networks
4. The PC just woke from sleep
5. Wi-Fi dropped mid-transfer, then came back
6. The phone rebooted
7. The PC rebooted
8. The app was swiped away from recent apps
9. Battery saver is on
