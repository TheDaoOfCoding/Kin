# One main window on the PC, and History filters on both apps

2026-10-28 · @Sam

## Goal

On the PC, Home, History and Settings are pages of one main window. A click in the sidebar swaps the page in place, at once, with no new window, no open animation and no reload, and each page keeps what it loaded. Pairing and Debug stay separate windows with the same strict command locks as today. On the PC and on the phone, History has one **Filter** button that opens a small panel (a popover on the PC, a bottom sheet on the phone) with a search box, a type filter (Photos, Videos, Documents, Apps, Other files, Texts, Links), a sort (Newest, Oldest, Name, Size) and a direction (Sent, Received, Both). Active choices show as removable chips. Filtering runs on the list that is already loaded (the newest 500 on both sides). Both skins, light and dark, look right through the existing theme tokens.

## Already done

- Sidebar shell shared by Home, History and Settings: `desktop/src/shell.ts` (commit ab7f345). Each tab click today calls `open_window`, which builds a new window at the old one's place and destroys the old one (`desktop/src-tauri/src/ui.rs` `open_window`, `open_page_at`, `placement_of`).
- Per-window command locks: `only(&window, ...)` in every command of `ui.rs`, `capabilities/{pairing,drop,history,settings,debug}.json`, and the guard tests in `desktop/src-tauri/tests/review_1_4.rs` ("window security" section, 37 commands).
- Warm/Simple skins, light and dark: CSS tokens in `desktop/src/styles.css` lines 10 to 15, `KinTheme` on Android (commits ab7f345, 17593e7).
- History caps: PC `HISTORY_CAP = 500` (`desktop/src-tauri/src/history.rs`), phone `QueuePolicy.HISTORY_LIMIT = 500`.
- PC history JSON already has per item: `direction`, `kind`, `createdMs`, `totalBytes`, `files` (count), `names` (**first 3 only**), `text` (a 160-character preview), `isUrl`, `isLink`, `isAlert`, `title`.
- Phone history: `HistoryRepository` already loads **every** file row of the 500 newest transfers, with `name`, `finalName`, `mime` and `size` (`data/Daos.kt` `observeRecentFiles`). `HistoryItem` keeps only a `title` built from them.
- Phone already has a filter row, All / Sent / Received / **Failed** (`ui/HistoryView.kt` `HistoryFilter`, `ui/HistoryScreen.kt` `FilterRow`). This plan replaces it.
- The `ModalBottomSheet` pattern to copy: `android/.../ui/send/SendSheet.kt` (commit f8bf527).

Checked and needing no change: the hotkey only sends the clipboard (`app.rs` `register_hotkey`); toast clicks open files, folders and links, never a Kin window (`register_toast_clicks`); "New device paired" opens the Pairing window, which stays separate; `--dev-dump-view` reads `Core` directly, not a window.

## Contracts

### C1. The page host (owned by lane A, in `desktop/src/shell.ts`)

Lane A replaces `mountShell(root, active)` with one shell that holds three pages. Each page module exports a start function that gets a host:

```ts
export type Page = "home" | "history" | "settings";
export type PageHost = {
  acts: HTMLElement;   // this page's buttons on the right of the heading (swapped with the page)
  sub: HTMLElement;    // this page's line under the heading
  body: HTMLElement;   // this page's content
  setPc: (pcName: string, version: string) => void;
  visible: () => boolean;
  go: (page: Page) => void;                 // show another page (Home's "See all" uses it)
  onShow: (run: () => void) => void;        // called each time the page is shown, also the first time
  onHide: (run: () => void) => void;
  every: (ms: number, run: () => void) => void; // a timer that only runs while the page is visible
};
export function startDropPage(host: PageHost): void;     // drop.ts (file name kept)
export function startHistoryPage(host: PageHost): void;  // history.ts
export function startSettingsPage(host: PageHost): void; // settings.ts
```

Lane B codes its History changes against this type. Nobody but lane A changes it.

### C2. The main window (owned by lane A, Rust)

- Label `main` (`ui::MAIN_WINDOW`), URL `index.html?w=main&page=<home|history|settings>`, plus `#unlink=<deviceId>` for the tray's "Unlink phone…" (the hash format of `settings.ts` `unlinkFromHash` is unchanged). Title **"Kin"** for every page. Size and minimum stay `MAIN_SIZE` and `MAIN_MIN` in `ui.rs`.
- One Rust entry point, `ui::open_main(app, MainPage, unlink: Option<&str>)`. If the window exists it is un-minimized, shown, focused, and sent the event `navigate` with payload `{ "page": "home" | "history" | "settings" }` (and `unlink-requested` as today for Unlink). If not, it is built with the URL above.
- The events `transfers-changed`, `media-changed`, `history-changed` and `settings-changed` all go to `main`. `appearance-changed` stays a broadcast.

### C3. History types (shared rule for lanes B and C, no shared code)

Both apps classify with the same extension table. Each app holds its own copy as data: PC in `history.rs` (`const FILE_TYPES`), phone in a new `history/HistoryTypes.kt`. Both lanes' tests use the sample names listed here so the two copies stay equal.

| Type | Extensions (lower case) | Phone also by MIME |
|---|---|---|
| Photos | jpg jpeg png gif webp heic heif bmp avif tif tiff | `image/*` |
| Videos | mp4 mov mkv webm avi m4v 3gp wmv | `video/*` |
| Documents | pdf doc docx odt rtf txt md xls xlsx ods csv ppt pptx odp epub | none |
| Apps | apk apks xapk | `application/vnd.android.package-archive` |
| Other files | everything else, and no extension | |
| Texts | a text item that is not a link (alerts count as Texts) | |
| Links | PC: `isLink` or `isUrl`. Phone: `kind == link` or `isUrl` | |

Sample names both test suites check: `IMG_1.JPG` Photos, `clip.MOV` Videos, `Report.pdf` Documents, `kin.apk` Apps, `song.mp3` Other, `archive.tar.gz` Other, `README` Other, `photo.jpeg.exe` Other.

Rules, the same on both apps:
- A transfer matches a type if **any** of its files is of that type. Several types selected: an item matching **any** of them is shown. Type, direction and search combine with AND.
- Direction: Both (default), Sent, Received. Failed and waiting items count in their direction.
- Search: case-insensitive "contains". PC searches the file names it is sent (C4), the text preview and the link or alert title. The phone searches all file names, the full text and the title (it has them in memory already).
- Sort Newest (default): as today. Oldest: oldest first. Name: by the first file name, or by what a text row shows (a link's title, else its address), case-insensitive and locale-aware, ties newest first. Size: largest first by the transfer's total bytes; **texts and links go after all files**, newest first among themselves.
- Chips: one per selected type ("Photos"), one for a direction other than Both ("Sent"), one for a sort other than Newest ("Oldest"), and the search words if any. The chip's x resets that one choice. With two or more chips, a "Clear" chip resets all.
- Nothing matches: "Nothing matches." and a "Clear filters" button.
- The choices live in memory: on the PC for as long as the window is open (switching pages keeps them), on the phone in `rememberSaveable` (survives rotation, not a restart). They are never saved to disk.

### C4. New fields in the PC's `history_view` JSON (owned by lane B)

- `types`: a sorted list of the C3 types of the item, from **all** its files (`"photo"`, `"video"`, `"document"`, `"app"`, `"other"`), or `["text"]` or `["link"]` for a text item. Computed in Rust because the page never sees every file name.
- `names`: raised from the first 3 to the first `NAMES_PER_ITEM = 20` (a `const` in `history.rs`). A transfer can hold up to 10,000 files (`transfers::MAX_FILES`), so the list stays capped. The row still shows only the first name or "N files".
- No per-file size or MIME: Size sorts by `totalBytes`, which is already there, and the PC database stores no MIME (`queuedb::HistoryFile`).

The phone needs no database change and no migration. `HistoryItem` gains three fields, with defaults so other callers keep compiling: `names: List<String>` (all names, in order), `types: Set<HistoryType>` and `isLink: Boolean`.

## Tracks

Keep all three lanes away from the files of the concurrent `probe` work: `desktop/src-tauri/src/messages.rs`, `ws.rs`, `android/.../link/*` and `docs/protocol.md`. Real-app checks use a test Kin from `tools/kin-dev.ps1` and the emulator, never the installed Kin (D17, D33 in `notes/docs/decisions.md`). Run cargo with `CARGO_BUILD_JOBS=2` when two lanes build at the same time.

### Lane A. Desktop: one main window

- **Runs it:** one agent, one worktree, branch `t3/one-window`. Sonnet, effort high, then an Opus review of the capability and `only(...)` diff before merge (it is a security boundary).
- **Owns:** `desktop/src-tauri/src/ui.rs`; the window call sites in `desktop/src-tauri/src/app.rs` (lines near 337, 703 and 752 to 754, the `OpenTarget::Main` hook and `handle_commands`) and nothing else in that file; `desktop/src-tauri/src/tray.rs` (left click and doc comments); `desktop/src-tauri/capabilities/*`; the "window security" section of `desktop/src-tauri/tests/review_1_4.rs`; `desktop/src/main.ts`, `shell.ts`, `drop.ts`, `settings.ts`; in `desktop/src/history.ts` **only** `startHistoryWindow` becoming `startHistoryPage(host)` (the `mountShell` call, the `setInterval` and the `listen` lines); in `desktop/src/styles.css` only the shell block (the rules under the comment `/* The three windows of the sidebar ... */`, near line 120); `desktop/capture-windows.ps1`, `desktop/measure-idle.ps1`, `desktop/check-dragdrop.ps1`; `docs/windows.md`, `desktop/README.md`; a new decision in `notes/docs/decisions.md` (its own git repo).
- **Must not touch:** `history.rs`, the `Item` type, `draw()` and `row()` in `history.ts`, `sprite.ts`, `icons.ts`, `dom.ts`, `build.rs` (no command is added or removed), `package.json`, `.github/workflows/ci.yml`, anything in `android/`, and the probe files above.
- **Read first:** `docs/windows.md` sections "Process model", "Tray", "Sending to the phone", "Window security"; `ui.rs` in full; `review_1_4.rs` lines 494 to 584.

Tasks:

1. **Rust: one main window.** In `ui.rs`: add `MAIN_WINDOW = "main"`, `enum MainPage { Home, History, Settings }` and `open_main` (C2), honouring `--dev-no-windows`. Remove `DROP_WINDOW`, `HISTORY_WINDOW`, `SETTINGS_WINDOW`, `open_drop_window`, `open_history_window`, `open_settings_window`, `open_unlink`, `placement_of`, the `replacing` path of `open_page_at`, and the drop/history/settings branches of `open_window`, which keeps only `"pairing"` and `"debug"`. Every `only(...)` list that named drop, history or settings names `MAIN_WINDOW` instead; `remove_device` becomes `[PAIRING_WINDOW, MAIN_WINDOW]`; the pairing-only and debug-only commands are untouched. `pick_and_send` and `choose_received_folder` take the main window as the dialog owner. `transfers_changed` and `lists_changed` emit to `main`. Callers: tray left click, `OpenTarget::Main` (the extension's "Open Kin"), `Command::Drop/History/Settings` (`--drop`, `--history`, `--settings`, also when a second `Kin.exe` hands them over), and the tray's History, Settings and Unlink items all call `open_main`. The window is still **destroyed** on close (windows.md "Process model": an idle WebView2 costs tens of MB), and the app stays in the tray through the existing `prevent_exit`. **Test:** unit tests in `ui.rs` for the URL (`index.html?w=main&page=settings#unlink=abc`) and `MainPage` parsing; `cargo test` passes.
2. **Capabilities and their guard.** Delete `drop.json`, `history.json`, `settings.json`; add `main.json` (`"windows": ["main"]`) with the union of their permissions. `pairing.json` and `debug.json` do not change. In `review_1_4.rs`: the window list becomes `pairing`, `main`, `debug`; the page source of `main` is every `desktop/src/*.ts` except `pairing.ts` and `debug.ts` (so new History files are covered with no further edit); add asserts that the three old capability files are gone, that `pairing.json` and `debug.json` hold exactly today's command sets, and a table test that reads each command's `only(&window, &[...])` list in `ui.rs` and compares it to an expected label set (pairing and debug commands keep their exact lists, no list names a removed label). The count of 37 commands stays. **Test:** `cargo test --test review_1_4`.
3. **Page host and in-place pages.** `shell.ts` builds the sidebar once and implements C1: a page is mounted the first time it is shown (Home at start, since it holds the drop target), then hidden and shown with the `hidden` attribute. Each page has its own `acts` and `sub` (the heading's buttons and line are swapped with the page), the heading text and the sidebar's `on` and `aria-current` follow. The sidebar calls `go()`, not `open_window`. `main.ts` handles `?w=main&page=...` and listens to `navigate`. `drop.ts`, `history.ts`, `settings.ts` become `start*Page(host)`. Home's "See all" calls `host.go("history")`. **Test:** `bun run build` (tsc and vite) in `desktop/` passes.
4. **Hidden pages go quiet.** Every `window.setInterval` of the three pages becomes `host.every` (Home 700 ms, History 3 s); each page refreshes in `onShow`; event handlers refresh only when `host.visible()` (the `onShow` refresh covers what was missed). Settings keeps applying `unlink-requested` while hidden and shows it on show. Home's `onHide` closes its Send menu and the Send-a-text dialog. File drops: `onDragDropEvent` stays in `drop.ts`; a drag that enters while another page shows switches to Home, so the person sees the drop zone and the progress. (Rejected: dropping anywhere sends without switching, because the person would not see the progress.) **Test:** a dev Kin on the History page: its log shows no `drop_view` calls for 10 s; `measure-idle.ps1` CPU on each page is no higher than today's drop window.
5. **Scripts.** `capture-windows.ps1`, `measure-idle.ps1` and `check-dragdrop.ps1` look the main window up by the title `Kin` (Pair new phone and Debug unchanged), and after `--history` or `--settings` wait for the page to switch before the screenshot. `check-dragdrop.ps1` also runs once with the window on History. `tools/` has no script that finds a desktop window by label or title (checked), so it needs no change. **Test:** the three scripts pass against a dev Kin; `capture-windows.ps1` screenshots of the three pages in Warm and Simple, light and dark.
6. **Docs.** `docs/windows.md`: "Process model", "Tray" and "Sending to the phone" say "the main window (Home)", the extension's `open main` row, "Window security" lists `pairing`, `main`, `debug`, and "UI". `desktop/README.md` layout lines. A decision D35 in `notes/docs/decisions.md`: one main window, pages swapped in place, Pairing and Debug separate, destroyed on close. **Test:** read-through; no em dash in any UI string (`grep` the changed `.ts` strings).

**Exit test:** a stranger runs a dev Kin, left-clicks the tray, clicks Home, History, Settings, History: no new window, no flicker, History's scroll position and the Settings fields are as left. `Kin.exe --history`, `--settings`, `--drop` and the tray's Unlink switch the open window's page. Pairing and Debug still open as their own windows. `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` (in `desktop/src-tauri`) and `bun run build` (in `desktop/`) pass.

### Lane B. Desktop: History filter and sort

- **Runs it:** one agent, one worktree, branch `t3/desktop-history-filters`. Sonnet, effort high.
- **Owns:** `desktop/src-tauri/src/history.rs`; new `desktop/src/historyFilter.ts` (pure logic, no DOM, no Tauri import); new `desktop/src/historyFilterPanel.ts` (the Filter button, popover, search box and chips); new `desktop/tests/historyFilter.test.ts`; `desktop/src/sprite.ts` (add `i-filter` and `i-search` symbols); in `desktop/src/history.ts` the `Item` type, `draw()` and `row()`, plus the one mount line in `startHistoryPage`; a block appended at the **end** of `desktop/src/styles.css` under the comment `/* History filters */`; `.github/workflows/ci.yml` (one line: `bun test tests` after `bun run build` in the rust job); one paragraph in `docs/windows.md` (after A merges).
- **Must not touch:** `ui.rs`, `app.rs`, `tray.rs`, `build.rs`, `capabilities/*`, `review_1_4.rs`, `shell.ts`, `main.ts`, `drop.ts`, `settings.ts`, `dom.ts`, the shell block of `styles.css`, anything in `android/`, and the probe files. No new Tauri command: filtering is client-side.
- **Read first:** C1, C3, C4 above; `history.rs` and `history.ts` in full; the `.pop` and `.chip` rules in `styles.css`; `dom.ts` `helpButton` (the popover pattern to copy: closes on outside click, Esc, focus leaving).

Tasks:

1. **Rust: `types` and more `names`.** `const FILE_TYPES` and `fn file_type(name) -> &'static str` in `history.rs`; `types` and the `NAMES_PER_ITEM = 20` cap in `history_item_json` (C4). **Test:** unit tests in `history.rs`: the C3 sample names; a transfer of `a.jpg` and `b.pdf` gives `["document", "photo"]`; a link text gives `["link"]`, an alert `["text"]`; a 30-file transfer gives 20 names. `cargo test --lib history`.
2. **Pure filter logic.** `historyFilter.ts`: the `Query` type (search, types, sort, direction), `applyQuery(items, query)`, `chips(query)` and `defaultQuery`, following C3 exactly. **Test:** `desktop/tests/historyFilter.test.ts` with `bun test tests` in `desktop/`: each type, any-file matching, OR within types and AND across groups, each sort including texts after files on Size and ties newest first, direction with failed items, search over names, preview and title, chip text, and no em dash in any string the module returns.
3. **The panel.** `historyFilterPanel.ts`: a "Filter" button (icon `filter`) for `host.acts`; a popover with a search field, the seven types as toggle chips, Sort as four choices, Direction as three; the chips row above the list; "Nothing matches." with "Clear filters". Theme tokens only (`--surface`, `--outline`, `--primary-container`, `--on-primary-container`, `--chip-container`, `--radius-*`), so both skins and modes follow. The search field and chips live **outside** the list that `draw()` rebuilds every 3 s, so typing never loses focus. Short labels only. **Test:** `bun run build` passes; screenshots of History with the popover open and two chips active in all four skin and mode pairs (needs lane A merged for `capture-windows.ps1`, or a manual capture of a dev Kin before that).
4. **Wire it in (after lane A is merged; rebase first).** In `startHistoryPage(host)`: `const filter = mountHistoryFilter(host.acts, shell area above the list, () => draw(last))`; in `draw()`: `const shown = applyQuery(view.items, filter.query())`, the empty states split into "Nothing here yet" (no items) and "Nothing matches." (filtered out). Add the `bun test tests` CI line. One paragraph "History: Filter" in `docs/windows.md`. **Test:** `cargo test --test review_1_4` still passes with the new files counted as the main window's page (they call no command); a dev Kin with mixed history: each filter, sort and chip works, and the choices survive a switch to Settings and back.

**Exit test:** on a dev Kin with photos, a video, a PDF, an APK, a zip, texts and links in both directions, every type, sort and direction gives the C3 result; chips remove one choice each; "Clear" resets all. `bun test tests`, `bun run build`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass.

### Lane C. Android: History filter and sort

- **Runs it:** one agent, one worktree, branch `t3/android-history-filters`. Sonnet, effort high.
- **Owns:** `android/app/src/main/java/app/kinrelay/android/history/HistoryModel.kt`; new `history/HistoryTypes.kt`; `ui/HistoryView.kt`; `ui/HistoryScreen.kt` (except `HistoryRow`'s signature); new `ui/HistoryFilterSheet.kt`; `ui/theme/KinIcons.kt` (add `Filter` and `Search`); tests `app/src/test/.../history/HistoryTest.kt` and `app/src/test/.../ui/HistoryViewTest.kt`; the "History" bullet of `docs/android.md` ("Queue, history, notifications").
- **Must not touch:** `ui/KinRoot.kt` (the `HistoryScreen(...)` parameters stay as they are), `HistoryRow`'s parameters (Home's "Recent" uses it), `data/*` (no schema change, no migration), `ui/send/*`, `link/*`, `docs/protocol.md`, anything in `desktop/`.
- **Read first:** C3 above; `HistoryModel.kt`, `HistoryView.kt`, `HistoryScreen.kt`, `ui/send/SendSheet.kt`, `ui/KinComponents.kt` (`KinChip`, `KinButton`, `KinCard`); `docs/android.md` line "History."

Tasks:

1. **Model.** `HistoryTypes.kt`: `enum class HistoryType { PHOTO, VIDEO, DOCUMENT, APP, OTHER, TEXT, LINK }` and `classify(name, mime)` per C3 (MIME first, then extension). `HistoryBuilder.fileItem` fills `names` (all, `finalName ?: name`, by `ordinal`) and `types`; `textItem` sets `isLink` (`kind == LINK || isUrl`) and `types`. **Test:** `HistoryTest`: the C3 sample names, MIME winning over a missing extension, a mixed transfer's set, links and alerts.
2. **Query.** In `HistoryView.kt` replace `HistoryFilter` with `data class HistoryQuery(search, types, sort, direction)` (`@Parcelize` or a `Saver`, for `rememberSaveable`), `fun apply(items, query)` and `fun chips(query)`. Name sort uses `java.text.Collator` (primary strength). Day groups only for Newest and Oldest; Name and Size show one card without day labels. **Test:** `HistoryViewTest`: replace `eachFilterKeepsWhatItSays` with cases for each type, OR and AND, each sort (texts after files on Size, ties newest first), direction with failed and waiting items, search over all names, full text and title; grouping off for Name and Size.
3. **Sheet and screen.** `HistoryFilterSheet.kt`: a `ModalBottomSheet` like `SendSheet` (`skipPartiallyExpanded = true`), with a search field, the seven types as toggle chips, Sort and Direction as single-choice chips, and a "Done" button. `HistoryScreen`: the old `FilterRow` goes; a "Filter" button (soft, small, icon `Filter`) beside the title; a chips row (`KinChip` with an x, plus "Clear" at two or more) under it; "Nothing matches." with "Clear filters". `KinTheme` colours and `KinTheme.style` shapes only. **Test:** add `HistoryFilterSheet` to the file list of `noUserVisibleStringOfTheNewScreensHasAnEmDash`; `.\gradlew.bat testDebugUnitTest`; emulator screenshots (`tools/emulator/`) of the sheet and the chips in Warm and Simple, light and dark.
4. **Docs.** The `docs/android.md` History bullet says what Filter does in two sentences. **Test:** read-through.

**Exit test:** on the emulator with mixed history, each filter, sort and direction gives the C3 result, the chips remove one choice each, rotation keeps the choices, Home's "Recent" looks as before. `.\gradlew.bat testDebugUnitTest lintDebug assembleDebug` passes.

## Dependencies

1. Lanes A and C start at once. Lane B starts at once on its tasks B1 to B3, which touch none of A's files.
2. Lane A merges into `ui/warm-simple` first.
3. Lane B rebases on it, does B4 (the wiring in `history.ts` and the `docs/windows.md` paragraph), then merges. The only overlap is `history.ts` (A changes the start function, B the `Item` type, `draw()`, `row()` and one mount line) and `styles.css` (A the shell block, B a block at the end).
4. Lane C merges whenever it is done; it shares no file with A or B.

## Risks

- **The main window may call more commands than any old window.** By design it holds the union. Check: lane A task 2's tests pin `pairing.json`, `debug.json` and the `only(...)` lists of their commands exactly, so a looser Pairing or Debug fails `cargo test`.
- **A hidden page keeps polling.** Home polls every 0.7 s. Check: lane A task 4 (no `drop_view` in the log while History shows; `measure-idle.ps1`).
- **A `navigate` sent while the window is still being built is lost** (the page is not listening yet). Today's `unlink-requested` has the same gap. Check: `check-dragdrop.ps1` or a script runs `Kin.exe --drop` then `--history` within 200 ms and expects History.
- **Scripts that wait on the old titles hang.** Check: run the three changed scripts in lane A task 5.
- **History redraw every 3 s steals the search field's focus.** Check: lane B task 4, typing in the search field while `--dev-history` changes an item.
- **The concurrent probe branch also edits `app.rs`.** Lane A's `app.rs` edits are the window call sites only. Check: `git diff` against the probe branch before merging; resolve by hand if both touched the same lines.

## Not in this plan

- Remembering the main window's size and place after it is closed: nothing remembers it today, and the in-place pages remove the only case that needed it (tab switches).
- Hiding the main window on close instead of destroying it: it contradicts the idle-memory rule in windows.md "Process model".
- Pairing and Debug as pages of the main window: Sam decided they stay separate.
- Filtering beyond the newest 500 or on the backend: Sam decided client-side over the loaded list.
- Saving filter choices across restarts: not asked for, and an empty filter is the expected start.
- A CHANGELOG entry: written at release time, not per lane.

## Open questions for Sam

1. **The phone's "Failed" tab goes away.** Today the phone has All / Sent / Received / Failed; the new panel has only Sent / Received / Both. *Recommendation:* drop it. Failed rows stay red with a Retry button and are easy to spot. If you want it, it becomes a fourth Direction choice on both apps, not a separate control.
2. **Where the search box sits.** *Recommendation:* inside the Filter panel, at the top, with the typed words shown as a chip while active. That keeps History's header to one button, as decided. The alternative is a search field always visible beside the Filter button: one tap less, but more on screen, especially on the phone.
