// "Tell my phone when a download finishes" (2B.5): the wiring of src/shared/downloads.ts to the
// browser. The permission `downloads` is optional and asked only when the user turns the switch on,
// so without it `chrome.downloads` doesn't exist and nothing here is registered.
import { getDownloadSettings, sendDownloadAlert } from "./flow";
import { ext } from "./native";
import { createDownloadAlerter, createRegistrar, factsFromItem, type DownloadsApi } from "./shared/downloads";

type BrowserDownloads = DownloadsApi & { search(query: { id: number }): Promise<unknown[]> };

/** `undefined` while the permission isn't granted. */
function api(): BrowserDownloads | undefined {
  return (ext as unknown as { downloads?: BrowserDownloads }).downloads;
}

const alerter = createDownloadAlerter({
  settings: getDownloadSettings,
  lookup: async (id) => factsFromItem((await api()?.search({ id }))?.[0]),
  schedule: (fn, ms) => setTimeout(fn, ms),
  deliver: async (alert) => {
    await sendDownloadAlert(alert);
  },
  // The permission can be held while the switch is off (a browser that wouldn't take it back): don't wake the worker for every download.
  disabled: () => registrar.release(),
});

const registrar = createRegistrar((delta) => void alerter.onChanged(delta));

/** Called at the top level of the worker (an event must be registered on its first turn) and again when the permission arrives. */
export function registerDownloads(): boolean {
  return registrar.register(api());
}

/** At the worker's start: the registration has to be synchronous, so the switch is read after it, and a listener that has no use is taken back. */
export async function releaseDownloadsIfOff(): Promise<void> {
  try {
    if (!(await getDownloadSettings()).enabled) registrar.release();
  } catch {
    // keep the listener; it checks the switch itself on its first event
  }
}

export function releaseDownloads(): void {
  registrar.release();
}

export function downloadsRegistered(): boolean {
  return registrar.registered;
}

/**
 * Whether this browser has a downloads API to offer at all. Without the permission `downloads` is undefined everywhere, so the permission can't tell; the platform can:
 * Firefox for Android has no `downloads` API (the permission can't be granted there), so the popup hides the switch instead of offering one that can never work.
 */
export async function downloadsSupported(): Promise<boolean> {
  try {
    const info = await ext.runtime.getPlatformInfo();
    return info.os !== "android";
  } catch {
    return true;
  }
}

export async function downloadsGranted(): Promise<boolean> {
  try {
    return await ext.permissions.contains({ permissions: ["downloads"] });
  } catch {
    return false;
  }
}

/** The switch's off side: forget the permission too, so the browser shows Kin as having no access to downloads. */
export async function dropDownloadsPermission(): Promise<void> {
  try {
    await ext.permissions.remove({ permissions: ["downloads"] });
  } catch {
    // a build where it is a normal permission (the test build) can't remove it; the setting is what counts
  }
}
