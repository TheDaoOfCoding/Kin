// Scenario S5 of tools/e2e-2b-1.ps1: the browser extension (headless Chromium, throwaway profile) -> the REAL dev native host -> the REAL test Kin (a dev
// instance beside the installed one, started and paired with the emulator by the PowerShell script) -> the EMULATOR phone. Run only through the script:
// it passes the paths in E2E_* variables and holds the scripts' lock.
//
// Safety, by construction (the same rules as desktop/native-host/smoke/run-real-kin.ts):
//  - headless only, a throwaway --user-data-dir under the work folder, the Playwright Chromium (never the user's browser or profile)
//  - the only registry key written is HKCU\Software\Chromium\NativeMessagingHosts\app.kinrelay.dev (the DEV host name) pointing at the dev host exe; refused if it
//    already exists, removed in `finally`, on a signal and on exit. The release keys (app.kinrelay.desktop) are never touched.
//  - the dev host is pointed at the test Kin's own pipe (KIN_HOST_DEV_PIPE); KIN_HOST_DEV_KIN_EXE is never set, so the host can't start any Kin (a build-folder Kin.exe
//    started by the host would run with no arguments, on the installed Kin's data directory)
//  - nothing is killed by name: the only thing stopped here is the test Kin, through `Kin.exe --data-dir <ours> --quit`
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium, type BrowserContext, type Worker } from "../extension/node_modules/playwright-core";

const here = resolve(dirname(fileURLToPath(import.meta.url)));
const repo = resolve(here, "..");
const env = (n: string): string => {
  const v = process.env[n];
  if (!v) throw new Error(`${n} is not set (run tools/e2e-2b-1.ps1)`);
  return v;
};
const DATA = env("E2E_DATA");
const SUFFIX = env("E2E_SUFFIX");
const KIN_EXE = env("E2E_KIN");
const HOST_EXE = env("E2E_HOST_EXE");
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
const sha = (b: Buffer) => createHash("sha256").update(b).digest("hex");

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
const phonePictures = (): Record<string, string> => {
  const h: Record<string, string> = {};
  const out = adb("shell", "sha256sum /sdcard/Pictures/Kin/* /sdcard/Download/Kin/* 2>/dev/null").stdout ?? "";
  for (const l of out.split(/\r?\n/)) {
    const m = /^([0-9a-f]{64})\s+(.+)$/.exec(l.trim());
    if (m) h[m[2]] = m[1];
  }
  return h;
};
const waitPhoneFile = (what: string, hash: string, ms = 30_000) =>
  until(what, () => Object.entries(phonePictures()).find(([, s]) => s === hash)?.[0], ms);
function notifBlocks(): string[] {
  const raw = adb("shell", "dumpsys notification --noredact").stdout ?? "";
  return raw.split(/^\s*NotificationRecord\(/m).filter((b) => b.includes(`pkg=${PKG} `) && !b.includes("tag=links-summary"));
}
const field = (block: string, name: string): string | null => {
  const m = new RegExp(`android\\.${name}=\\w+ \\(([^\\r\\n]*)\\)\\r?\\n`).exec(block);
  return m ? m[1] : null;
};
const notifByTitle = (part: string) => notifBlocks().find((b) => (field(b, "title") ?? "").includes(part));
const waitRow = (what: string, pred: (r: Row) => boolean, ms = 30_000) => until(what, () => phoneRows().find(pred), ms);
function phoneShot(name: string): void {
  if (!SHOTS) return;
  mkdirSync(SHOTS, { recursive: true });
  adb("shell", "cmd statusbar expand-notifications");
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 1500);
  const r = spawnSync(ADB, ["-s", SERIAL, "exec-out", "screencap", "-p"], { maxBuffer: 64 * 1024 * 1024, windowsHide: true });
  if (r.stdout && r.stdout.length > 1000) writeFileSync(join(SHOTS, `${name}.png`), r.stdout);
  adb("shell", "cmd statusbar collapse");
}

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

// ---- test site (served on loopback; Kin on this PC downloads from it) ------------------------------------
const PNG = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==", "base64");
const PNG2 = Buffer.concat([PNG, Buffer.from("secret-image-padding")]);
const BIG = Buffer.concat([PNG, Buffer.alloc(5 * 1024 * 1024, 0x5a)]);
const requests: { path: string; authed: boolean }[] = [];
function startSite() {
  return Bun.serve({
    hostname: "127.0.0.1",
    port: 0,
    fetch(req) {
      const url = new URL(req.url);
      const authed = (req.headers.get("cookie") ?? "").includes("auth=1");
      requests.push({ path: url.pathname, authed });
      switch (url.pathname) {
        case "/":
          return new Response(`<!doctype html><title>Kin test page</title><h1>Hello</h1>`, { headers: { "content-type": "text/html" } });
        case "/img/ok.png":
          return new Response(PNG, { headers: { "content-type": "image/png" } });
        case "/img/secret.png":
          return authed ? new Response(PNG2, { headers: { "content-type": "image/png" } }) : new Response("sign in", { status: 403 });
        case "/img/big.png":
          return authed ? new Response(BIG, { headers: { "content-type": "image/png" } }) : new Response("sign in", { status: 403 });
        default:
          return new Response("not found", { status: 404 });
      }
    },
  });
}
const youtubeHtml = `<!doctype html><title>Fake YouTube</title>
<div id="movie_player" class="html5-video-player"><video class="html5-main-video" muted></video></div>
<script>
  const rate = 8000, seconds = 100, n = rate * seconds, buf = new Uint8Array(44 + n).fill(128), dv = new DataView(buf.buffer);
  const tag = (o, s) => [...s].forEach((c, i) => (buf[o + i] = c.charCodeAt(0)));
  tag(0, "RIFF"); dv.setUint32(4, 36 + n, true); tag(8, "WAVEfmt "); dv.setUint32(16, 16, true); dv.setUint16(20, 1, true);
  dv.setUint16(22, 1, true); dv.setUint32(24, rate, true); dv.setUint32(28, rate, true); dv.setUint16(32, 1, true); dv.setUint16(34, 8, true);
  tag(36, "data"); dv.setUint32(40, n, true);
  const v = document.querySelector("video");
  v.addEventListener("loadedmetadata", () => { v.currentTime = 83.4; });
  v.src = URL.createObjectURL(new Blob([buf], { type: "audio/wav" }));
</script>`;

// ---- main ----------------------------------------------------------------------------------------------
const statusOf = (): any => {
  try {
    return JSON.parse(readFileSync(join(DATA, "status.json"), "utf8"));
  } catch {
    return {};
  }
};
const kinLog = () => {
  const f = join(DATA, "logs", "kin.log");
  return existsSync(f) ? readFileSync(f, "utf8") : "";
};

async function main(): Promise<number> {
  for (const f of [join(extDir, "manifest.json"), HOST_EXE, KIN_EXE]) check(existsSync(f), `missing ${f}`);
  if (regQuery()) throw new Error(`${REG_KEY} already exists. Not touching it: remove it by hand if it is a leftover.`);
  const sid = spawnSync("powershell", ["-NoProfile", "-Command", "[Security.Principal.WindowsIdentity]::GetCurrent().User.Value"], { encoding: "utf8" }).stdout.trim();
  check(/^S-1-5-/.test(sid), "could not read the user SID");
  const PIPE = `\\\\.\\pipe\\${HOST}.${sid}.${SUFFIX}`;
  const browser = findChromium();
  console.log(`browser: ${browser}\nKin: ${KIN_EXE}\nphone: ${SERIAL}`);
  check(statusOf().state === "phone_connected", `the test Kin is not connected to the phone (state ${statusOf().state})`);
  const phoneName = statusOf().devices?.[0]?.name as string;

  const work = join(WORK, "s5");
  const profile = join(work, "profile");
  mkdirSync(profile, { recursive: true });
  const manifestPath = join(work, `${HOST}.json`);
  writeFileSync(manifestPath, JSON.stringify({ name: HOST, description: "Kin e2e 2B.1", path: HOST_EXE, type: "stdio", allowed_origins: [`chrome-extension://${EXPECTED_ID}/`] }, null, 2));
  const site = startSite();
  const siteUrl = `http://127.0.0.1:${site.port}`;
  let context: BrowserContext | undefined;
  const stamp = Date.now().toString(36);

  try {
    const add = spawnSync("reg", ["add", REG_KEY, "/ve", "/t", "REG_SZ", "/d", manifestPath, "/f"], { encoding: "utf8" });
    check(add.status === 0, `reg add failed: ${add.stderr}`);
    registered = true;

    context = await chromium.launchPersistentContext(profile, {
      executablePath: browser,
      headless: true,
      env: { ...(process.env as Record<string, string>), KIN_HOST_DEV_PIPE: PIPE },
      args: [`--disable-extensions-except=${extDir}`, `--load-extension=${extDir}`, "--mute-audio", "--no-first-run", "--no-default-browser-check", "--disable-sync", "--disable-background-networking"],
      viewport: { width: 900, height: 700 },
    });
    const ctx = context;
    await ctx.addCookies([{ name: "auth", value: "1", url: siteUrl }]);
    await ctx.route("https://www.youtube.com/**", (route) => route.fulfill({ contentType: "text/html", body: youtubeHtml }));
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
    const findTab = async (prefix: string): Promise<any> => {
      const w = await getSw();
      const tab = await w.evaluate(async (p) => (await chrome.tabs.query({})).find((t) => (t.url ?? "").startsWith(p)), prefix);
      check(tab, `no tab starting with ${prefix}`);
      return tab;
    };
    const popupUrl = `chrome-extension://${EXPECTED_ID}/popup/popup.html`;
    const click = (info: Record<string, unknown>, tab?: unknown) => kin("menuClick", info, tab);
    const popupShot = async (p: import("playwright-core").Page, name: string) => {
      if (!SHOTS) return;
      mkdirSync(SHOTS, { recursive: true });
      await p.screenshot({ path: join(SHOTS, `${name}.png`) });
    };

    const page = await ctx.newPage();
    await page.goto(siteUrl + "/");
    const tab = await findTab(siteUrl);
    let deviceId = "";
    const menu = () => ({ menuItemId: `kin:dev:${deviceId}` });

    await step("extension loads; the real host and the real Kin answer status; the EMULATOR is the online device", async () => {
      eq(new URL((await getSw()).url()).host, EXPECTED_ID, "extension id");
      const view = await kin("refreshStatus");
      eq(view.ok, true, `view.ok (${JSON.stringify(view)})`);
      eq(view.status.devices.length, 1, "one device");
      eq([view.status.devices[0].name, view.status.devices[0].online], [phoneName, true], "the emulator phone is online");
      deviceId = view.status.devices[0].id;
      eq(view.status.defaultDeviceId, deviceId, "default device");
    });

    await step("popup shows Kin running and the emulator online", async () => {
      const p = await ctx.newPage();
      await p.goto(popupUrl);
      await p.waitForFunction(() => document.getElementById("state")?.textContent === "Kin is running", null, { timeout: 10_000 });
      check((await p.locator("#devices").innerText()).includes(phoneName), "device row");
      await popupShot(p, "s5-01-popup-running");
      await p.close();
    });

    await step("selected text reaches the emulator as a text, byte for byte", async () => {
      const text = `e2e selection ${stamp}: caf\u00e9 \u2713 line1\nline2`;
      const out = await click({ ...menu(), selectionText: text, pageUrl: siteUrl + "/" }, tab);
      eq(out.outcome, "sent", `outcome (${out.message})`);
      const row = await waitRow("the text on the phone", (r) => r.dir === "down" && r.kind === "text" && r.body === text);
      eq(row.status, "done", "status");
    });

    await step("a link reaches the emulator as a link: URL exactly, title cleaned, a notification with the real domain", async () => {
      const url = "https://kin-link-test.example/watch?v=dQw4w9WgXcQ&t=5s&list=PLx#frag";
      const out = await click({ ...menu(), linkUrl: url, linkText: "  e2e \n link   title ", pageUrl: siteUrl + "/" }, tab);
      eq(out.outcome, "sent", `outcome (${out.message})`);
      const row = await waitRow("the link on the phone", (r) => r.dir === "down" && r.kind === "link" && r.body === url);
      eq(row.title, "e2e link title", "title cleaned");
      const n = await until("its notification", () => notifByTitle("e2e link title"));
      eq(field(n, "text"), "kin-link-test.example", "the real domain under the title");
      phoneShot("s5-02-phone-link-and-text");
    });

    await step("the page goes as a link with its title", async () => {
      const out = await click({ ...menu(), pageUrl: siteUrl + "/" }, tab);
      eq(out.outcome, "sent", `outcome (${out.message})`);
      await waitRow("the page on the phone", (r) => r.dir === "down" && r.kind === "link" && r.body === siteUrl + "/" && r.title === "Kin test page");
    });

    await step("YouTube: the time is read at click time; the emulator gets t=83s (old t replaced, list kept)", async () => {
      const yt = await ctx.newPage();
      await yt.goto("https://www.youtube.com/watch?v=abcdefghijk&t=5s&list=PLx");
      await yt.waitForFunction(() => (document.querySelector("video")?.currentTime ?? 0) > 83, null, { timeout: 10_000 });
      const ytTab = await findTab("https://www.youtube.com/");
      const out = await kin("sendTab", ytTab, deviceId);
      eq(out.outcome, "sent", "outcome");
      await waitRow("the YouTube link", (r) => r.dir === "down" && r.kind === "link" && r.body === "https://www.youtube.com/watch?v=abcdefghijk&list=PLx&t=83s");
      await yt.close();
    });

    await step("image by URL: Kin downloads it, the emulator gets the same bytes", async () => {
      const out = await click({ ...menu(), srcUrl: `${siteUrl}/img/ok.png`, mediaType: "image", pageUrl: siteUrl + "/", frameId: 0 }, tab);
      eq(out.outcome, "sent", `outcome (${out.message})`);
      const name = await waitPhoneFile("ok.png on the phone", sha(PNG));
      check(/ok\.png$/.test(name), `file name ${name}`);
      check(requests.some((r) => r.path === "/img/ok.png" && !r.authed), "Kin fetched it without the page's cookies");
    });

    await step("image needing bytes: Kin gets 403 (no cookies), the extension fetches it in the page, the bytes land on the emulator", async () => {
      const out = await click({ ...menu(), srcUrl: `${siteUrl}/img/secret.png`, mediaType: "image", pageUrl: siteUrl + "/", frameId: 0 }, tab);
      eq(out.outcome, "sent", `outcome (${out.message})`);
      await waitPhoneFile("secret.png on the phone", sha(PNG2));
      check(requests.some((r) => r.path === "/img/secret.png" && !r.authed), "Kin's own download (no cookie) was tried first");
      check(requests.some((r) => r.path === "/img/secret.png" && r.authed), "then the page fetched it with its cookie");
    });

    await step("image needing bytes, 5 MiB: one big native message, arrives intact", async () => {
      const out = await click({ ...menu(), srcUrl: `${siteUrl}/img/big.png`, mediaType: "image", pageUrl: siteUrl + "/", frameId: 0 }, tab);
      eq(out.outcome, "sent", `outcome (${out.message})`);
      await waitPhoneFile("big.png on the phone", sha(BIG), 90_000);
    });

    await step("image from a data: URL goes as bytes", async () => {
      const data = Buffer.concat([PNG, Buffer.from("data-url-padding")]);
      const out = await click({ ...menu(), srcUrl: "data:image/png;base64," + data.toString("base64"), mediaType: "image", frameId: 0 }, tab);
      eq(out.outcome, "sent", `outcome (${out.message})`);
      await waitPhoneFile("the data: image on the phone", sha(data));
    });

    await step("an image neither Kin nor the page can get gives a clear failure (IMAGE_UNREACHABLE)", async () => {
      const out = await click({ ...menu(), srcUrl: `${siteUrl}/img/missing.png`, mediaType: "image", pageUrl: siteUrl + "/", frameId: 0 }, tab);
      eq([out.outcome, out.code], ["failed", "IMAGE_UNREACHABLE"], "outcome");
    });

    await step("a bad link is refused (BAD_URL) before the host, and the same sent straight over the pipe is refused by Kin", async () => {
      const before = phoneRows().length;
      for (const bad of ["javascript:alert(1)", "file:///C:/Windows/win.ini"]) {
        const out = await click({ ...menu(), linkUrl: bad }, tab);
        eq([out.outcome, out.code], ["failed", "BAD_URL"], `outcome for ${bad}`);
      }
      const sw = await getSw();
      const answer = await sw.evaluate(
        (h) => new Promise<any>((res) => chrome.runtime.sendNativeMessage(h, { v: 1, id: "raw-1", type: "send", kind: "link", url: "javascript:alert(1)" }, (r) => res(r))),
        HOST,
      );
      eq([answer?.ok, answer?.code, answer?.ref], [false, "BAD_URL", "raw-1"], "Kin refuses by itself");
      await sleep(1000);
      eq(phoneRows().length, before, "nothing more reached the phone");
    });

    await step("a text over 128 KiB reaches the emulator as Text.txt", async () => {
      const big = "x".repeat(200 * 1024);
      const out = await click({ ...menu(), selectionText: big, pageUrl: siteUrl + "/" }, tab);
      eq(out.outcome, "sent", `outcome (${out.message})`);
      await waitPhoneFile("Text.txt on the phone", sha(Buffer.from(big)));
    });

    await step("toolbar popup: Send this tab (the same path as the keyboard shortcut) lands on the emulator", async () => {
      const p = await ctx.newPage();
      await p.goto(popupUrl);
      await p.waitForFunction(() => document.getElementById("state")?.textContent === "Kin is running", null, { timeout: 10_000 });
      await page.bringToFront();
      await p.locator("#send-tab").click({ force: true });
      await p.waitForFunction(() => /Sent|queued/i.test(document.getElementById("result")?.textContent ?? ""), null, { timeout: 15_000 });
      check((await p.locator("#history li").count()) >= 1, "history shows the send");
      await popupShot(p, "s5-03-popup-after-send");
      await p.close();
    });

    await step("the phone is offline: the send is queued, the popup says offline, and it arrives when the phone is back", async () => {
      adb("shell", "am", "force-stop", PKG);
      await until("Kin to see the phone gone", () => statusOf().state === "phone_offline", 60_000);
      const view = await kin("refreshStatus");
      eq(view.status.devices[0].online, false, "device offline");
      const url = `https://example.com/queued-while-offline-${stamp}`;
      const out = await click({ ...menu(), linkUrl: url, linkText: "queued while offline" }, tab);
      eq(out.outcome, "queued", `outcome (${out.message})`);
      check(/offline/.test(out.message), out.message);
      eq((await kin("refreshStatus")).status.devices[0].queuedCount, 1, "queuedCount");
      const p = await ctx.newPage();
      await p.goto(popupUrl);
      await p.waitForFunction(() => /offline/i.test(document.getElementById("devices")?.textContent ?? ""), null, { timeout: 10_000 });
      await popupShot(p, "s5-04-popup-phone-offline-queued");
      await p.close();
      adb("shell", "am", "start", "-n", `${PKG}/.MainActivity`);
      await sleep(3000);
      adb("shell", "am", "broadcast", "-a", "app.kinrelay.android.debug.CONNECT", "-n", `${PKG}/.debug.DebugCommandReceiver`);
      await until("the phone to reconnect", () => statusOf().state === "phone_connected", 60_000);
      await waitRow("the queued link on the phone", (r) => r.dir === "down" && r.kind === "link" && r.body === url && r.status === "done", 60_000);
      eq((await kin("refreshStatus")).status.devices[0].queuedCount, 0, "queue empty");
    });

    await step("Kin quits: the popup says Kin isn't running, a send says so, Start Kin can't start it here", async () => {
      const q = spawnSync(KIN_EXE, ["--data-dir", DATA, "--quit"], { encoding: "utf8", windowsHide: true, timeout: 30_000 });
      eq(q.status, 0, "the --quit command");
      await until("Kin to be gone (no pipe)", async () => !(await kin("refreshStatus")).status?.running, 30_000);
      const p = await ctx.newPage();
      await p.goto(popupUrl);
      await p.waitForFunction(() => document.getElementById("state")?.textContent === "Kin isn't running", null, { timeout: 10_000 });
      await popupShot(p, "s5-05-popup-kin-not-running");
      await p.close();
      const out = await click({ menuItemId: "kin:send", selectionText: "x" }, tab);
      eq([out.outcome, out.code], ["failed", "KIN_NOT_RUNNING"], "outcome");
    });

    await page.close();
    await ctx.close();
    context = undefined;
  } finally {
    if (context) await context.close().catch(() => {});
    site.stop(true);
    unregister();
    if (regQuery()) console.error(`WARNING: ${REG_KEY} is still there!`);
    await sleep(500);
    try {
      rmSync(join(WORK, "s5"), { recursive: true, force: true });
    } catch {}
  }

  const failed = results.filter((r) => !r.ok);
  writeFileSync(OUT, JSON.stringify(results, null, 2));
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
