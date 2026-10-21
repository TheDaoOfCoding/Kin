# Kin browser extension

Send links, text, images and pages from your browser to your phone with Kin. One Manifest V3 codebase builds for Chromium browsers (Chrome, Edge, Brave, Vivaldi, Opera) and Firefox. It talks to the Kin desktop app through a native messaging host, see [docs/extension.md](../docs/extension.md) for the contract.

- **Right-click menu:** "Send to Kin" on selected text, links, images and pages (a submenu when several phones are paired).
- **Toolbar button:** a popup with Kin's and the phones' state, **Send this tab**, and the last 10 sends.
- **Shortcut:** `Alt+Shift+K` sends the current tab (change it in the browser's shortcut page).
- **YouTube:** the current video time is added to the link as `t=<seconds>s`.
- **Download alerts (off by default):** a switch in the popup, "Tell my phone when a download finishes". Only the file's name is sent, never the address or folder, never from a private window.
- **Permissions:** `contextMenus`, `nativeMessaging`, `activeTab`, `scripting`, `storage`. The optional permission `downloads` is asked only when the user turns the download alert switch on. No site access, no content scripts, no network.

## Build

Needs [bun](https://bun.sh) (the project was built with 1.4).

```
cd extension
bun install
bun run build            # release build: dist/chromium and dist/firefox (host app.kinrelay.desktop)
bun run build:dev        # dev build:     dist/chromium-dev and dist/firefox-dev (host app.kinrelay.dev)
bun run package          # build + the zips: dist/kin-chromium-<version>.zip (store), dist/kin-chromium-<version>-sideload.zip, dist/kin-firefox-<version>.zip
bun run typecheck
bun run test             # unit tests
bun run smoke            # headless Chromium smoke test against a mock host (see below)
```

There are two Chromium zips. `kin-chromium-<version>.zip` has no `key` in its manifest: it is for uploading to the Chrome Web Store and Edge Add-ons, which assign their own ID. `kin-chromium-<version>-sideload.zip` has the (public) `key`, so an unpacked copy always gets the extension ID `cbioefpddjpaionhljhffgbedncfchmn`, which is the one the host manifest's `allowed_origins` names. Without the key a side-loaded extension gets a random ID and Kin's native host refuses it. The unpacked `dist/chromium*` folders have the key too. `tools/build-extension-zips.ps1` builds all the zips and checks both: no key in the store zip, and the key in the side-load zip computes to the expected ID.

## Load it unpacked

The extension only does something when **Kin is installed**: the installer registers the native messaging host. (Until Kin's installer does that, the popup says "Kin isn't installed".)

**Chrome, Edge, Brave, Vivaldi, Opera** (from the release zip)

1. Unzip `kin-chromium-<version>-sideload.zip` into a folder you keep (the browser reads the extension from that folder, so don't delete it).
2. Open the extensions page: `chrome://extensions`, `edge://extensions`, `brave://extensions`, `vivaldi://extensions`, `opera://extensions`.
3. Turn on **Developer mode**, click **Load unpacked** and pick the unzipped folder (the one with `manifest.json` in it).
4. Pin the Kin button if you like (puzzle icon, then the pin).

From a checkout instead: `bun run build` (or `bun run build:dev` to talk to a dev Kin), then pick `extension/dist/chromium` (or `dist/chromium-dev`) in step 3.

**Firefox**

1. `bun run build`.
2. Open `about:debugging#/runtime/this-firefox`, click **Load Temporary Add-on…** and pick `extension/dist/firefox/manifest.json`.
3. A temporary add-on is removed when Firefox closes. A permanent install needs the signed `.xpi` from addons.mozilla.org (not published yet).

## The smoke test

`bun run smoke` builds the test flavor (`dist/chromium-test`: the test hooks in the background script and host permissions for `127.0.0.1` and a faked YouTube, so the test can inject into its own pages without a toolbar click, and `downloads` as a normal permission because a headless browser can't click its prompt), starts a **headless** Chromium with a **throwaway profile** and runs the extension against `tests/smoke/mock-host.ts`, a mock of the native host and Kin.

What it touches:

- the Playwright Chromium from `%LOCALAPPDATA%\ms-playwright` (install with `bunx playwright-core install chromium` if it's missing, no admin needed; or set `KIN_CHROMIUM` to a `chrome.exe`). It never starts your own browser or reads its profile;
- one registry key, `HKCU\Software\Chromium\NativeMessagingHosts\app.kinrelay.dev`, which points at the mock host, and is deleted when the run ends (also on failure and on Ctrl+C). The run refuses to start if that key already exists;
- a temp folder under `%TEMP%` (profile, mock host files, and the folder the test downloads land in), deleted at the end, and a web server on `127.0.0.1` with a random port.

It doesn't touch Kin, your own browsers, the Run key, SendTo, your Downloads folder or the firewall. Screenshots of the popup go to `dist/smoke-shots/` (git-ignored). The download alert steps include one real download that takes 31.5 s, so the whole run takes about two minutes. While developing, `KIN_SMOKE_ONLY="downloads: a"` (comma separated name prefixes) runs only the matching steps.

## Layout

```
src/background.ts      listeners (menu, shortcut, popup messages), menu rebuild
src/flow.ts            status and send flows, history, toolbar badge
src/downloads.ts       the download alert listener (registered only when the browser has `downloads`)
src/native.ts          the one place that calls sendNativeMessage
src/inject.ts          functions that run inside a page (video time, image fetch)
src/shared/            pure logic: validate, youtube, messages, menus, history, downloads (unit tested)
src/popup/             the toolbar popup
scripts/               build, package (zip), manifest, icon and key generators
tests/unit/            bun test; includes the mock host as a conformance check
tests/smoke/          headless Chromium run + mock-host.ts
assets/icons/         placeholder icons (bun scripts/gen-icons.ts)
manifest-key.json      the public dev key and the ID derived from it
```

## Manual check on Firefox

There is no headless Firefox run yet. To check by hand (needs the installed Kin, whose host registers a Firefox key, or a mock host registered under `HKCU\Software\Mozilla\NativeMessagingHosts\app.kinrelay.dev` with `allowed_extensions: ["kin@kinrelay.app"]`, using `bun run build:dev`):

1. `about:debugging#/runtime/this-firefox`, **Load Temporary Add-on…**, pick `extension/dist/firefox-dev/manifest.json`. It loads with no error and the Kin button shows in the toolbar.
2. Open the popup: it shows Kin's state within a second or two.
3. Right-click a link: "Send to Kin" is in the menu (a submenu with two phones). Choose it: the badge shows `✓` or `Q`.
4. Select some text, right-click, send. Right-click an image, send.
5. On a `youtube.com/watch?v=…` tab, play for about a minute, press the toolbar **Send this tab**: the phone's link has `t=<about 60>s`.
6. `Alt+Shift+K` sends the tab (`about:addons`, gear, **Manage Extension Shortcuts** shows or changes it).
7. Stop Kin, send: "Kin isn't running…". Click **Start Kin** in the popup.
