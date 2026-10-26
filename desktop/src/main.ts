// The windows of the app share this page, told apart by `?w=` in the URL: the pairing window (the default), the drop window of 1.3
// (`?w=drop`) and the history, settings and debug windows of 1.4.
import { startAppearance } from "./appearance";
import { startDebugWindow } from "./debug";
import { startDropWindow } from "./drop";
import { startHistoryWindow } from "./history";
import { installSprite } from "./icons";
import { startPairingWindow } from "./pairing";
import { startSettingsWindow } from "./settings";

const root = document.querySelector<HTMLElement>("#app");
if (root) {
  installSprite();
  startAppearance();
  switch (new URLSearchParams(window.location.search).get("w")) {
    case "drop":
      startDropWindow(root);
      break;
    case "history":
      startHistoryWindow(root);
      break;
    case "settings":
      startSettingsWindow(root);
      break;
    case "debug":
      startDebugWindow(root);
      break;
    default:
      startPairingWindow(root);
  }
}
