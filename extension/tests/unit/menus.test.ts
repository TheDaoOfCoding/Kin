import { describe, expect, test } from "bun:test";
import { buildMenuModel, clickTarget, escapeMenuTitle, isKinMenuItem, menuDeviceId, pickDevice, MENU_ROOT, MENU_SINGLE } from "../../src/shared/menus";
import type { StatusInfo } from "../../src/shared/protocol";

const dev = (id: string, name: string, online: boolean, queuedCount = 0) => ({ id, name, online, queuedCount });
const status = (devices: ReturnType<typeof dev>[], defaultDeviceId?: string): StatusInfo => ({ running: true, devices, ...(defaultDeviceId ? { defaultDeviceId } : {}) });

describe("buildMenuModel", () => {
  test("unknown status or Kin not running: one plain item", () => {
    expect(buildMenuModel(null)).toEqual([{ id: MENU_SINGLE, title: "Send to Kin" }]);
    expect(buildMenuModel({ running: false, devices: [] })).toEqual([{ id: MENU_SINGLE, title: "Send to Kin" }]);
  });
  test("running, nobody paired", () => {
    expect(buildMenuModel(status([]))[0]!.title).toContain("pair your phone");
  });
  test("one online device: a single item", () => {
    expect(buildMenuModel(status([dev("a", "Pixel 8", true)]))).toEqual([{ id: MENU_SINGLE, title: "Send to Kin" }]);
  });
  test("one offline device: the item says it will queue", () => {
    const m = buildMenuModel(status([dev("a", "Pixel 8", false)]));
    expect(m).toHaveLength(1);
    expect(m[0]!.title).toBe("Send to Kin (Pixel 8 is offline, will queue)");
  });
  test("several devices: a submenu, offline and waiting shown", () => {
    const m = buildMenuModel(status([dev("a", "Pixel 8", true), dev("b", "Galaxy A54", false, 2), dev("c", "Moto", true, 3)]));
    expect(m[0]).toEqual({ id: MENU_ROOT, title: "Send to Kin" });
    expect(m.slice(1)).toEqual([
      { id: "kin:dev:a", title: "Pixel 8", parentId: MENU_ROOT },
      { id: "kin:dev:b", title: "Galaxy A54 (offline, will queue)", parentId: MENU_ROOT },
      { id: "kin:dev:c", title: "Moto (3 waiting)", parentId: MENU_ROOT },
    ]);
  });
  test("an & in a device name is escaped", () => {
    expect(escapeMenuTitle("Tom & Jerry")).toBe("Tom && Jerry");
    const m = buildMenuModel(status([dev("a", "A&B", true), dev("b", "C", true)]));
    expect(m[1]!.title).toBe("A&&B");
  });
});

describe("menu ids", () => {
  test("device id comes out of the item id", () => {
    expect(menuDeviceId("kin:dev:abc_DEF-1")).toBe("abc_DEF-1");
    expect(menuDeviceId(MENU_SINGLE)).toBeUndefined();
    expect(menuDeviceId(7)).toBeUndefined();
  });
  test("only our items are ours", () => {
    expect(isKinMenuItem(MENU_SINGLE)).toBe(true);
    expect(isKinMenuItem("kin:dev:x")).toBe(true);
    expect(isKinMenuItem(MENU_ROOT)).toBe(false);
    expect(isKinMenuItem("other")).toBe(false);
  });
});

describe("clickTarget", () => {
  test("an image wins over its link", () => {
    expect(clickTarget({ srcUrl: "https://x/a.png", mediaType: "image", linkUrl: "https://x/" })).toEqual({ kind: "image", srcUrl: "https://x/a.png" });
  });
  test("a video's srcUrl is not an image", () => {
    expect(clickTarget({ srcUrl: "https://x/a.mp4", mediaType: "video", pageUrl: "https://x/" })).toEqual({ kind: "page" });
  });
  test("a link wins over a selection", () => {
    expect(clickTarget({ linkUrl: "https://x/", linkText: "go", selectionText: "go" })).toEqual({ kind: "link", linkUrl: "https://x/", linkText: "go" });
  });
  test("a selection wins over the page", () => {
    expect(clickTarget({ selectionText: "hello", pageUrl: "https://x/" })).toEqual({ kind: "text", text: "hello" });
  });
  test("otherwise the page", () => {
    expect(clickTarget({ pageUrl: "https://x/" })).toEqual({ kind: "page" });
    expect(clickTarget({ selectionText: "" })).toEqual({ kind: "page" });
  });
});

describe("pickDevice", () => {
  const s = status([dev("a", "A", true), dev("b", "B", true)], "b");
  test("the saved choice when it still exists", () => {
    expect(pickDevice(s, "a")).toBe("a");
  });
  test("else Kin's default, else the first", () => {
    expect(pickDevice(s, "gone")).toBe("b");
    expect(pickDevice(status([dev("a", "A", true)]), undefined)).toBe("a");
  });
  test("nothing known means let Kin choose", () => {
    expect(pickDevice(null, "a")).toBeUndefined();
    expect(pickDevice(status([]), "a")).toBeUndefined();
  });
});
