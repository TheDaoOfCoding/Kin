// The `downloadAlert[]` cases of protocol/alerts.json, run against the pure rules.
// The same cases run in a real headless browser in tests/smoke/run.ts.
import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import {
  ALERT_TTL_SEC,
  BURST_MS,
  baseName,
  composeAlert,
  createDownloadAlerter,
  createRegistrar,
  DEFAULT_SETTINGS,
  factsFromItem,
  judge,
  sanitizeSettings,
  type ChangeDelta,
  type DownloadFacts,
  type DownloadSettings,
} from "../../src/shared/downloads";
import { buildSendAlert } from "../../src/shared/messages";
import { sanitizeHistory } from "../../src/shared/history";

const facts = (over: Partial<DownloadFacts> = {}): DownloadFacts => ({
  state: "complete",
  incognito: false,
  filename: "C:\\Users\\someone\\Downloads\\report.pdf",
  startMs: 1_000_000,
  endMs: 1_040_000,
  ...over,
});
const tookSeconds = (s: number): Partial<DownloadFacts> => ({ startMs: 1_000_000, endMs: 1_000_000 + s * 1000 });

/** The alerter with a fake clock: timers run when the test says so. */
function rig(settings: DownloadSettings, items: Record<number, DownloadFacts | null>) {
  const delivered: { title: string; text: string }[] = [];
  const timers: { fn: () => void; ms: number }[] = [];
  let current = settings;
  let clock = 1_000_000;
  let disabledCalls = 0;
  const alerter = createDownloadAlerter({
    now: () => clock,
    disabled: () => void disabledCalls++,
    settings: async () => current,
    lookup: async (id) => items[id] ?? null,
    schedule: (fn, ms) => {
      timers.push({ fn, ms });
      return timers.length;
    },
    deliver: async (a) => {
      delivered.push(a);
    },
  });
  const settle = () => new Promise((r) => setTimeout(r, 0));
  return {
    delivered,
    timers,
    setSettings: (s: DownloadSettings) => (current = s),
    /** The time the next event arrives; downloads in `facts()` start at 1_000_000. */
    setClock: (ms: number) => (clock = ms),
    disabledCalls: () => disabledCalls,
    raw: (delta: ChangeDelta) => alerter.onChanged(delta),
    async change(id: number, state: string) {
      await alerter.onChanged({ id, state: { current: state } });
    },
    async advance() {
      for (const t of timers.splice(0)) t.fn();
      await settle();
    },
  };
}
const on: DownloadSettings = { enabled: true, longOnly: true };

describe("downloadAlert[]: which downloads alert", () => {
  test("a completed 40 s download gives one alert with the base name only", async () => {
    const r = rig(on, { 1: facts() });
    await r.change(1, "complete");
    expect(r.delivered).toEqual([]); // held for the burst window
    expect(r.timers.map((t) => t.ms)).toEqual([BURST_MS]);
    await r.advance();
    expect(r.delivered).toEqual([{ title: "Download finished", text: "report.pdf" }]);
  });

  test("a 10 s download gives none (the switch 'only over 30 seconds' is on)", async () => {
    const r = rig(on, { 1: facts(tookSeconds(10)) });
    await r.change(1, "complete");
    expect(r.timers).toEqual([]);
    await r.advance();
    expect(r.delivered).toEqual([]);
  });

  test("with 'only over 30 seconds' off, a 10 s download alerts", async () => {
    const r = rig({ enabled: true, longOnly: false }, { 1: facts(tookSeconds(10)) });
    await r.change(1, "complete");
    await r.advance();
    expect(r.delivered).toEqual([{ title: "Download finished", text: "report.pdf" }]);
  });

  test("exactly 30 s counts, 29.9 s does not", () => {
    expect(judge(facts(tookSeconds(30)), true).alert).toBe(true);
    expect(judge(facts({ startMs: 0, endMs: 29_999 }), true).alert).toBe(false);
  });

  test("an incognito download gives none, even with the 30 s rule off", async () => {
    const r = rig({ enabled: true, longOnly: false }, { 1: facts({ incognito: true }), 2: facts({ incognito: true, state: "interrupted" }) });
    await r.change(1, "complete");
    await r.change(2, "interrupted");
    await r.advance();
    expect(r.delivered).toEqual([]);
  });

  test("an interrupted download gives 'Download failed'", async () => {
    const r = rig(on, { 1: facts({ state: "interrupted", error: "NETWORK_FAILED" }) });
    await r.change(1, "interrupted");
    await r.advance();
    expect(r.delivered).toEqual([{ title: "Download failed", text: "report.pdf" }]);
  });

  test("a download the user cancelled gives none", async () => {
    const r = rig({ enabled: true, longOnly: false }, { 1: facts({ state: "interrupted", error: "USER_CANCELED" }) });
    await r.change(1, "interrupted");
    await r.advance();
    expect(r.delivered).toEqual([]);
  });

  test("other state changes (in_progress, a filename change) give none", async () => {
    const r = rig(on, { 1: facts() });
    await r.change(1, "in_progress");
    await r.raw({ id: 1 }); // e.g. only the filename or the progress changed
    expect(r.timers).toEqual([]);
  });

  test("nothing happens while the switch is off", async () => {
    const r = rig({ enabled: false, longOnly: false }, { 1: facts() });
    await r.change(1, "complete");
    await r.advance();
    expect(r.delivered).toEqual([]);
    expect(r.timers).toEqual([]);
  });

  test("turned off while the burst fills: nothing leaves", async () => {
    const r = rig(on, { 1: facts() });
    await r.change(1, "complete");
    r.setSettings({ enabled: false, longOnly: true });
    await r.advance();
    expect(r.delivered).toEqual([]);
  });

  test("a download the browser forgot is skipped quietly", async () => {
    const r = rig(on, {});
    await r.change(9, "complete");
    await r.advance();
    expect(r.delivered).toEqual([]);
  });

  test("the same download reported twice in the same state alerts once", async () => {
    const r = rig(on, { 1: facts() });
    await r.change(1, "complete");
    await r.change(1, "complete");
    await r.advance();
    expect(r.delivered).toHaveLength(1);
    await r.change(1, "complete");
    await r.advance();
    expect(r.delivered).toHaveLength(1);
  });

  test("the de-duplication is per download and state: interrupted, then resumed to completion, is two alerts", async () => {
    const items: Record<number, DownloadFacts> = { 1: facts({ state: "interrupted", error: "NETWORK_FAILED" }) };
    const r = rig(on, items);
    r.setClock(1_040_000);
    await r.change(1, "interrupted");
    await r.change(1, "interrupted"); // the same event again: still one
    await r.advance();
    expect(r.delivered).toEqual([{ title: "Download failed", text: "report.pdf" }]);
    items[1] = facts({ state: "complete" });
    r.setClock(1_100_000);
    await r.change(1, "complete");
    await r.advance();
    expect(r.delivered).toEqual([
      { title: "Download failed", text: "report.pdf" },
      { title: "Download finished", text: "report.pdf" },
    ]);
  });

  test("while the switch is off nothing is remembered and the listener is told it can go", async () => {
    const r = rig({ enabled: false, longOnly: false }, { 1: facts() });
    await r.change(1, "complete");
    expect(r.disabledCalls()).toBe(1);
    r.setSettings({ enabled: true, longOnly: false });
    await r.change(1, "complete"); // not "seen" for having arrived while off
    await r.advance();
    expect(r.delivered).toEqual([{ title: "Download finished", text: "report.pdf" }]);
  });
});

// The browser doesn't always fill in `endTime`: Firefox never does, Chromium only for a completed download. The 30 s
// rule then measures to the time of the onChanged event, with the box ON (the default).
describe("downloadAlert[]: no end time, 'only over 30 seconds' on", () => {
  const noEnd = (over: Partial<DownloadFacts> = {}): DownloadFacts => {
    const { endMs: _dropped, ...rest } = facts(over);
    return rest;
  };
  const START = 1_000_000;

  test("Firefox-shaped completed download (no endTime): over 30 s alerts, under 30 s does not", async () => {
    const r = rig(on, { 1: noEnd(), 2: noEnd({ filename: "two.zip" }) });
    r.setClock(START + 45_000);
    await r.change(1, "complete");
    r.setClock(START + 5_000);
    await r.change(2, "complete");
    await r.advance();
    expect(r.delivered).toEqual([{ title: "Download finished", text: "report.pdf" }]);
  });

  test("Chromium-shaped interrupted download (no endTime): failed after 40 s gives 'Download failed'", async () => {
    const r = rig(on, { 1: noEnd({ state: "interrupted", error: "NETWORK_FAILED" }) });
    r.setClock(START + 40_000);
    await r.change(1, "interrupted");
    await r.advance();
    expect(r.delivered).toEqual([{ title: "Download failed", text: "report.pdf" }]);
  });

  test("an interrupted download that failed after 3 s gives none; a USER_CANCELED one never does", async () => {
    const r = rig(on, { 1: noEnd({ state: "interrupted", error: "NETWORK_FAILED" }), 2: noEnd({ state: "interrupted", error: "USER_CANCELED" }) });
    r.setClock(START + 3_000);
    await r.change(1, "interrupted");
    r.setClock(START + 90_000);
    await r.change(2, "interrupted");
    await r.advance();
    expect(r.delivered).toEqual([]);
  });

  test("a Chromium completed download with an endTime still uses it, not the event time", () => {
    expect(judge(facts({ startMs: START, endMs: START + 10_000 }), true, START + 99_000).alert).toBe(false);
    expect(judge(facts({ startMs: START, endMs: START + 31_000 }), true, START + 31_100)).toMatchObject({ alert: true, kind: "finished" });
  });

  test("without a start time the length is unknown and counts as short", () => {
    const { startMs: _s, endMs: _e, ...none } = facts();
    expect(judge(none, true, START + 99_000).alert).toBe(false);
    expect(judge(none, false, START + 99_000).alert).toBe(true);
  });

  test("factsFromItem leaves endMs out when the browser has no endTime", () => {
    const f = factsFromItem({ state: "interrupted", error: "NETWORK_FAILED", incognito: false, filename: "x.bin", startTime: "2026-10-17T10:00:00.000Z" })!;
    expect(f.endMs).toBeUndefined();
    expect(f.startMs).toBe(Date.parse("2026-10-17T10:00:00.000Z"));
  });
});

describe("downloadAlert[]: bursts", () => {
  test("3 downloads in 3 s give one alert, '3 downloads finished'", async () => {
    const r = rig(on, { 1: facts(), 2: facts({ filename: "/home/x/two.zip" }), 3: facts({ filename: "three.iso" }) });
    await r.change(1, "complete");
    await r.change(2, "complete");
    await r.change(3, "complete");
    expect(r.timers).toHaveLength(1); // one window, started by the first
    await r.advance();
    expect(r.delivered).toEqual([{ title: "Downloads finished", text: "3 downloads finished" }]);
  });

  test("a burst of failures, and a mixed one", () => {
    expect(composeAlert({ finished: [], failed: ["a", "b"] })).toEqual({ title: "Downloads failed", text: "2 downloads failed" });
    expect(composeAlert({ finished: ["a", "b"], failed: ["c"] })).toEqual({ title: "Downloads finished", text: "2 finished, 1 failed" });
    expect(composeAlert({ finished: [], failed: [] })).toBeNull();
  });

  test("a download after the window opens a new one", async () => {
    const r = rig(on, { 1: facts(), 2: facts({ filename: "later.txt" }) });
    await r.change(1, "complete");
    await r.advance();
    await r.change(2, "complete");
    await r.advance();
    expect(r.delivered.map((d) => d.text)).toEqual(["report.pdf", "later.txt"]);
  });
});

describe("what leaves the browser", () => {
  test("base name: folders of both kinds are gone", () => {
    expect(baseName("C:\\Users\\someone\\Downloads\\a b.pdf")).toBe("a b.pdf");
    expect(baseName("/home/someone/Downloads/a.pdf")).toBe("a.pdf");
    expect(baseName("a.pdf")).toBe("a.pdf");
    expect(baseName("C:\\x\\")).toBe("");
  });

  test("bidi controls become _ and control characters and runs of spaces collapse", () => {
    expect(baseName("D:\\x\\evil\u202Efdp.exe")).toBe("evil_fdp.exe");
    expect(baseName("x\u200Fy\u2066z\u2069.txt")).toBe("x_y_z_.txt");
    expect(baseName("a\u0000\nb   c.txt")).toBe("a b c.txt");
  });

  test("a name over 120 characters is cut at 120 code points", () => {
    const long = "é".repeat(130) + ".txt";
    expect(Array.from(baseName(`C:\\x\\${long}`))).toHaveLength(120);
    const emoji = "😀".repeat(130);
    expect(Array.from(baseName(emoji))).toHaveLength(120);
  });

  test("a download with no name yet still alerts, as 'A file'", () => {
    expect(judge(facts({ state: "interrupted", filename: "" }), false)).toEqual({ alert: true, kind: "failed", name: "A file" });
  });

  test("the request carries the base name and no address, path or referrer", async () => {
    // A browser item has far more than the extension keeps: the facts hold none of it.
    const item = {
      id: 7,
      url: "https://secret.example/token?x=1",
      finalUrl: "https://cdn.secret.example/f?sig=2",
      referrer: "https://page.secret.example/",
      filename: "C:\\Users\\someone\\Downloads\\report.pdf",
      state: "complete",
      incognito: false,
      startTime: "2026-10-17T10:00:00.000Z",
      endTime: "2026-10-17T10:00:40.000Z",
    };
    const f = factsFromItem(item)!;
    expect(Object.keys(f).sort()).toEqual(["endMs", "filename", "incognito", "startMs", "state"]);
    const r = rig(on, { 7: f });
    await r.change(7, "complete");
    await r.advance();
    const built = buildSendAlert(r.delivered[0]!.title, r.delivered[0]!.text, { source: "downloads", ttlSec: ALERT_TTL_SEC, deviceId: "dev" });
    expect(built.ok).toBe(true);
    if (!built.ok) return;
    expect(built.request).toMatchObject({ v: 1, type: "send", kind: "alert", title: "Download finished", text: "report.pdf", source: "downloads", auto: true, ttlSec: 1800, deviceId: "dev" });
    const wire = JSON.stringify(built.request);
    for (const secret of ["secret.example", "token", "sig=2", "Users", "someone", "Downloads", "\\\\"]) expect(wire).not.toContain(secret);
  });

  test("the code that reads downloads never touches a URL or a referrer", () => {
    const strip = (src: string) => src.replace(/\/\*[\s\S]*?\*\//g, "").replace(/^\s*\/\/.*$/gm, "").replace(/\s\/\/ .*$/gm, "");
    for (const file of ["src/shared/downloads.ts", "src/downloads.ts"]) {
      const code = strip(readFileSync(join(import.meta.dir, "..", "..", file), "utf8"));
      expect(code).not.toMatch(/finalUrl|referrer|\.url\b|\bpath\b/i);
    }
  });
});

describe("the request", () => {
  test("shape, the 30 minute expiry, a title cut to 60 and the text to 300", () => {
    const b = buildSendAlert("t".repeat(80), "x".repeat(400), { source: "downloads", ttlSec: ALERT_TTL_SEC });
    expect(b.ok).toBe(true);
    if (!b.ok) return;
    const req = b.request as unknown as Record<string, unknown>;
    expect(Array.from(req.title as string)).toHaveLength(60);
    expect(Array.from(req.text as string)).toHaveLength(300);
    expect(req).toMatchObject({ kind: "alert", source: "downloads", auto: true, ttlSec: 1800 });
    expect(req.deviceId).toBeUndefined();
    expect(ALERT_TTL_SEC).toBe(1800);
  });
  test("an empty text is refused before it reaches the host", () => {
    expect(buildSendAlert("t", "   ", { source: "downloads", ttlSec: 1800 })).toMatchObject({ ok: false, code: "BAD_REQUEST" });
  });
  test("the history keeps a 'download' entry", () => {
    expect(sanitizeHistory([{ at: 1, kind: "download", label: "Download alert", outcome: "sent" }])).toEqual([{ at: 1, kind: "download", label: "Download alert", outcome: "sent" }]);
  });
});

describe("the listener is registered only when the browser has the API", () => {
  const fakeApi = () => {
    const added: unknown[] = [];
    const removed: unknown[] = [];
    return { added, removed, api: { onChanged: { addListener: (l: unknown) => void added.push(l), removeListener: (l: unknown) => void removed.push(l) } } };
  };
  const noop = (_d: ChangeDelta) => {};

  test("no permission: `downloads` is undefined, nothing is registered", () => {
    const reg = createRegistrar(noop);
    expect(reg.register(undefined)).toBe(false);
    expect(reg.register({})).toBe(false);
    expect(reg.registered).toBe(false);
  });
  test("with the permission it registers once, however often it is asked", () => {
    const { added, api } = fakeApi();
    const reg = createRegistrar(noop);
    expect(reg.register(api)).toBe(true);
    expect(reg.register(api)).toBe(false);
    expect(added).toEqual([noop]);
    expect(reg.registered).toBe(true);
  });
  test("when the permission goes, the listener is released and can be registered again", () => {
    const { added, removed, api } = fakeApi();
    const reg = createRegistrar(noop);
    reg.register(api);
    reg.release();
    expect(removed).toEqual([noop]);
    expect(reg.registered).toBe(false);
    expect(reg.register(api)).toBe(true);
    expect(added).toHaveLength(2);
  });
  test("a vanished API cannot break the release", () => {
    const reg = createRegistrar(noop);
    reg.register({
      onChanged: {
        addListener: () => {},
        removeListener: () => {
          throw new Error("context invalidated");
        },
      },
    });
    expect(() => reg.release()).not.toThrow();
  });
});

// The shared vectors: every `downloadAlert[]` case of protocol/alerts.json is run against this extension's own code (the alerter, the filters and the composer), so what the spec
// says and what the extension sends can't drift apart again (2B.5 review round 3: the burst titles did). The cases are written from the spec; this file only reads them.
describe("protocol/alerts.json downloadAlert[] against the extension's code", () => {
  type Pieces = [string, number][];
  interface VectorEvent {
    t: number;
    id: number;
    state: string;
    incognito: boolean;
    durationMs: number;
    path: string;
    pathParts?: Pieces;
    error?: string;
  }
  interface Vector {
    name: string;
    setting: { enabled: boolean; onlyLong: boolean };
    events: VectorEvent[];
    expect: { t: number; title: string; text?: string; textParts?: Pieces }[];
  }
  const pieces = (p: Pieces) => p.map(([s, n]) => s.repeat(n)).join("");
  const file = join(import.meta.dir, "..", "..", "..", "protocol", "alerts.json");
  const vectors = (JSON.parse(readFileSync(file, "utf8")) as { downloadAlert: Vector[] }).downloadAlert;

  /** The alerter on a virtual clock (milliseconds from the first event's 0): timers fire at their time, before an event at that same time. */
  async function run(v: Vector) {
    const START = 1_000_000;
    let now = 0;
    const lookups = new Map<number, DownloadFacts>();
    const timers: { at: number; fn: () => void }[] = [];
    const sent: { t: number; title: string; text: string }[] = [];
    const alerter = createDownloadAlerter({
      now: () => START + now,
      settings: async () => ({ enabled: v.setting.enabled, longOnly: v.setting.onlyLong }),
      lookup: async (id) => lookups.get(id) ?? null,
      schedule: (fn, ms) => timers.push({ at: now + ms, fn }),
      deliver: async (a) => void sent.push({ t: now, ...a }),
    });
    const settle = () => new Promise((r) => setTimeout(r, 0));
    const fireUntil = async (limit: number) => {
      for (;;) {
        const due = timers.filter((t) => t.at <= limit).sort((a, b) => a.at - b.at)[0];
        if (!due) return;
        timers.splice(timers.indexOf(due), 1);
        now = due.at;
        due.fn();
        await settle();
      }
    };
    for (const e of v.events) {
      await fireUntil(e.t);
      now = e.t;
      lookups.set(e.id, {
        state: e.state,
        ...(e.error !== undefined ? { error: e.error } : {}),
        incognito: e.incognito,
        filename: e.path + (e.pathParts ? pieces(e.pathParts) : ""),
        startMs: START,
        endMs: START + e.durationMs,
      });
      await alerter.onChanged({ id: e.id, state: { current: e.state } });
    }
    await fireUntil(Number.POSITIVE_INFINITY);
    return sent;
  }

  test("the file has the cases this test is about", () => {
    const names = vectors.map((v) => v.name);
    expect(vectors.length).toBeGreaterThanOrEqual(20);
    for (const n of ["three-in-3-seconds-are-one-alert", "two-interrupted-in-a-burst", "a-mixed-burst", "a-canceled-download-is-skipped", "interrupted-then-resumed-is-two-alerts", "a-name-with-bidi-controls-gets-underscores"]) {
      expect(names).toContain(n);
    }
  });

  for (const v of vectors) {
    test(v.name, async () => {
      const expected = v.expect.map((x) => ({ t: x.t, title: x.title, text: x.textParts ? pieces(x.textParts) : (x.text as string) }));
      expect(await run(v)).toEqual(expected);
    });
  }
});

describe("settings", () => {
  test("off by default, 30 s rule on", () => {
    expect(DEFAULT_SETTINGS).toEqual({ enabled: false, longOnly: true });
    expect(sanitizeSettings(undefined)).toEqual({ enabled: false, longOnly: true });
    expect(sanitizeSettings("junk")).toEqual({ enabled: false, longOnly: true });
  });
  test("only a real true turns it on", () => {
    expect(sanitizeSettings({ enabled: "yes" }).enabled).toBe(false);
    expect(sanitizeSettings({ enabled: true, longOnly: false })).toEqual({ enabled: true, longOnly: false });
  });
});
