// Headless Chromium smoke test of the extension against a MOCK native host.
//   bun scripts/build.ts --test && bun tests/smoke/run.ts
//
// Safety, by construction:
//  - headless only, and a throwaway --user-data-dir under the OS temp folder: never a user's profile
//  - the browser is the Playwright Chromium from %LOCALAPPDATA%\ms-playwright (not the user's Chrome)
//  - the only registry key written is HKCU\Software\Chromium\NativeMessagingHosts\app.kinrelay.dev,
//    pointing at the mock host; it is removed in `finally`, on a signal and on exit
//  - the mock host never touches Kin; the pages come from a server on 127.0.0.1
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium, type BrowserContext, type Page, type Worker } from "playwright-core";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const extDir = join(root, "dist", "chromium-test");
const shotsDir = resolve(root, "dist/smoke-shots/popup");
const shots5Dir = resolve(root, "dist/smoke-shots/download-alerts");
const EXPECTED_ID = "cbioefpddjpaionhljhffgbedncfchmn";
const HOST = "app.kinrelay.dev";
const REG_KEY = `HKCU\\Software\\Chromium\\NativeMessagingHosts\\${HOST}`;

// ---- small test harness -----------------------------------------------------------------------

const results: { name: string; ok: boolean; detail?: string }[] = [];
async function step(name: string, fn: () => Promise<void>): Promise<void> {
  // While developing: KIN_SMOKE_ONLY="downloads: a,downloads: b" runs only the steps whose name starts with one of these.
  if (process.env.KIN_SMOKE_ONLY && !process.env.KIN_SMOKE_ONLY.split(",").some((p) => name.startsWith(p))) return;
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

// ---- registry (HKCU, dev host name only) -------------------------------------------------------

function regQuery(): string | null {
  const r = spawnSync("reg", ["query", REG_KEY, "/ve"], { encoding: "utf8" });
  return r.status === 0 ? r.stdout : null;
}
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

// ---- find the browser --------------------------------------------------------------------------

function findChromium(): string {
  if (process.env.KIN_CHROMIUM && existsSync(process.env.KIN_CHROMIUM)) return process.env.KIN_CHROMIUM;
  const base = join(process.env.LOCALAPPDATA ?? "", "ms-playwright");
  if (existsSync(base)) {
    const dirs = readdirSync(base).filter((d) => /^chromium-\d+$/.test(d)).sort((a, b) => Number(b.split("-")[1]) - Number(a.split("-")[1]));
    for (const d of dirs) {
      const exe = join(base, d, "chrome-win64", "chrome.exe");
      if (existsSync(exe)) return exe;
    }
  }
  throw new Error("No Playwright Chromium found. Run `bunx playwright-core install chromium` (installs into %LOCALAPPDATA%\\ms-playwright, no admin) or set KIN_CHROMIUM.");
}

// ---- test site ---------------------------------------------------------------------------------

const PNG = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==", "base64");
const BIG = Buffer.concat([PNG, Buffer.alloc(5 * 1024 * 1024, 0x5a)]); // a PNG header followed by 5 MiB of filler

function startSite() {
  return Bun.serve({
    hostname: "127.0.0.1",
    port: 0,
    idleTimeout: 0, // the 31.5 s download below must not be cut at Bun's default 10 s
    fetch(req) {
      const url = new URL(req.url);
      const authed = (req.headers.get("cookie") ?? "").includes("auth=1");
      switch (url.pathname) {
        case "/":
          return new Response(`<!doctype html><title>Kin test page</title><h1>Hello</h1><a href="/other">a link</a><img id="pub" src="/img/ok.png"><img id="sec" src="/img/secret.png">`, { headers: { "content-type": "text/html" } });
        case "/img/ok.png":
          return new Response(PNG, { headers: { "content-type": "image/png" } });
        case "/img/secret.png":
          return authed ? new Response(PNG, { headers: { "content-type": "image/png" } }) : new Response("sign in", { status: 403 });
        case "/img/big.png":
          return authed ? new Response(BIG, { headers: { "content-type": "image/png" } }) : new Response("sign in", { status: 403 });
        case "/dl/slow.bin": {
          // Headers and the first chunk now, the rest after 31.5 s: a download that really takes over 30 s.
          // (The browser starts counting when the body starts, so a late header would not do.)
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
            { headers: { "content-type": "application/octet-stream", "content-disposition": 'attachment; filename="slow-render.bin"' } },
          );
        }
        case "/dl/hang.bin": {
          // Never finishes: the test cancels it.
          return new Response(new ReadableStream({ start: (c) => c.enqueue(new Uint8Array(1000).fill(0x63)) }), {
            headers: { "content-type": "application/octet-stream", "content-disposition": 'attachment; filename="hang.bin"' },
          });
        }
        case "/dl/broken.bin": {
          // Promises 100000 bytes, sends 1000 and drops the connection: the browser marks it interrupted.
          return new Response(
            new ReadableStream({
              start(c) {
                c.enqueue(new Uint8Array(1000).fill(0x64));
                setTimeout(() => c.error(new Error("drop")), 100);
              },
            }),
            { headers: { "content-type": "application/octet-stream", "content-length": "100000", "content-disposition": 'attachment; filename="broken.bin"' } },
          );
        }
        case "/dl/broken-slow.bin": {
          // Like broken.bin, but the connection drops only after 31.5 s: an interrupted download that took over 30 s.
          let timer: ReturnType<typeof setTimeout>;
          return new Response(
            new ReadableStream({
              start(c) {
                c.enqueue(new Uint8Array(1000).fill(0x65));
                timer = setTimeout(() => c.error(new Error("drop")), 31_500);
              },
              cancel() {
                clearTimeout(timer);
              },
            }),
            { headers: { "content-type": "application/octet-stream", "content-length": "100000", "content-disposition": 'attachment; filename="broken-slow.bin"' } },
          );
        }
        default:
          // /dl/<name>: a small file that downloads as <name>
          if (url.pathname.startsWith("/dl/")) {
            const name = decodeURIComponent(url.pathname.slice(4));
            return new Response("hello from the test server\n", { headers: { "content-type": "application/octet-stream", "content-disposition": `attachment; filename="${name}"` } });
          }
          return new Response("not found", { status: 404 });
      }
    },
  });
}

// A real (silent) media element, so currentTime and duration are the browser's own values and are
// visible to the extension's isolated world: a 100 s WAV built in the page, parked at 83.4 s.
const youtubeHtml = (ad: boolean) => `<!doctype html><title>Fake YouTube</title>
<div id="movie_player" class="html5-video-player${ad ? " ad-showing" : ""}"><video class="html5-main-video" muted></video></div>
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

// ---- main --------------------------------------------------------------------------------------

async function main(): Promise<number> {
  check(existsSync(join(extDir, "manifest.json")), `${extDir} is not built. Run: bun scripts/build.ts --test`);
  const exe = findChromium();
  console.log(`browser: ${exe}`);
  if (regQuery() !== null) throw new Error(`${REG_KEY} already exists. Not touching it: remove it by hand if it is a leftover of an old run.`);

  const work = join(tmpdir(), `kin-ext-smoke-${process.pid}-${Date.now().toString(36)}`);
  const profile = join(work, "profile");
  const hostDir = join(work, "host");
  const state = join(hostDir, "state");
  mkdirSync(profile, { recursive: true });
  mkdirSync(state, { recursive: true });
  mkdirSync(shotsDir, { recursive: true });
  mkdirSync(shots5Dir, { recursive: true });

  // The mock host and its wrapper. Browsers start a .cmd on Windows through cmd.exe.
  const mock = join(root, "tests", "smoke", "mock-host.ts");
  const cmdPath = join(hostDir, "mock-host.cmd");
  writeFileSync(cmdPath, `@echo off\r\n"${process.execPath}" "${mock}" --state "${state}" %*\r\n`);
  const manifestPath = join(hostDir, `${HOST}.json`);
  writeFileSync(manifestPath, JSON.stringify({ name: HOST, description: "Kin mock host (test)", path: cmdPath, type: "stdio", allowed_origins: [`chrome-extension://${EXPECTED_ID}/`] }, null, 2));

  const site = startSite();
  const siteUrl = `http://127.0.0.1:${site.port}`;
  let context: BrowserContext | undefined;
  const logFile = join(state, "log.jsonl");

  const setScenario = (s: Record<string, unknown>) => writeFileSync(join(state, "scenario.json"), JSON.stringify(s));
  const resetLog = () => writeFileSync(logFile, "");
  const readLog = (): any[] => (existsSync(logFile) ? readFileSync(logFile, "utf8").split("\n").filter(Boolean).map((l) => JSON.parse(l)) : []);
  const requests = (type?: string) => readLog().filter((e) => e.req && (!type || e.req.type === type)).map((e) => e.req);

  const PIXEL = { id: "pixelDeviceIdAAAAAAAAAAAAAAAAAAAAAAAAAAAA", name: "Pixel 8", online: true, queuedCount: 0 };
  const GALAXY = { id: "galaxyDeviceIdBBBBBBBBBBBBBBBBBBBBBBBBBBB", name: "Galaxy & Co", online: false, queuedCount: 2 };
  const scenarioTwo = { running: true, startable: true, devices: [PIXEL, GALAXY], defaultDeviceId: PIXEL.id };
  const scenarioOne = { running: true, startable: true, devices: [PIXEL], defaultDeviceId: PIXEL.id };

  try {
    // Register the mock host. Removed in the finally below (and on exit/signals).
    const add = spawnSync("reg", ["add", REG_KEY, "/ve", "/t", "REG_SZ", "/d", manifestPath, "/f"], { encoding: "utf8" });
    check(add.status === 0, `reg add failed: ${add.stderr}`);
    registered = true;

    setScenario(scenarioTwo);
    resetLog();

    context = await chromium.launchPersistentContext(profile, {
      executablePath: exe,
      headless: true, // never a visible window
      args: [`--disable-extensions-except=${extDir}`, `--load-extension=${extDir}`, "--mute-audio", "--no-first-run", "--no-default-browser-check", "--disable-sync", "--disable-background-networking"],
      viewport: { width: 900, height: 700 },
      // Downloads of the download alert steps land here, inside the throwaway work folder: never the user's Downloads.
      downloadsPath: join(work, "downloads"),
    });
    const ctx = context;
    await ctx.addCookies([{ name: "auth", value: "1", url: siteUrl }]);

    // The fake YouTube. Served by route interception: no network, no real YouTube.
    await ctx.route("https://www.youtube.com/**", (route) => route.fulfill({ contentType: "text/html", body: youtubeHtml(new URL(route.request().url()).searchParams.has("ad")) }));

    const getSw = async (): Promise<Worker> => {
      const find = () => ctx.serviceWorkers().find((w) => w.url().startsWith("chrome-extension://"));
      let w = find();
      if (!w) w = await ctx.waitForEvent("serviceworker", { timeout: 15_000 });
      return w;
    };
    const kin = async (name: string, ...args: unknown[]): Promise<any> => {
      for (let attempt = 0; attempt < 3; attempt++) {
        try {
          const w = await getSw();
          return await w.evaluate(([n, a]) => (globalThis as any).__kin[n](...(a as unknown[])), [name, args] as [string, unknown[]]);
        } catch (e) {
          if (attempt === 2) throw e;
          // The worker may have been stopped between calls; touching an extension page wakes it.
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

    const page = await ctx.newPage();
    await page.goto(siteUrl + "/");
    const tab = await findTab(siteUrl);

    await step("extension loads with the key-derived ID and the test hooks", async () => {
      const w = await getSw();
      eq(new URL(w.url()).host, EXPECTED_ID, "extension id");
      const menu = await kin("getMenu");
      check(Array.isArray(menu), "getMenu");
    });

    await step("status with two phones: view is ok, menu is a submenu with no create errors", async () => {
      const view = await kin("refreshStatus");
      eq(view.ok, true, "view.ok");
      eq(view.status.devices.map((d: any) => d.name), ["Pixel 8", "Galaxy & Co"], "devices");
      await new Promise((r) => setTimeout(r, 300));
      const menu = await kin("getMenu");
      eq(menu.map((m: any) => m.title), ["Send to Kin", "Pixel 8", "Galaxy && Co (offline, will queue)"], "menu titles");
      eq(await kin("getMenuErrors"), [], "menu errors");
      eq(requests("status").length >= 1, true, "host got a status request");
    });

    await step("popup shows Kin running, both phones, offline state and the queue (screenshot)", async () => {
      const p = await ctx.newPage();
      await p.goto(popupUrl);
      await p.waitForFunction(() => document.getElementById("state")?.textContent === "Kin is running", null, { timeout: 10_000 });
      eq(await p.locator("#devices li").count(), 2, "device rows");
      eq(await p.locator("#devices input[type=radio]").count(), 2, "radios");
      const text = await p.locator("#devices").innerText();
      check(text.includes("offline, will queue") && text.includes("2 waiting"), `device text: ${text}`);
      await p.screenshot({ path: join(shotsDir, "popup-running.png"), clip: { x: 0, y: 0, width: 360, height: 400 } });
      await p.close();
    });

    await step("menu: a link sent to the offline phone is queued and reports it", async () => {
      resetLog();
      const out = await click({ menuItemId: `kin:dev:${GALAXY.id}`, linkUrl: `${siteUrl}/other`, linkText: "  an \n interesting  link ", pageUrl: siteUrl + "/" }, tab);
      eq(out.outcome, "queued", "outcome");
      check(out.message.includes("offline"), `message: ${out.message}`);
      const sends = requests("send");
      eq(sends.length, 1, "send count");
      eq({ kind: sends[0].kind, url: sends[0].url, title: sends[0].title, deviceId: sends[0].deviceId, v: sends[0].v }, { kind: "link", url: `${siteUrl}/other`, title: "an interesting link", deviceId: GALAXY.id, v: 1 }, "link request");
      check(String(sends[0].id).length >= 8, "request id");
      eq(sends[0].origin === undefined, true, "extension doesn't invent an origin");
    });

    await step("menu: selected text goes out as text (to Kin's default device)", async () => {
      resetLog();
      const out = await click({ menuItemId: "kin:dev:" + PIXEL.id, selectionText: "hello from the browser", pageUrl: siteUrl + "/" }, tab);
      eq(out.outcome, "sent", "outcome");
      const s = requests("send")[0];
      eq({ kind: s.kind, text: s.text, deviceId: s.deviceId }, { kind: "text", text: "hello from the browser", deviceId: PIXEL.id }, "text request");
    });

    await step("menu: the page itself goes out as page with its title", async () => {
      resetLog();
      const out = await click({ menuItemId: "kin:dev:" + PIXEL.id, pageUrl: siteUrl + "/" }, tab);
      eq(out.outcome, "sent", "outcome");
      const s = requests("send")[0];
      eq({ kind: s.kind, url: s.url, title: s.title }, { kind: "page", url: siteUrl + "/", title: "Kin test page" }, "page request");
    });

    await step("menu: a bad link is refused before it reaches the host", async () => {
      resetLog();
      for (const bad of ["javascript:alert(1)", "file:///C:/Windows/win.ini", "ftp://example.com/x"]) {
        const out = await click({ menuItemId: "kin:dev:" + PIXEL.id, linkUrl: bad }, tab);
        eq([out.outcome, out.code], ["failed", "BAD_URL"], `outcome for ${bad}`);
      }
      eq(requests("send").length, 0, "nothing sent");
    });

    await step("image: Kin downloads a public image itself (URL first, no bytes)", async () => {
      resetLog();
      const out = await click({ menuItemId: "kin:dev:" + PIXEL.id, srcUrl: `${siteUrl}/img/ok.png`, mediaType: "image", pageUrl: siteUrl + "/", frameId: 0 }, tab);
      eq(out.outcome, "sent", "outcome");
      const sends = requests("send");
      eq(sends.length, 1, "one send");
      eq({ kind: sends[0].kind, url: sends[0].url, pageUrl: sends[0].pageUrl, name: sends[0].name, data: sends[0].data }, { kind: "image", url: `${siteUrl}/img/ok.png`, pageUrl: siteUrl + "/", name: "ok.png", data: undefined }, "image request");
    });

    await step("image: a sign-in image is retried with bytes fetched inside the page", async () => {
      resetLog();
      const out = await click({ menuItemId: "kin:dev:" + PIXEL.id, srcUrl: `${siteUrl}/img/secret.png`, mediaType: "image", pageUrl: siteUrl + "/", frameId: 0 }, tab);
      eq(out.outcome, "sent", "outcome");
      const sends = requests("send");
      eq(sends.length, 2, "two sends (url, then bytes)");
      eq(sends[0].url, `${siteUrl}/img/secret.png`, "first by URL");
      eq(sends[1].url, undefined, "second has no URL");
      eq([sends[1].mime, sends[1].name, sends[1].retryOf], ["image/png", "secret.png", sends[0].id], "second is the bytes retry");
      const { createHash } = await import("node:crypto");
      eq(sends[1].dataSha256, createHash("sha256").update(PNG).digest("hex"), "the bytes are the image");
      eq(sends[1].data, `<${PNG.length} bytes>`, "size");
    });

    await step("image: a 5 MiB sign-in image goes through as one big message", async () => {
      resetLog();
      const out = await click({ menuItemId: "kin:dev:" + PIXEL.id, srcUrl: `${siteUrl}/img/big.png`, mediaType: "image", pageUrl: siteUrl + "/", frameId: 0 }, tab);
      eq(out.outcome, "sent", `outcome (${out.message})`);
      const sends = requests("send");
      eq(sends.length, 2, "two sends");
      const { createHash } = await import("node:crypto");
      eq(sends[1].dataSha256, createHash("sha256").update(BIG).digest("hex"), "bytes intact");
    });

    await step("image: data: URL skips the URL step and sends bytes", async () => {
      resetLog();
      const out = await click({ menuItemId: "kin:dev:" + PIXEL.id, srcUrl: "data:image/png;base64," + PNG.toString("base64"), mediaType: "image", frameId: 0 }, tab);
      eq(out.outcome, "sent", `outcome (${out.message})`);
      const sends = requests("send");
      eq(sends.length, 1, "one send");
      eq([sends[0].data, sends[0].mime], [`<${PNG.length} bytes>`, "image/png"], "bytes request");
    });

    await step("image: when the page can't fetch it either, the user gets a clear failure", async () => {
      resetLog();
      const out = await click({ menuItemId: "kin:dev:" + PIXEL.id, srcUrl: `${siteUrl}/img/missing.png`, mediaType: "image", pageUrl: siteUrl + "/", frameId: 0 }, tab);
      eq([out.outcome, out.code], ["failed", "IMAGE_UNREACHABLE"], "outcome");
      check(out.message.includes("couldn't get that image"), out.message);
      eq(requests("send").length, 1, "only the URL attempt reached the host");
    });

    await step("YouTube: the video time is read at click time and written as t=83s (replacing the old t)", async () => {
      const yt = await ctx.newPage();
      await yt.goto("https://www.youtube.com/watch?v=abcdefghijk&t=5s&list=PLx");
      await yt.waitForFunction(() => (document.querySelector("video")?.currentTime ?? 0) > 83, null, { timeout: 10_000 });
      const ytTab = await findTab("https://www.youtube.com/");
      resetLog();
      const out = await kin("sendTab", ytTab, PIXEL.id);
      eq(out.outcome, "sent", "outcome");
      const s = requests("send")[0];
      eq([s.kind, s.url, s.timestampSec], ["page", "https://www.youtube.com/watch?v=abcdefghijk&list=PLx&t=83s", undefined], "page with t");

      // The same through the context menu on a link to the video on screen: kind link carries timestampSec.
      resetLog();
      await click({ menuItemId: "kin:dev:" + PIXEL.id, linkUrl: "https://youtu.be/abcdefghijk", linkText: "the video" }, ytTab);
      const l = requests("send")[0];
      eq([l.kind, l.url, l.timestampSec], ["link", "https://youtu.be/abcdefghijk?t=83s", 83], "link with t");

      // A link to a different video is left alone.
      resetLog();
      await click({ menuItemId: "kin:dev:" + PIXEL.id, linkUrl: "https://www.youtube.com/watch?v=zzzzzzzzzzz" }, ytTab);
      eq(requests("send")[0].url, "https://www.youtube.com/watch?v=zzzzzzzzzzz", "other video unchanged");
      await yt.close();
    });

    await step("YouTube: no timestamp while an ad is playing", async () => {
      const yt = await ctx.newPage();
      await yt.goto("https://www.youtube.com/watch?v=abcdefghijk&ad=1");
      await yt.waitForFunction(() => (document.querySelector("video")?.currentTime ?? 0) > 83, null, { timeout: 10_000 });
      const ytTab = await findTab("https://www.youtube.com/watch?v=abcdefghijk&ad=1");
      resetLog();
      await kin("sendTab", ytTab, PIXEL.id);
      eq(requests("send")[0].url, "https://www.youtube.com/watch?v=abcdefghijk&ad=1", "url unchanged");
      await yt.close();
    });

    await step("toolbar popup: Send this tab sends the active tab to the chosen phone and lists it", async () => {
      resetLog();
      const p = await ctx.newPage();
      await p.goto(popupUrl);
      await p.waitForFunction(() => document.getElementById("state")?.textContent === "Kin is running", null, { timeout: 10_000 });
      await p.locator("#devices input[value]").nth(1).check(); // Galaxy: offline
      await page.bringToFront();
      await p.locator("#send-tab").click({ force: true });
      await p.waitForFunction(() => /queued|Sent/i.test(document.getElementById("result")?.textContent ?? ""), null, { timeout: 15_000 });
      const sends = requests("send");
      eq(sends.length, 1, "one send");
      eq([sends[0].kind, sends[0].url, sends[0].deviceId], ["page", siteUrl + "/", GALAXY.id], "page request to Galaxy");
      check((await p.locator("#history li").count()) >= 1, "history shows the send");
      check((await p.locator("#history").innerText()).includes("Kin test page"), "history label is the title");
      await p.screenshot({ path: join(shotsDir, "popup-after-send.png"), clip: { x: 0, y: 0, width: 360, height: 460 } });
      await p.locator("#clear").click();
      await p.waitForFunction(() => document.querySelectorAll("#history li").length === 0, null, { timeout: 5_000 });
      await p.close();
    });

    await step("one phone: the menu is a single item", async () => {
      setScenario(scenarioOne);
      await kin("refreshStatus");
      await new Promise((r) => setTimeout(r, 300));
      eq((await kin("getMenu")).map((m: any) => m.title), ["Send to Kin"], "menu");
      eq(await kin("getMenuErrors"), [], "menu errors");
    });

    await step("Kin not running: the popup says so and Start Kin starts it", async () => {
      setScenario({ running: false, startable: true, devices: [] });
      const p = await ctx.newPage();
      await p.goto(popupUrl);
      await p.waitForFunction(() => document.getElementById("state")?.textContent === "Kin isn't running", null, { timeout: 10_000 });
      check(await p.locator("#start").isVisible(), "Start Kin button is visible");
      check((await p.locator("#problem-text").innerText()).includes("Start Kin"), "hint text");
      await p.screenshot({ path: join(shotsDir, "popup-not-running.png"), clip: { x: 0, y: 0, width: 360, height: 300 } });
      setScenario({ ...scenarioOne, running: false, startable: true });
      await p.locator("#start").click();
      await p.waitForFunction(() => document.getElementById("state")?.textContent === "Kin is running", null, { timeout: 10_000 });
      eq(requests("start").length >= 1, true, "host got start");
      await p.close();
    });

    await step("Kin can't be started: a send fails with a clear Start Kin message", async () => {
      setScenario({ running: false, startable: false, devices: [] });
      const out = await click({ menuItemId: "kin:send", selectionText: "x" }, tab);
      eq([out.outcome, out.code], ["failed", "KIN_NOT_RUNNING"], "outcome");
      check(/Kin/.test(out.message) && /running|started/.test(out.message), out.message);
    });

    await step("no phone paired: popup says so, Send this tab is disabled, a send fails with NOT_PAIRED", async () => {
      setScenario({ running: true, startable: true, devices: [] });
      const p = await ctx.newPage();
      await p.goto(popupUrl);
      await p.waitForFunction(() => document.getElementById("state")?.textContent === "No phone paired", null, { timeout: 10_000 });
      check(await p.locator("#send-tab").isDisabled(), "send button disabled");
      await p.close();
      const out = await click({ menuItemId: "kin:send", selectionText: "x" }, tab);
      eq([out.outcome, out.code], ["failed", "NOT_PAIRED"], "outcome");
    });

    await step("Kin not installed: HOST_NOT_INSTALLED with the install hint", async () => {
      await kin("setHostName", "app.kinrelay.missing");
      const view = await kin("refreshStatus");
      eq([view.ok, view.code], [false, "HOST_NOT_INSTALLED"], "view");
      check(/install/i.test(view.message), view.message);
      const p = await ctx.newPage();
      await p.goto(popupUrl);
      await p.waitForFunction(() => document.getElementById("state")?.textContent === "Kin unavailable", null, { timeout: 10_000 });
      check((await p.locator("#problem-text").innerText()).toLowerCase().includes("install"), "popup text");
      check(await p.locator("#send-tab").isDisabled(), "send disabled");
      await p.screenshot({ path: join(shotsDir, "popup-not-installed.png"), clip: { x: 0, y: 0, width: 360, height: 300 } });
      await p.close();
      await kin("setHostName", HOST);
    });

    await step("a host that exits without answering: HOST_EXITED", async () => {
      setScenario({ ...scenarioOne, exitWithoutAnswer: true });
      const out = await click({ menuItemId: "kin:send", selectionText: "x" }, tab);
      eq([out.outcome, out.code], ["failed", "HOST_EXITED"], "outcome");
    });

    await step("a response for another request is refused", async () => {
      setScenario({ ...scenarioOne, badRef: true });
      const out = await click({ menuItemId: "kin:send", selectionText: "x" }, tab);
      eq(out.outcome, "failed", "outcome");
    });

    await step("Kin's own error text reaches the user (TOO_LARGE)", async () => {
      setScenario({ ...scenarioOne, sendError: { code: "TOO_LARGE", message: "Kin says: too big." } });
      const out = await click({ menuItemId: "kin:send", selectionText: "x" }, tab);
      eq([out.outcome, out.code, out.message], ["failed", "TOO_LARGE", "Kin says: too big."], "outcome");
    });

    // ---- download alerts (2B.5) ---------------------------------------------------------------------
    // The test build holds `downloads` as a normal permission, so the real chrome.downloads events fire.
    // The downloads are started from the worker (Chrome blocks several automatic downloads from a page).

    const BURST_WAIT = 4_500; // the 3 s window plus the send
    const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
    const alerts = () => requests("send").filter((r) => r.kind === "alert");
    const waitFor = async (what: string, pred: () => boolean, ms = 15_000) => {
      const end = Date.now() + ms;
      while (!pred()) {
        if (Date.now() > end) throw new Error(`timed out waiting for ${what}`);
        await sleep(100);
      }
    };
    /** Starts a download from the worker and waits until the browser has finished with it. */
    const download = async (path: string, opts: { cancelAfterMs?: number } = {}) => {
      const w = await getSw();
      return (await w.evaluate(
        async ([u, cancelAfter]) => {
          const id = await chrome.downloads.download({ url: u as string, conflictAction: "uniquify" });
          if (cancelAfter) {
            await new Promise((r) => setTimeout(r, cancelAfter as number));
            await chrome.downloads.cancel(id);
          }
          for (let i = 0; i < 400; i++) {
            const [item] = await chrome.downloads.search({ id });
            if (item && item.state !== "in_progress") return { id, state: item.state, error: item.error, filename: item.filename, incognito: item.incognito };
            await new Promise((r) => setTimeout(r, 100));
          }
          return { id, state: "in_progress", filename: "", incognito: false };
        },
        [siteUrl + path, opts.cancelAfterMs ?? 0] as [string, number],
      )) as { id: number; state: string; error?: string; filename: string; incognito: boolean };
    };
    const baseOf = (p: string) => p.split(/[\\/]/).pop()!;
    const setAlerts = (patch: { enabled?: boolean; longOnly?: boolean }) => kin("handlePopup", { kin: "downloadAlerts", ...patch });

    let realName = "";
    await step("downloads: off by default, so a finished download sends nothing", async () => {
      setScenario(scenarioOne);
      await kin("refreshStatus");
      resetLog();
      const d = await download("/dl/report.pdf");
      eq(d.state, "complete", "download state");
      realName = baseOf(d.filename);
      await sleep(BURST_WAIT);
      eq(alerts().length, 0, "alerts");
      eq(requests("send").length, 0, "sends of any kind");
    });

    await step("downloads: the popup switch turns it on (screenshot), and the 30 s box starts on", async () => {
      const p = await ctx.newPage();
      await p.goto(popupUrl);
      await p.waitForFunction(() => document.getElementById("state")?.textContent === "Kin is running", null, { timeout: 10_000 });
      eq(await p.locator("#dl-enabled").isChecked(), false, "switch starts off");
      check(await p.locator("#dl-long").isDisabled(), "30 s box disabled while off");
      check((await p.locator("#dl-note").innerText()).startsWith("Off."), "note says off");
      await p.locator("#dl-enabled").check();
      await p.waitForFunction(() => document.getElementById("dl-note")?.textContent?.startsWith("Kin sends only the file name"), null, { timeout: 10_000 });
      eq(await p.locator("#dl-enabled").isChecked(), true, "switch on");
      eq(await p.locator("#dl-long").isChecked(), true, "30 s box starts on");
      check(await p.locator("#dl-long").isEnabled(), "30 s box enabled");
      eq((await kin("handlePopup", { kin: "state" })).downloadAlerts, { enabled: true, longOnly: true, granted: true, supported: true }, "state");
      eq(await kin("downloadsRegistered"), true, "listener registered");
      await p.screenshot({ path: join(shots5Dir, "popup-download-alerts-on.png"), clip: { x: 0, y: 0, width: 360, height: 520 } });
      await p.close();
    });

    await step("downloads: a quick download and a cancelled one send nothing while the 30 s rule is on", async () => {
      resetLog();
      const fast = await download("/dl/quick.txt");
      eq(fast.state, "complete", "fast state");
      const cancelled = await download("/dl/hang.bin", { cancelAfterMs: 800 });
      eq([cancelled.state, cancelled.error], ["interrupted", "USER_CANCELED"], "cancelled");
      await sleep(BURST_WAIT);
      eq(alerts().length, 0, "alerts");
    });

    await step("downloads: a real download of over 30 s sends one alert", async () => {
      resetLog();
      const t0 = Date.now();
      const d = await download("/dl/slow.bin");
      check(Date.now() - t0 >= 30_000, `the download took ${Date.now() - t0} ms`);
      eq(d.state, "complete", "state");
      await waitFor("the alert", () => alerts().length >= 1);
      await sleep(500);
      const a = alerts();
      eq(a.length, 1, "alerts");
      eq({ kind: a[0].kind, title: a[0].title, text: a[0].text, source: a[0].source, auto: a[0].auto, ttlSec: a[0].ttlSec }, { kind: "alert", title: "Download finished", text: baseOf(d.filename), source: "downloads", auto: true, ttlSec: 1800 }, "alert");
    });

    await step("downloads: with the 30 s rule off, one download gives one alert: base name only, no address, no folder", async () => {
      await setAlerts({ longOnly: false });
      resetLog();
      const d = await download("/dl/report.pdf");
      await waitFor("the alert", () => alerts().length >= 1);
      await sleep(500);
      const a = alerts();
      eq(a.length, 1, "alerts");
      eq([a[0].title, a[0].text, a[0].source, a[0].auto, a[0].ttlSec, a[0].deviceId], ["Download finished", baseOf(d.filename), "downloads", true, 1800, PIXEL.id], "alert");
      // Everything the host was sent for this download, as one string.
      const wire = JSON.stringify(readLog().filter((e) => e.req && e.req.kind === "alert"));
      for (const leak of ["127.0.0.1", "localhost", "/dl/", "kin-ext-smoke", "ms-playwright", "Users", "http"]) check(!wire.includes(leak), `the alert leaked ${leak}: ${wire}`);
      check(!/[A-Za-z]:\\\\/.test(wire), `the alert holds a drive path: ${wire}`);
      check(!/[\\/]/.test(a[0].text), `the text has a path separator: ${a[0].text}`);
      console.log(`        (the browser named the file "${baseOf(d.filename)}"; the server offered "report.pdf"; first download was "${realName}")`);
    });

    await step("downloads: the history lists 'Download alert' with the time and no file name (screenshot)", async () => {
      const p = await ctx.newPage();
      await p.goto(popupUrl);
      await p.waitForFunction(() => document.getElementById("state")?.textContent === "Kin is running", null, { timeout: 10_000 });
      const text = await p.locator("#history").innerText();
      check(text.includes("Download alert") && text.includes("sent"), `history: ${text}`);
      check(!text.includes("report") && !text.includes("slow-render"), `history shows a file name: ${text}`);
      await p.screenshot({ path: join(shots5Dir, "popup-download-alert-history.png"), clip: { x: 0, y: 0, width: 360, height: 520 } });
      await p.close();
    });

    await step("downloads: 3 downloads within 3 s give one alert, '3 downloads finished'", async () => {
      resetLog();
      await Promise.all([download("/dl/a.txt"), download("/dl/b.txt"), download("/dl/c.txt")]);
      await waitFor("the alert", () => alerts().length >= 1);
      await sleep(BURST_WAIT);
      const a = alerts();
      eq(a.length, 1, "alerts");
      eq([a[0].title, a[0].text], ["Downloads finished", "3 downloads finished"], "alert");
    });

    await step("downloads: an interrupted download gives 'Download failed' (30 s rule off)", async () => {
      resetLog();
      const d = await download("/dl/broken.bin");
      eq(d.state, "interrupted", `download state (error ${d.error})`);
      await waitFor("the alert", () => alerts().length >= 1);
      await sleep(500);
      const a = alerts();
      eq(a.length, 1, "alerts");
      eq([a[0].title, a[0].text], ["Download failed", baseOf(d.filename)], "alert");
    });

    // Chromium gives an interrupted download no end time, so the 30 s rule must use the time of the event.
    await step("downloads: with the 30 s rule ON, a quick interrupted download gives none and one that failed after 30 s gives 'Download failed'", async () => {
      await setAlerts({ longOnly: true });
      resetLog();
      const quick = await download("/dl/broken.bin");
      eq(quick.state, "interrupted", `quick download state (error ${quick.error})`);
      await sleep(BURST_WAIT);
      eq(alerts().length, 0, "alerts after the quick failure");
      const t0 = Date.now();
      const d = await download("/dl/broken-slow.bin");
      check(Date.now() - t0 >= 30_000, `the download took ${Date.now() - t0} ms`);
      eq(d.state, "interrupted", `slow download state (error ${d.error})`);
      await waitFor("the alert", () => alerts().length >= 1);
      await sleep(500);
      const a = alerts();
      eq(a.length, 1, "alerts");
      eq([a[0].title, a[0].text], ["Download failed", baseOf(d.filename)], "alert");
      await setAlerts({ longOnly: false }); // the next steps use small, quick downloads
    });

    await step("downloads: an offline phone queues the alert and the popup says so", async () => {
      setScenario({ ...scenarioOne, devices: [{ ...PIXEL, online: false }] });
      await kin("refreshStatus");
      resetLog();
      await download("/dl/offline.txt");
      await waitFor("the alert", () => alerts().length >= 1);
      await sleep(500);
      const list = (await kin("handlePopup", { kin: "state" })).history;
      eq([list[0].kind, list[0].label, list[0].outcome], ["download", "Download alert", "queued"], "latest history entry");
    });

    await step("downloads: Kin paused answers PAUSED_SKIPPED: the alert was asked for, no error, nothing recorded", async () => {
      setScenario({ ...scenarioOne, paused: true });
      await kin("refreshStatus");
      const before = (await kin("handlePopup", { kin: "state" })).history.length;
      const beforeResult = (await kin("handlePopup", { kin: "state" })).lastResult;
      resetLog();
      await download("/dl/paused.txt");
      await waitFor("the alert request", () => alerts().length >= 1);
      await sleep(500);
      eq(alerts().length, 1, "alert requests");
      const answered = readLog().find((e) => e.res && e.res.code === "PAUSED_SKIPPED");
      check(answered, "the host answered PAUSED_SKIPPED");
      const st = await kin("handlePopup", { kin: "state" });
      eq(st.history.length, before, "history unchanged");
      eq(st.lastResult, beforeResult, "no new last result");
    });

    await step("downloads: Kin not running: nothing is sent and Kin is not started", async () => {
      setScenario({ running: false, startable: true, devices: [] });
      resetLog();
      await download("/dl/closed.txt");
      await sleep(BURST_WAIT);
      eq(requests("send").length, 0, "sends");
      eq(requests("start").length, 0, "starts");
      eq(requests("status").length >= 1, true, "it asked for the status");
      eq(JSON.parse(readFileSync(join(state, "scenario.json"), "utf8")).running, false, "Kin still not running");
    });

    await step("downloads: switch off in the popup: no alert for the next download", async () => {
      setScenario(scenarioOne);
      await kin("refreshStatus");
      const p = await ctx.newPage();
      await p.goto(popupUrl);
      await p.waitForFunction(() => document.getElementById("state")?.textContent === "Kin is running", null, { timeout: 10_000 });
      await p.locator("#dl-enabled").uncheck();
      await p.waitForFunction(() => document.getElementById("dl-note")?.textContent?.startsWith("Off."), null, { timeout: 10_000 });
      check(await p.locator("#dl-long").isDisabled(), "30 s box disabled again");
      await p.close();
      eq((await kin("handlePopup", { kin: "state" })).downloadAlerts.enabled, false, "setting");
      resetLog();
      await download("/dl/after-off.txt");
      await sleep(BURST_WAIT);
      eq(alerts().length, 0, "alerts");
    });

    await page.close();
    await ctx.close();
    context = undefined;
  } finally {
    if (context) await context.close().catch(() => {});
    site.stop(true);
    unregister();
    const left = regQuery();
    if (left !== null) console.error(`WARNING: ${REG_KEY} is still there!`);
    rmSync(work, { recursive: true, force: true });
  }

  const failed = results.filter((r) => !r.ok);
  console.log(`\n${results.length - failed.length}/${results.length} steps passed; registry key ${regQuery() === null ? "removed" : "STILL PRESENT"}`);
  return failed.length === 0 && regQuery() === null ? 0 : 1;
}

main().then(
  (code) => process.exit(code),
  (e) => {
    console.error(e);
    unregister();
    process.exit(1);
  },
);
