// The windows of the app share this page, told apart by `?w=` in the URL: the pairing window (the default), the main window (`?w=main&page=home|history|settings`,
// with Home, History and Settings as pages that its sidebar swaps in place) and the debug window.
import { listen } from "@tauri-apps/api/event";
import { startAppearance } from "./appearance";
import { startDebugWindow } from "./debug";
import { startDropPage } from "./drop";
import { startHistoryPage } from "./history";
import { installSprite } from "./icons";
import { startPairingWindow } from "./pairing";
import { requestUnlink, startSettingsPage, unlinkFromHash } from "./settings";
import { mountShell, parsePage } from "./shell";

function startMainWindow(root: HTMLElement): void {
  const params = new URLSearchParams(window.location.search);
  // The tray's "Unlink phone…" builds the window with `#unlink=<deviceId>`, or tells an open one with an event (the Settings page picks it up either way).
  const fromHash = unlinkFromHash(window.location.hash);
  if (fromHash) requestUnlink(fromHash);
  void listen<string>("unlink-requested", (event) => requestUnlink(event.payload));
  const shell = mountShell(root, { home: startDropPage, history: startHistoryPage, settings: startSettingsPage }, parsePage(params.get("page")) ?? "home");
  // The tray, `Kin.exe --history` and the like ask an open window to show another page.
  void listen<{ page?: string }>("navigate", (event) => {
    const page = parsePage(event.payload?.page);
    if (page) shell.go(page);
  });
}

const root = document.querySelector<HTMLElement>("#app");
if (root) {
  installSprite();
  startAppearance();
  switch (new URLSearchParams(window.location.search).get("w")) {
    case "main":
      startMainWindow(root);
      break;
    case "debug":
      startDebugWindow(root);
      break;
    default:
      startPairingWindow(root);
  }
}
