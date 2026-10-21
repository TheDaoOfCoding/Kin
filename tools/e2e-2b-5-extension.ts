// Scenario S6 of tools/e2e-2b-5.ps1: the browser extension's "Download finished" alerts (milestone 2B.5, design 5.4), end to end:
// headless Chromium (throwaway profile, the Playwright Chromium) with the extension's TEST build (`downloads` is a normal permission there: a headless run can't click the prompt)
// -> the REAL dev native host -> the REAL test Kin (a dev instance beside the installed one, paired with the emulator by the PowerShell script) -> the EMULATOR phone.
// Run only through the script: it passes the paths in E2E_* variables and holds the scripts' lock.
//
// Safety, by construction (the rules of tools/e2e-2b-1-extension.ts):
//  - headless only, a throwaway --user-data-dir and download folder under the work folder, never the user's browser, profile or Downloads folder
//  - the only registry key written is HKCU\Software\Chromium\NativeMessagingHosts\app.kinrelay.dev (the DEV host name) pointing at the dev host exe; refused if it already exists,
//    removed in `finally`, on a signal and on exit. The release keys (app.kinrelay.desktop) are never touched.
//  - phase A: the dev host is pointed at the test Kin's own pipe (KIN_HOST_DEV_PIPE) and KIN_HOST_DEV_KIN_EXE is NOT set, so the host can't start any Kin.
//  - phase B (the "does a download alert start Kin?" check): a second browser whose host is pointed at a pipe that nobody serves and whose KIN_HOST_DEV_KIN_EXE is the harmless
//    dev stand-in mock-kin.exe (it only serves a mock pipe and writes the pid of each start into starts.log). Nothing there can reach the real Kin or the phone. The mock processes the host
//    started are stopped by the pids it logged.
//  - nothing is killed by name; the only thing stopped on the PC is the test Kin, by the script, through `Kin.exe --data-dir <ours> ...`
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium, type BrowserContext, type Worker } from "../extension/node_modules/playwright-core";

const here = resolve(dirname(fileURLToPath(import.meta.url)));
const repo = resolve(here, "..");
const env = (n: string): string => {
  const v = process.env[n];
  if (!v) throw new Error(`${n} is not set (run tools/e2e-2b-5.ps1)`);
  return v;
};
const DATA = env("E2E_DATA");
const SUFFIX = env("E2E_SUFFIX");
const KIN_EXE = env("E2E_KIN");
const HOST_EXE = env("E2E_HOST_EXE");
const MOCK_EXE = env("E2E_MOCK_EXE");
const ADB = env("E2E_ADB");
const SERIAL = env("E2E_SERIAL");
const PKG = env("E2E_PKG");
const OUT = env("E2E_OUT");
const WORK = env("E2E_WORK");
const SHOTS = process.env.E2E_SHOTS ?? "";
const extDir = join(repo, "extension", "dist", "chromium-test");
const EXPECTED_ID = "cbioefpddjpaionhljhffgbedncfchmn";
const HOST = "app.kinrelay.dev";
const REG_KEY = `HKCU\\Software\\Chromium\\NativeMessagingHosts\\${HOST}`;

// ---- small test harness ------------------------------------------------------------------------
const results: { name: string; ok: boolean; detail?: string }[] = [];
async function step(name: string, fn: () => Promise<void>): Promise<void> {
  try {
    await fn();
    results.push({ name, ok: true });
    console.log(`  ok    ${name}`);
  } catch (e) {
    const detail = e instanceof Error ? e.message : String(e);
    results.push({ name, ok: false, detail });
    console.log(`  FAIL  ${name}\n        ${detail.split("\n").join("\n        ")}`);
  }
}
function check(cond: unknown, msg: string): asserts cond {
  if (!cond) throw new Error(msg);
}
function eq<T>(actual: T, expected: T, what: string): void {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) throw new Error(`${what}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
}
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
async function until<T>(what: string, fn: () => T | Promise<T>, ms = 20_000): Promise<NonNullable<T>> {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    const v = await fn();
    if (v) return v as NonNullable<T>;
    await sleep(250);
  }
  throw new Error(`timed out waiting for ${what}`);
}

// ---- the phone, through adb --------------------------------------------------------------------
const adb = (...a: string[]) => spawnSync(ADB, ["-s", SERIAL, ...a], { encoding: "utf8", maxBuffer: 256 * 1024 * 1024, windowsHide: true, timeout: 60_000 });
const unhex = (h: string) => (h ? Buffer.from(h, "hex").toString("utf8") : "");
type Row = { id: string; dir: string; status: string; kind: string; title: string; body: string };
function phoneRows(): Row[] {
  const out = adb("shell", `run-as ${PKG} sqlite3 databases/kin.db 'select id,direction,status,kind,hex(title),hex(body) from texts order by createdAtMillis'`).stdout ?? "";
  return out
    .split(/\r?\n/)
    .map((l) => l.split("|"))
    .filter((f) => f.length >= 6)
    .map((f) => ({ id: f[0], dir: f[1], status: f[2], kind: f[3], title: unhex(f[4]), body: unhex(f[5]) }));
}
const alertRows = (): Row[] => phoneRows().filter((r) => r.kind === "alert");
function alertNotifText(): string {
  const raw = adb("shell", "dumpsys notification --noredact").stdout ?? "";
  return raw.split(/^\s*NotificationRecord\(/m).filter((b) => b.includes(`pkg=${PKG} `) && b.includes("channel=kin_alerts")).join("\n");
}
function phoneShot(name: string): void {
  if (!SHOTS) return;
  mkdirSync(SHOTS, { recursive: true });
  adb("shell", "cmd statusbar expand-notifications");
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 1500);
  const r = spawnSync(ADB, ["-s", SERIAL, "exec-out", "screencap", "-p"], { maxBuffer: 64 * 1024 * 1024, windowsHide: true });
  if (r.stdout && r.stdout.length > 1000) writeFileSync(join(SHOTS, `${name}.png`), r.stdout);
  adb("shell", "cmd statusbar collapse");
}
const kinCli = (...a: string[]) => spawnSync(KIN_EXE, ["--data-dir", DATA, ...a], { encoding: "utf8", windowsHide: true, timeout: 30_000 });

// ---- registry (HKCU, dev host name only) ----------------------------------------------------------
const regQuery = () => spawnSync("reg", ["query", REG_KEY, "/ve"], { encoding: "utf8" }).status === 0;
let registered = false;
function unregister(): void {
  if (!registered) return;
  spawnSync("reg", ["delete", REG_KEY, "/f"], { encoding: "utf8" });
  registered = false;
}
process.on("exit", unregister);
for (const sig of ["SIGINT", "SIGTERM"] as const)
  process.on(sig, () => {
    unregister();
    process.exit(130);
  });

function findChromium(): string {
  if (process.env.KIN_CHROMIUM && existsSync(process.env.KIN_CHROMIUM)) return process.env.KIN_CHROMIUM;
  const base = join(process.env.LOCALAPPDATA ?? "", "ms-playwright");
  const dirs = existsSync(base) ? readdirSync(base).filter((d) => /^chromium-\d+$/.test(d)).sort((a, b) => Number(b.split("-")[1]) - Number(a.split("-")[1])) : [];
  for (const d of dirs) {
    const exe = join(base, d, "chrome-win64", "chrome.exe");
    if (existsSync(exe)) return exe;
  }
  throw new Error("No Playwright Chromium found (set KIN_CHROMIUM).");
}

// ---- test site (loopback only): small downloads, a slow one (31.5 s) and a broken one ----------------------
function startSite() {
  return Bun.serve({
    hostname: "127.0.0.1",
    port: 0,
    idleTimeout: 0,
    fetch(req) {
      const url = new URL(req.url);
      if (url.pathname === "/") return new Response(`<!doctype html><title>Kin test page</title><h1>Hello</h1>`, { headers: { "content-type": "text/html" } });
      if (url.pathname === "/dl/slow.bin") {
        let timer: ReturnType<typeof setTimeout>;
        return new Response(
          new ReadableStream({
            start(c) {
              c.enqueue(new Uint8Array(1000).fill(0x61));
              timer = setTimeout(() => {
                c.enqueue(new Uint8Array(1000).fill(0x62));
                c.close();
              }, 31_500);
            },
            cancel() {
              clearTimeout(timer);
            },
          }),
          { headers: { "content-type": "application/octet-stream", "content-disposition": 'attachment; filename="ZQDownloadName-slow.bin"' } },
        );
      }
      if (url.pathname === "/dl/broken.bin") {
        return new Response(
          new ReadableStream({
            start(c) {
              c.enqueue(new Uint8Array(1000).fill(0x64));
              setTimeout(() => c.error(new Error("drop")), 100);
            },
          }),
          { headers: { "content-type": "application/octet-stream", "content-length": "100000", "content-disposition": 'attachment; filename="ZQDownloadName-broken.bin"' } },
        );
      }
      if (url.pathname.startsWith("/dl/")) {
        const name = decodeURIComponent(url.pathname.slice(4));
        return new Response("hello from the test server\n", { headers: { "content-type": "application/octet-stream", "content-disposition": `attachment; filename="${name}"` } });
      }
      return new Response("not found", { status: 404 });
    },
  });
}

// ---- main ----------------------------------------------------------------------------------------------
const statusOf = (): any => {
  try {
    return JSON.parse(readFileSync(join(DATA, "status.json"), "utf8"));
  } catch {
    return {};
  }
};
const names: string[] = [];

async function main(): Promise<number> {
  for (const f of [join(extDir, "manifest.json"), HOST_EXE, KIN_EXE, MOCK_EXE]) check(existsSync(f), `missing ${f}`);
  if (regQuery()) throw new Error(`${REG_KEY} already exists. Not touching it: remove it by hand if it is a leftover.`);
  const sid = spawnSync("powershell", ["-NoProfile", "-Command", "[Security.Principal.WindowsIdentity]::GetCurrent().User.Value"], { encoding: "utf8" }).stdout.trim();
  check(/^S-1-5-/.test(sid), "could not read the user SID");
  const PIPE = `\\\\.\\pipe\\${HOST}.${sid}.${SUFFIX}`;
  const PIPE_NOBODY = `\\\\.\\pipe\\${HOST}.${sid}.${SUFFIX}nobody`;
  const browser = findChromium();
  console.log(`browser: ${browser}\nKin: ${KIN_EXE}\nphone: ${SERIAL}`);
  check(statusOf().state === "phone_connected", `the test Kin is not connected to the phone (state ${statusOf().state})`);

  const work = join(WORK, "s6");
  mkdirSync(work, { recursive: true });
  const manifestPath = join(work, `${HOST}.json`);
  writeFileSync(manifestPath, JSON.stringify({ name: HOST, description: "Kin e2e 2B.5", path: HOST_EXE, type: "stdio", allowed_origins: [`chrome-extension://${EXPECTED_ID}/`] }, null, 2));
  const site = startSite();
  const siteUrl = `http://127.0.0.1:${site.port}`;
  const mockPids: number[] = [];
  let context: BrowserContext | undefined;
  let context2: BrowserContext | undefined;

  const launch = async (profileName: string, extraEnv: Record<string, string>) => {
    const profile = join(work, profileName);
    mkdirSync(profile, { recursive: true });
    const ctx = await chromium.launchPersistentContext(profile, {
      executablePath: browser,
      headless: true,
      downloadsPath: join(work, `downloads-${profileName}`),
      env: { ...(process.env as Record<string, string>), ...extraEnv },
      args: [`--disable-extensions-except=${extDir}`, `--load-extension=${extDir}`, "--mute-audio", "--no-first-run", "--no-default-browser-check", "--disable-sync", "--disable-background-networking"],
      viewport: { width: 900, height: 700 },
    });
    const getSw = async (): Promise<Worker> => {
      const find = () => ctx.serviceWorkers().find((w) => w.url().startsWith("chrome-extension://"));
      return find() ?? (await ctx.waitForEvent("serviceworker", { timeout: 15_000 }));
    };
    const kin = async (name: string, ...args: unknown[]): Promise<any> => {
      for (let attempt = 0; attempt < 3; attempt++) {
        try {
          const w = await getSw();
          return await w.evaluate(([n, a]) => (globalThis as any).__kin[n](...(a as unknown[])), [name, args] as [string, unknown[]]);
        } catch (e) {
          if (attempt === 2) throw e;
          const p = await ctx.newPage();
          await p.goto(`chrome-extension://${EXPECTED_ID}/popup/popup.html`).catch(() => {});
          await p.close().catch(() => {});
        }
      }
    };
    /** Starts a download from the worker and waits until the browser has finished with it. */
    const download = async (path: string) => {
      const w = await getSw();
      return (await w.evaluate(
        async (u) => {
          const id = await chrome.downloads.download({ url: u as string, conflictAction: "uniquify" });
          for (let i = 0; i < 600; i++) {
            const [item] = await chrome.downloads.search({ id });
            if (item && item.state !== "in_progress") return { id, state: item.state, error: item.error, filename: item.filename, incognito: item.incognito };
            await new Promise((r) => setTimeout(r, 100));
          }
          return { id, state: "in_progress", filename: "", incognito: false };
        },
        siteUrl + path,
      )) as { id: number; state: string; error?: string; filename: string; incognito: boolean };
    };
    return { ctx, getSw, kin, download };
  };
  const baseOf = (p: string) => p.split(/[\\/]/).pop()!;
  const setAlerts = (kin: (n: string, ...a: unknown[]) => Promise<any>, patch: { enabled?: boolean; longOnly?: boolean }) => kin("handlePopup", { kin: "downloadAlerts", ...patch });

  try {
    const add = spawnSync("reg", ["add", REG_KEY, "/ve", "/t", "REG_SZ", "/d", manifestPath, "/f"], { encoding: "utf8" });
    check(add.status === 0, `reg add failed: ${add.stderr}`);
    registered = true;

    // ================================================================ phase A: the real dev host, the real test Kin, the emulator
    const A = await launch("profile-a", { KIN_HOST_DEV_PIPE: PIPE });
    context = A.ctx;
    await A.ctx.addCookies([{ name: "auth", value: "1", url: siteUrl }]);
    const page = await A.ctx.newPage();
    await page.goto(siteUrl + "/");
    const popupUrl = `chrome-extension://${EXPECTED_ID}/popup/popup.html`;

    await step("A: the extension loads; the real host and the real Kin answer status", async () => {
      eq(new URL((await A.getSw()).url()).host, EXPECTED_ID, "extension id");
      const view = await A.kin("refreshStatus");
      eq(view.ok, true, `view.ok (${JSON.stringify(view)})`);
      eq(view.status.devices.length >= 1, true, "a device");
    });

    await step("A: download alerts are off by default: a finished download sends nothing", async () => {
      const before = alertRows().length;
      const d = await A.download("/dl/zq-default-off.txt");
      eq(d.state, "complete", "download state");
      await sleep(6000);
      eq(alertRows().length, before, "alert rows on the phone");
    });

    let fastName = "";
    await step("A: switch on (30 s rule off): a quick download reaches the phone as 'Download finished' with the file's base name only", async () => {
      const r = await setAlerts(A.kin, { enabled: true, longOnly: false });
      check(r, "popup reply");
      eq(await A.kin("downloadsRegistered"), true, "listener registered");
      const before = alertRows().length;
      const d = await A.download("/dl/zq-quick.txt");
      eq(d.state, "complete", "download state");
      fastName = baseOf(d.filename);
      names.push(fastName);
      const row = await until("an alert row on the phone", () => alertRows().slice(before).find((r) => r.body === fastName), 25_000);
      eq([row.dir, row.status, row.title], ["down", "done", "Download finished"], "row");
      check(!/[\\/]/.test(row.body) && !/127\.0\.0\.1|http|localhost|Users|Temp/i.test(row.body), `the alert text carries no path or address: ${row.body}`);
      const n = await until("the kin_alerts notification", () => (alertNotifText().includes(fastName) ? alertNotifText() : ""), 15_000);
      check(/channel=kin_alerts/.test(n), "channel kin_alerts");
      check(!/127\.0\.0\.1|localhost|ms-playwright/.test(n), "no address or folder in the notification");
      phoneShot("s6-01-phone-download-alert");
    });

    await step("A: the PC kept a history row for it (Alert, same text) and the Kin log has no file name", async () => {
      const q = kinCli("--dev-dump-view", "history");
      await sleep(1500);
      const v = readFileSync(join(DATA, "view-history.json"), "utf8");
      check(v.includes(fastName), "the PC's history (the text table) holds the alert text by design");
      const log = readFileSync(join(DATA, "logs", "kin.log"), "utf8");
      check(!log.includes(fastName), "the PC log must not hold the file name");
      check(/extension: alert \(downloads, ttl 30m\)/.test(log) || /alert \(downloads/.test(log), `the PC log records an alert from the extension by counts only (${(log.match(/extension: alert[^\r\n]*/g) ?? []).slice(-2).join(" | ")})`);
      eq(q.status, 0, "the dump command");
    });

    await step("A: 30 s rule on: a quick download sends nothing, a real download over 30 s sends one alert", async () => {
      await setAlerts(A.kin, { enabled: true, longOnly: true });
      const before = alertRows().length;
      const q = await A.download("/dl/zq-quick-2.txt");
      eq(q.state, "complete", "quick state");
      await sleep(6000);
      eq(alertRows().length, before, "no alert for a quick download");
      const t0 = Date.now();
      const d = await A.download("/dl/slow.bin");
      eq(d.state, "complete", `slow state (${d.error})`);
      check(Date.now() - t0 >= 30_000, `the slow download took ${Date.now() - t0} ms`);
      const name = baseOf(d.filename);
      names.push(name);
      const row = await until("the alert of the slow download", () => alertRows().slice(before).find((r) => r.body === name), 25_000);
      eq([row.title, row.status], ["Download finished", "done"], "row");
    });

    await step("A: an interrupted download reaches the phone as 'Download failed'", async () => {
      await setAlerts(A.kin, { enabled: true, longOnly: false });
      const before = alertRows().length;
      const d = await A.download("/dl/broken.bin");
      eq(d.state, "interrupted", `state (error ${d.error})`);
      const name = baseOf(d.filename);
      names.push(name);
      const row = await until("the failed alert", () => alertRows().slice(before).find((r) => r.title === "Download failed"), 25_000);
      check(row, "row");
    });

    await step("A: the PC is paused: PAUSED_SKIPPED, no error in the popup, nothing reaches the phone", async () => {
     try {
      const p = kinCli("--pause", "1h");
      eq(p.status, 0, "--pause 1h");
      await until("the PC to be paused", () => statusOf().pause?.on === true, 10_000);
      await A.kin("refreshStatus");
      const st0 = await A.kin("handlePopup", { kin: "state" });
      const before = alertRows().length;
      const d = await A.download("/dl/zq-paused.txt");
      eq(d.state, "complete", "state");
      names.push(baseOf(d.filename));
      await sleep(7000);
      eq(alertRows().length, before, "no alert row on the phone while the PC is paused");
      const st = await A.kin("handlePopup", { kin: "state" });
      eq(st.history.length, st0.history.length, "popup history unchanged");
      eq(st.lastResult, st0.lastResult, "no new last result (no error)");
      const log = readFileSync(join(DATA, "logs", "kin.log"), "utf8");
      check(/extension: alert \(downloads\) skipped, this PC is paused/.test(log), `the PC log says it skipped the alert (${(log.match(/extension: alert[^\r\n]*/g) ?? []).slice(-2).join(" | ")})`);
     } finally {
      // whatever happened above, the test Kin is resumed: a paused PC would silently skip what the later scenarios send
      const r = kinCli("--pause", "resume");
      eq(r.status, 0, "--pause resume");
      await until("the PC to be resumed", () => statusOf().pause?.on === false, 10_000);
     }
    });

    await step("A: the switch off in the popup: the next download sends nothing", async () => {
      await setAlerts(A.kin, { enabled: false });
      const before = alertRows().length;
      const d = await A.download("/dl/zq-after-off.txt");
      eq(d.state, "complete", "state");
      await sleep(6000);
      eq(alertRows().length, before, "no alert after switching off");
    });

    await page.close();
    await A.ctx.close();
    context = undefined;

    // ================================================================ phase B: does a download alert start Kin when it is closed? (a pipe nobody serves + the harmless mock Kin)
    const state = join(work, "mock-state");
    mkdirSync(state, { recursive: true });
    const B = await launch("profile-b", { KIN_HOST_DEV_PIPE: PIPE_NOBODY, KIN_HOST_DEV_KIN_EXE: MOCK_EXE, KIN_MOCK_PIPE: PIPE_NOBODY, KIN_MOCK_STATE: state });
    context2 = B.ctx;
    const pageB = await B.ctx.newPage();
    await pageB.goto(siteUrl + "/");
    const startsLog = join(state, "starts.log");
    const startsCount = () => (existsSync(startsLog) ? readFileSync(startsLog, "utf8").split(/\r?\n/).filter(Boolean).length : 0);
    const collectPids = () => {
      if (existsSync(startsLog)) for (const l of readFileSync(startsLog, "utf8").split(/\r?\n/).filter(Boolean)) if (/^\d+$/.test(l) && !mockPids.includes(Number(l))) mockPids.push(Number(l));
    };
    await step("B: Kin is closed (no pipe) and the alert switch is on: a download finishes and Kin is NOT started (the host never starts the stand-in)", async () => {
      await setAlerts(B.kin, { enabled: true, longOnly: false });
      await B.kin("refreshStatus");
      eq(startsCount(), 0, "starts before");
      const d = await B.download("/dl/zq-closed.txt");
      eq(d.state, "complete", "state");
      await sleep(7000);
      collectPids();
      eq(startsCount(), 0, "the host started the stand-in Kin for a download alert");
    });
    await step("B: control: a real send (the context menu 'send selection') with Kin closed DOES make the host start the stand-in (so the check above could have failed)", async () => {
      const tabInfo = await (await B.getSw()).evaluate(async (p) => (await chrome.tabs.query({})).find((t) => (t.url ?? "").startsWith(p)), siteUrl);
      check(tabInfo, "a tab");
      await B.kin("menuClick", { menuItemId: "kin:send", selectionText: "zq control" }, tabInfo);
      await until("the host to start the stand-in", () => startsCount() >= 1, 20_000);
      collectPids();
      check(mockPids.length >= 1, "a start was logged");
    });
    await pageB.close().catch(() => {});
    await B.ctx.close();
    context2 = undefined;
  } finally {
    if (context) await context.close().catch(() => {});
    if (context2) await context2.close().catch(() => {});
    // The stand-in Kin processes the host started: stopped by the pids it wrote (never by name)
    try {
      const f = join(work, "mock-state", "starts.log");
      if (existsSync(f))
        for (const l of readFileSync(f, "utf8").split(/\r?\n/).filter(Boolean)) {
          if (!/^\d+$/.test(l)) continue;
          // only if that pid is still the stand-in program (a pid can be reused after the process ended)
          const path = spawnSync("powershell", ["-NoProfile", "-Command", `(Get-Process -Id ${l} -ErrorAction SilentlyContinue).Path`], { encoding: "utf8", windowsHide: true }).stdout.trim();
          if (path && path.toLowerCase() === MOCK_EXE.toLowerCase()) spawnSync("taskkill", ["/PID", l, "/F"], { windowsHide: true });
        }
    } catch {}
    site.stop(true);
    unregister();
    if (regQuery()) console.error(`WARNING: ${REG_KEY} is still there!`);
    await sleep(500);
    try {
      rmSync(work, { recursive: true, force: true });
    } catch {}
  }

  const failed = results.filter((r) => !r.ok);
  writeFileSync(OUT, JSON.stringify({ results, names }, null, 2));
  console.log(`\n${results.length - failed.length}/${results.length} steps passed; registry key ${regQuery() ? "STILL PRESENT" : "removed"}`);
  return failed.length === 0 && !regQuery() ? 0 : 1;
}

main().then(
  (code) => process.exit(code),
  (e) => {
    console.error(e);
    unregister();
    process.exit(1);
  },
);
