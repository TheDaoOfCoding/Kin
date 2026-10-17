# Kin browser extension

Send links, text, images and pages from your browser to your phone with Kin. One Manifest V3 codebase builds for Chromium browsers (Chrome, Edge, Brave, Vivaldi, Opera) and Firefox. It talks to the Kin desktop app through a native messaging host, see [docs/extension.md](../docs/extension.md) for the contract.

- **Right-click menu:** "Send to Kin" on selected text, links, images and pages (a submenu when several phones are paired).
- **Toolbar button:** a popup with Kin's and the phones' state, **Send this tab**, and the last 10 sends.
- **Shortcut:** `Alt+Shift+K` sends the current tab (change it in the browser's shortcut page).
- **YouTube:** the current video time is added to the link as `t=<seconds>s`.
- **Permissions:** `contextMenus`, `nativeMessaging`, `activeTab`, `scripting`, `storage`. No site access, no content scripts, no network.

## Build

Needs [bun](https://bun.sh) (the project was built with 1.4).

```
cd extension
bun install
bun run build            # release build: dist/chromium and dist/firefox (host app.kinrelay.desktop)
bun run build:dev        # dev build:     dist/chromium-dev and dist/firefox-dev (host app.kinrelay.dev)
bun run package          # build + the store zips: dist/kin-chromium-<version>.zip, dist/kin-firefox-<version>.zip
bun run typecheck
bun run test             # unit tests
bun run smoke            # headless Chromium smoke test against a mock host (see below)
```

The Chromium zip has no `key` in its manifest (the stores assign their own ID). The unpacked `dist/chromium*` folders have it, so they always get the extension ID `cbioefpddjpaionhljhffgbedncfchmn`, which is the one the host manifest's `allowed_origins` names.

## Load it unpacked

The extension only does something when **Kin is installed**: the installer registers the native messaging host. (Until Kin's installer does that, the popup says "Kin isn't installed".)

**Chrome, Edge, Brave, Vivaldi, Opera**

1. `bun run build` (or `bun run build:dev` to talk to a dev Kin).
2. Open the extensions page: `chrome://extensions`, `edge://extensions`, `brave://extensions`, `vivaldi://extensions`, `opera://extensions`.
3. Turn on **Developer mode**, click **Load unpacked** and pick `extension/dist/chromium` (or `dist/chromium-dev`).
4. Pin the Kin button if you like (puzzle icon, then the pin).

**Firefox**

1. `bun run build`.
2. Open `about:debugging#/runtime/this-firefox`, click **Load Temporary Add-on…** and pick `extension/dist/firefox/manifest.json`.
3. A temporary add-on is removed when Firefox closes. A permanent install needs the signed `.xpi` from addons.mozilla.org (not published yet).

## The smoke test

`bun run smoke` builds the test flavor (`dist/chromium-test`: the test hooks in the background script and host permissions for `127.0.0.1` and a faked YouTube, so the test can inject into its own pages without a toolbar click), starts a **headless** Chromium with a **throwaway profile** and runs the extension against `tests/smoke/mock-host.ts`, a mock of the native host and Kin.

What it touches:

- the Playwright Chromium from `%LOCALAPPDATA%\ms-playwright` (install with `bunx playwright-core install chromium` if it's missing, no admin needed; or set `KIN_CHROMIUM` to a `chrome.exe`). It never starts your own browser or reads its profile;
- one registry key, `HKCU\Software\Chromium\NativeMessagingHosts\app.kinrelay.dev`, which points at the mock host, and is deleted when the run ends (also on failure and on Ctrl+C). The run refuses to start if that key already exists;
- a temp folder under `%TEMP%` (profile, mock host files), deleted at the end, and a web server on `127.0.0.1` with a random port.

It doesn't touch Kin, the user's browsers, Run key, SendTo, Downloads or the firewall. Screenshots of the popup go to `docs/results/phase-2b-1-extension-screens/`.

## Layout

```
src/background.ts      listeners (menu, shortcut, popup messages), menu rebuild
src/flow.ts            status and send flows, history, toolbar badge
src/native.ts          the one place that calls sendNativeMessage
src/inject.ts          functions that run inside a page (video time, image fetch)
src/shared/            pure logic: validate, youtube, messages, menus, history (unit tested)
src/popup/             the toolbar popup
scripts/               build, package (zip), manifest, icon and key generators
tests/unit/            bun test; includes the mock host as a conformance check
tests/smoke/          headless Chromium run + mock-host.ts
assets/icons/         placeholder icons (bun scripts/gen-icons.ts)
manifest-key.json      the public dev key and the ID derived from it
```

## Manual check on Firefox

There is no headless Firefox run yet; see the checklist in [docs/results/phase-2b-1-extension.md](../docs/results/phase-2b-1-extension.md#firefox-manual-checklist).
