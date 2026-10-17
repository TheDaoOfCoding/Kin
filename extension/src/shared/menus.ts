// The right-click menu as data, so the logic is testable without a browser.
import type { DeviceInfo, StatusInfo } from "./protocol";

export const MENU_ROOT = "kin:root";
export const MENU_SINGLE = "kin:send";
export const MENU_DEVICE_PREFIX = "kin:dev:";
export const MENU_CONTEXTS = ["selection", "link", "image", "page"] as const;

export interface MenuSpec {
  id: string;
  title: string;
  parentId?: string;
}

/** Menu titles treat & as an access key marker, so a literal one is doubled. */
export function escapeMenuTitle(s: string): string {
  return s.replace(/&/g, "&&");
}

export function deviceLabel(d: DeviceInfo): string {
  const name = escapeMenuTitle(d.name);
  if (!d.online) return `${name} (offline, will queue)`;
  return d.queuedCount > 0 ? `${name} (${d.queuedCount} waiting)` : name;
}

/**
 * What the menu should look like for the last known status.
 *  - unknown or Kin not running: one plain item (a click starts Kin, then sends)
 *  - Kin running, nothing paired: one item that says so (a click explains)
 *  - one device: one item, with the state if it is offline
 *  - several devices: "Send to Kin" with one entry per device
 */
export function buildMenuModel(status: StatusInfo | null | undefined): MenuSpec[] {
  if (!status || !status.running) return [{ id: MENU_SINGLE, title: "Send to Kin" }];
  const devices = status.devices;
  if (devices.length === 0) return [{ id: MENU_SINGLE, title: "Send to Kin (pair your phone first)" }];
  if (devices.length === 1) {
    const d = devices[0]!;
    return [{ id: MENU_SINGLE, title: d.online ? "Send to Kin" : `Send to Kin (${escapeMenuTitle(d.name)} is offline, will queue)` }];
  }
  return [
    { id: MENU_ROOT, title: "Send to Kin" },
    ...devices.map((d) => ({ id: MENU_DEVICE_PREFIX + d.id, title: deviceLabel(d), parentId: MENU_ROOT })),
  ];
}

/** The device a clicked menu item stands for. undefined means "let Kin choose". */
export function menuDeviceId(menuItemId: string | number): string | undefined {
  const id = String(menuItemId);
  return id.startsWith(MENU_DEVICE_PREFIX) ? id.slice(MENU_DEVICE_PREFIX.length) : undefined;
}

export function isKinMenuItem(menuItemId: string | number): boolean {
  const id = String(menuItemId);
  return id === MENU_SINGLE || id.startsWith(MENU_DEVICE_PREFIX);
}

export interface ClickInfo {
  srcUrl?: string;
  mediaType?: string;
  linkUrl?: string;
  linkText?: string;
  selectionText?: string;
  pageUrl?: string;
  frameId?: number;
}

export type ClickTarget =
  | { kind: "image"; srcUrl: string }
  | { kind: "link"; linkUrl: string; linkText?: string }
  | { kind: "text"; text: string }
  | { kind: "page" };

/**
 * What a click was on. An image beats the link around it (the menu item was shown on the image),
 * a link beats a selection, a selection beats the page.
 */
export function clickTarget(info: ClickInfo): ClickTarget {
  if (info.srcUrl && (info.mediaType === "image" || info.mediaType === undefined)) return { kind: "image", srcUrl: info.srcUrl };
  if (info.linkUrl) return { kind: "link", linkUrl: info.linkUrl, ...(info.linkText ? { linkText: info.linkText } : {}) };
  if (info.selectionText && info.selectionText !== "") return { kind: "text", text: info.selectionText };
  return { kind: "page" };
}

/** The device a send without an explicit choice should use: the saved choice if it still exists. */
export function pickDevice(status: StatusInfo | null | undefined, selectedId: string | undefined): string | undefined {
  if (!status || status.devices.length === 0) return undefined;
  if (selectedId && status.devices.some((d) => d.id === selectedId)) return selectedId;
  if (status.defaultDeviceId && status.devices.some((d) => d.id === status.defaultDeviceId)) return status.defaultDeviceId;
  return status.devices[0]!.id;
}
