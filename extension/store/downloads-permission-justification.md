# Store justification: the optional `downloads` permission

For the permission field of the Chrome Web Store, Edge Add-ons and addons.mozilla.org listing of **Kin Relay** (`kin-chromium-<version>.zip`, `kin-firefox-<version>.zip`,
built by `tools/build-extension-zips.ps1`). Paste the text as it is; the facts behind each sentence are in [docs/extension.md](../../docs/extension.md) ("Download alerts") and
are held by unit tests.

## Short text (Chrome Web Store "permission justification", Edge "notes for certification")

> `downloads` is an **optional** permission. Kin Relay asks for it only when the user turns on "Tell my phone when a download finishes" in the extension's popup, never at install,
> and it is removed again when the user turns the switch off. With it the extension learns that a download has finished (or failed) and its file name, so it can tell the user's own
> phone through the Kin Relay app on the user's own PC. It never reads the address of a download or the page it came from, never reads or opens the downloaded file, never acts on
> downloads from private windows, keeps no address or page of a download, and sends nothing to any server: the message goes to the Kin app on the same computer (native messaging) and from there to the user's phone over
> their own network.

## Longer text (if a reviewer asks for more)

* **What the permission is used for:** exactly one thing. The extension listens to `downloads.onChanged`; when a download reaches `complete` (or `interrupted`, unless the user
  cancelled it) the extension reads the item's `state`, `error`, `incognito`, `filename` and start and end times, and sends the phone one notification: "Download finished" with the
  file's **base name** (the folder part is cut off in the extension, at most 120 characters) or "Download failed". Several downloads that finish within 3 seconds are one notification.
* **What it does not use:** to read those facts the extension asks the browser for the one item that just changed (`downloads.search({ id })`), and the browser hands over the whole
  item; the extension keeps only the fields above. The item's `url`, `finalUrl` and `referrer` are never read, stored or sent (a unit test fails if the code mentions them). No download is
  started, paused, cancelled, opened or removed, and the list of past downloads is never searched.
* **When it is on:** off by default. The switch asks for the permission from the click (a user gesture); turning the switch off takes the listener back and removes the permission.
  While the switch is off the extension registers no download listener at all.
* **Who sees the data:** nobody but the user's devices. The extension has no host permissions and no network access of its own. The notification goes to the Kin Relay app on the same
  computer through Chrome's / Firefox's native messaging, and from there to the user's own paired phone over the local network. There is no account, no telemetry and no server of ours.
* **Private windows:** downloads made in a private (incognito) window are skipped, and the extension can only see them at all if the user allowed it in private windows.
* **Why it has to be a permission:** the only way to learn that a download finished is the `downloads` API. Without the permission `chrome.downloads` does not exist in the browser.

## Firefox (addons.mozilla.org)

The same text. The manifest declares `data_collection_permissions: { required: ["none"] }`. Firefox for Android has no `downloads` API: the extension hides the switch there
(`downloadsSupported()`), and Firefox shows its own permission prompt text for the optional permission.

## Still to check on real browsers (not covered by an automated test)

The permission prompt text of each browser, and an idle service worker or Firefox event page waking for a finished download, have not been checked on every real browser.
Until they have, this text states what the code does, not that every store reviewer's browser was tried.
