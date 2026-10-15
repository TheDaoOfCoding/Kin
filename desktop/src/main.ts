// Two windows share this page, told apart by `?w=` in the URL: the pairing window (the default) and the drop window of 1.3 (`?w=drop`).
// Later milestones add the others (history, settings).
import { startDropWindow } from "./drop";
import { startPairingWindow } from "./pairing";

const root = document.querySelector<HTMLElement>("#app");
if (root) {
  if (new URLSearchParams(window.location.search).get("w") === "drop") startDropWindow(root);
  else startPairingWindow(root);
}
