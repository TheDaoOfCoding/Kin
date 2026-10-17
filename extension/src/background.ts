// Entry point of the service worker (Chromium) / event page (Firefox).
// Every listener is registered at the top level, as Manifest V3 requires.
declare const __TEST__: boolean;
import { buildMenuModel, clickTarget, isKinMenuItem, MENU_CONTEXTS, menuDeviceId, type ClickInfo, type MenuSpec } from "./shared/menus";
import type { StatusInfo } from "./shared/protocol";
import {
  clearHistory,
  getCachedStatus,
  getHistory,
  getLastResult,
  getSelectedDevice,
  openKin,
  performSend,
  refreshStatus,
  sendTab,
  setSelectedDevice,
  setStatusListener,
  startKin,
  type SendOutcome,
  type SendSpec,
} from "./flow";
import { ext, setHostName } from "./native";

// ---- menu -----------------------------------------------------------------------------------

let menuChain: Promise<void> = Promise.resolve();
let currentSpecs: MenuSpec[] = [];
let currentKey = "";
const menuErrors: string[] = [];

function createItem(spec: MenuSpec): Promise<void> {
  return new Promise((resolve) => {
    ext.contextMenus.create(
      { id: spec.id, title: spec.title, contexts: [...MENU_CONTEXTS], ...(spec.parentId ? { parentId: spec.parentId } : {}) },
      () => {
        const e = ext.runtime.lastError;
        if (e) menuErrors.push(`${spec.id}: ${e.message}`);
        resolve();
      },
    );
  });
}

/** Rebuilds the menu for a status. Rebuilds are queued so two at once can't clash on item IDs. */
function rebuildMenu(status: StatusInfo | null): Promise<void> {
  const specs = buildMenuModel(status);
  const key = JSON.stringify(specs);
  menuChain = menuChain
    .then(async () => {
      if (key === currentKey) return;
      await ext.contextMenus.removeAll();
      for (const spec of specs) await createItem(spec);
      currentSpecs = specs;
      currentKey = key;
    })
    .catch(() => {
      currentKey = "";
    });
  return menuChain;
}

setStatusListener((status) => {
  void rebuildMenu(status);
});

// ---- events ---------------------------------------------------------------------------------

async function onMenuClick(info: ClickInfo & { menuItemId: string | number }, tab?: chrome.tabs.Tab): Promise<SendOutcome | undefined> {
  if (!isKinMenuItem(info.menuItemId)) return undefined;
  const target = clickTarget(info);
  const deviceId = menuDeviceId(info.menuItemId);
  const ctx = {
    ...(tab ? { tab } : {}),
    ...(info.frameId !== undefined ? { frameId: info.frameId } : {}),
    ...(deviceId ? { deviceId } : {}),
  };
  let spec: SendSpec;
  switch (target.kind) {
    case "image":
      spec = { kind: "image", srcUrl: target.srcUrl, ...(info.pageUrl ? { pageUrl: info.pageUrl } : {}) };
      break;
    case "link":
      spec = { kind: "link", url: target.linkUrl, ...(target.linkText ? { title: target.linkText } : {}) };
      break;
    case "text":
      spec = { kind: "text", text: target.text };
      break;
    case "page":
      spec = { kind: "page", url: tab?.url ?? info.pageUrl ?? "", ...(tab?.title ? { title: tab.title } : {}) };
      break;
  }
  return performSend(spec, ctx);
}

ext.contextMenus.onClicked.addListener((info, tab) => {
  void onMenuClick(info as ClickInfo & { menuItemId: string | number }, tab);
});

ext.commands.onCommand.addListener((command, tab) => {
  if (command === "send-tab") void sendTab(tab);
});

ext.runtime.onInstalled.addListener(() => {
  void rebuildMenu(null).then(() => refreshStatus());
});

ext.runtime.onStartup.addListener(() => {
  void refreshStatus();
});

// ---- popup ----------------------------------------------------------------------------------

type PopupMessage =
  | { kin: "state" }
  | { kin: "start" }
  | { kin: "open"; target: "main" | "pairing" }
  | { kin: "sendTab"; deviceId?: string }
  | { kin: "select"; deviceId?: string }
  | { kin: "clear" };

async function popupState() {
  const view = await refreshStatus();
  return {
    view,
    selectedDeviceId: await getSelectedDevice(),
    history: await getHistory(),
    lastResult: await getLastResult(),
  };
}

async function handlePopup(msg: PopupMessage): Promise<unknown> {
  switch (msg.kin) {
    case "state":
      return popupState();
    case "start":
      return { view: await startKin() };
    case "open": {
      const r = await openKin(msg.target);
      return { ok: r.ok, code: r.code, message: r.message };
    }
    case "sendTab": {
      const outcome = await sendTab(undefined, msg.deviceId);
      return { outcome, history: await getHistory(), status: await getCachedStatus() };
    }
    case "select":
      await setSelectedDevice(msg.deviceId);
      return { ok: true };
    case "clear":
      await clearHistory();
      return { ok: true };
  }
}

ext.runtime.onMessage.addListener((raw, _sender, sendResponse) => {
  const msg = raw as PopupMessage | undefined;
  if (!msg || typeof msg !== "object" || typeof (msg as { kin?: unknown }).kin !== "string") return false;
  handlePopup(msg).then(sendResponse, () => sendResponse({ error: true }));
  return true;
});

// ---- test hooks (only in the test build) ----------------------------------------------------

if (__TEST__) {
  (globalThis as Record<string, unknown>).__kin = {
    menuClick: onMenuClick,
    sendTab,
    refreshStatus,
    startKin,
    getMenu: () => currentSpecs,
    getMenuErrors: () => menuErrors,
    setHostName,
    handlePopup,
  };
}
