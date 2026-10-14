// The only window of milestone 1.1 is the pairing window. Later milestones add the others (drop window, history, settings).
import { startPairingWindow } from "./pairing";

const root = document.querySelector<HTMLElement>("#app");
if (root) startPairingWindow(root);
