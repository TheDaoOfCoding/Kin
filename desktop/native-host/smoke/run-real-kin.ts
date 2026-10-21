// Headless Chromium smoke test of the extension against the REAL Kin (a dev instance beside the installed one) and the REAL dev host,
// with a fake phone (desktop/src-tauri/examples/fake_phone.rs) on the other side. Started by smoke/run-real-kin.ps1, which builds everything,
// takes the scripts' lock and checks that no game runs.
//
// Safety, by construction:
//  - headless only, a throwaway --user-data-dir under the OS temp folder, the Playwright Chromium (never the user's browser or profile)
//  - the only registry key written is HKCU\Software\Chromium\NativeMessagingHosts\app.kinrelay.dev (the DEV host name), pointing at the dev
//    host exe; it is removed in `finally`, on a signal and on exit. The release keys (app.kinrelay.desktop) are never touched.
//  - the Kin is a dev instance: its own throwaway --data-dir, --dev-bind 127.0.0.1, --dev-no-windows (no window, toasts only logged),
//    --dev-no-open (it opens nothing: no browser, no Explorer), and a pipe of its own (app.kinrelay.dev.<SID>.<suffix>), so it can never be
//    reached from, or reach, the user's installed Kin. The installed Kin is never started, stopped or asked.
//  - only processes this script started are killed, by PID (and image name).
//  - the host can't start Kin here (no KIN_HOST_DEV_KIN_EXE): "Start Kin" is answered KIN_NOT_RUNNING, which is what is checked.
import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium, type BrowserContext, type Worker } from "../../../extension/node_modules/playwright-core";

const here = resolve(dirname(fileURLToPath(import.meta.url)));
const hostRoot = resolve(here, "..");
const repo = resolve(hostRoot, "../..");
const extRoot = join(repo, "extension");
const extDir = join(extRoot, "dist", "chromium-test");
const HOST_EXE = join(hostRoot, "target", "dev", "debug", "kin-native-host.exe");
const KIN_EXE = process.env.KIN_SMOKE_EXE ?? join(repo, "desktop", "src-tauri", "target", "debug", "Kin.exe");
const PHONE_EXE = join(repo, "desktop", "src-tauri", "target", "debug", "examples", "fake_phone.exe");
const EXPECTED_ID = "cbioefpddjpaionhljhffgbedncfchmn";
const HOST = "app.kinrelay.dev";
const REG_KEY = `HKCU\\Software\\Chromium\\NativeMessagingHosts\\${HOST}`;

// ---- small test harness -----------------------------------------------------------------------

const results: { name: string; ok: boolean }[] = [];
async function step(name: string, fn: () => Promise<void>): Promise<void> {
  try {
    await fn();
    results.push({ name, ok: true });
    console.log(`  ok    ${name}`);
  } catch (e) {
    results.push({ name, ok: false });
    console.log(`  FAIL  ${name}\n        ${(e instanceof Error ? e.message : String(e)).split("\n").join("\n        ")}`);
  }
}
function check(cond: unknown, msg: string): asserts cond {
  if (!cond) throw new Error(msg);
}
function eq<T>(actual: T, expected: T, what: string): void {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) throw new Error(`${what}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
}
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
async function until(what: string, fn: () => boolean | Promise<boolean>, ms = 15_000): Promise<void> {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    if (await fn()) return;
    await sleep(100);
  }
  throw new Error(`timed out waiting for ${what}`);
}
const sha = (b: Buffer) => createHash("sha256").update(b).digest("hex");

// ---- registry (HKCU, dev host name only) -------------------------------------------------------

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

// ---- test site ---------------------------------------------------------------------------------

const PNG = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==", "base64");
const BIG = Buffer.concat([PNG, Buffer.alloc(5 * 1024 * 1024, 0x5a)]);

function startSite() {
  return Bun.serve({
    hostname: "127.0.0.1",
    port: 0,
    fetch(req) {
      const url = new URL(req.url);
      const authed = (req.headers.get("cookie") ?? "").includes("auth=1");
      switch (url.pathname) {
        case "/":
          return new Response(`<!doctype html><title>Kin test page</title><h1>Hello</h1>`, { headers: { "content-type": "text/html" } });
        case "/img/ok.png":
          return new Response(PNG, { headers: { "content-type": "image/png" } });
        case "/img/secret.png":
          return authed ? new Response(PNG, { headers: { "content-type": "image/png" } }) : new Response("sign in", { status: 403 });
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

// ---- main --------------------------------------------------------------------------------------

async function main(): Promise<number> {
  for (const f of [join(extDir, "manifest.json"), HOST_EXE, KIN_EXE, PHONE_EXE]) check(existsSync(f), `missing ${f} (run smoke/run-real-kin.ps1, it builds them)`);
  if (regQuery()) throw new Error(`${REG_KEY} already exists. Not touching it: remove it by hand if it is a leftover.`);
  const browser = findChromium();
  console.log(`browser: ${browser}\nKin: ${KIN_EXE}`);

  const work = join(tmpdir(), `kin-real-smoke-${process.pid}-${Date.now().toString(36)}`);
  const profile = join(work, "profile");
  const data = join(work, "kin-data");
  const phoneDir = join(work, "phone");
  for (const d of [profile, data, phoneDir]) mkdirSync(d, { recursive: true });
  const sid = spawnSync("powershell", ["-NoProfile", "-Command", "[Security.Principal.WindowsIdentity]::GetCurrent().User.Value"], { encoding: "utf8" }).stdout.trim();
  check(/^S-1-5-/.test(sid), "could not read the user SID");
  const suffix = `smoke${process.pid}`;
  const PIPE = `\\\\.\\pipe\\${HOST}.${sid}.${suffix}`;
  const manifestPath = join(work, `${HOST}.json`);
  writeFileSync(manifestPath, JSON.stringify({ name: HOST, description: "Kin real-Kin smoke test", path: HOST_EXE, type: "stdio", allowed_origins: [`chrome-extension://${EXPECTED_ID}/`] }, null, 2));

  const site = startSite();
  const siteUrl = `http://127.0.0.1:${site.port}`;
  let context: BrowserContext | undefined;
  let kinProc: ChildProcess | undefined;
  let phoneProc: ChildProcess | undefined;
  let phoneRuns = 0;
  let pairUrl = "";

  // ---- the dev Kin ------------------------------------------------------------------------------------------------------------
  const kinArgs = ["--data-dir", data, "--dev-bind", "127.0.0.1", "--dev-qr-address", "10.0.2.2", "--dev-no-windows", "--dev-no-open", "--dev-print-pair-url", "--dev-received-dir", join(data, "received")];
  // 2B.6: this Kin never opens the user's real clipboard (--dev-clipboard-sink through the environment; it is removed with `work`).
  const kinEnv = { ...(process.env as Record<string, string>), KIN_DEV_PIPE_SUFFIX: suffix, KIN_DEV_CLIPBOARD_SINK: join(work, "kin-clipboard-sink") };
  const kinCmd = (...extra: string[]) => spawnSync(KIN_EXE, ["--data-dir", data, ...extra], { encoding: "utf8", env: kinEnv, windowsHide: true, timeout: 20_000 });
  const kinLog = () => {
    const f = join(data, "logs", "kin.log");
    return existsSync(f) ? readFileSync(f, "utf8") : "";
  };
  const kinUp = () => !!kinProc && kinProc.exitCode === null && existsSync(join(data, "status.json"));
  const startKin = async () => {
    kinProc = spawn(KIN_EXE, kinArgs, { env: kinEnv, windowsHide: true, stdio: "ignore" });
    await until("Kin's status.json", () => existsSync(join(data, "status.json")), 20_000);
    await until("the extension pipe", () => kinLog().includes("extension pipe: listening"), 20_000);
  };
  const quitKin = async () => {
    kinCmd("--quit");
    await until("Kin to quit", () => kinProc?.exitCode !== null, 15_000).catch(() => {});
    if (kinProc && kinProc.exitCode === null && kinProc.pid) spawnSync("taskkill", ["/PID", String(kinProc.pid), "/F", "/FI", "IMAGENAME eq Kin.exe"], { encoding: "utf8" });
  };

  // ---- the fake phone ----------------------------------------------------------------------------------------------------------
  const phoneKey = join(work, "phone.key");
  let phoneLogFile = "";
  const phoneLog = () => (phoneLogFile && existsSync(phoneLogFile) ? readFileSync(phoneLogFile, "utf8") : "");
  const stopPhone = async () => {
    if (phoneProc && phoneProc.exitCode === null && phoneProc.pid) spawnSync("taskkill", ["/PID", String(phoneProc.pid), "/F", "/FI", "IMAGENAME eq fake_phone.exe"], { encoding: "utf8" });
    phoneProc = undefined;
    await sleep(500);
  };
  const startPhone = async (...extra: string[]) => {
    await stopPhone();
    const paired = phoneRuns > 0;
    phoneRuns++;
    phoneLogFile = join(work, `phone-${phoneRuns}.log`);
    // (A later run is the same phone with the same key, with the same URL: it only pins the PC and finds the port; the token in it is spent.)
    if (!paired) {
      kinCmd("--dev-start-pairing");
      await until("the pairing URL", () => existsSync(join(data, "pair-url.txt")), 10_000);
      pairUrl = readFileSync(join(data, "pair-url.txt"), "utf8").trim();
    }
    const url = pairUrl;
    const args = [url, "--host", "127.0.0.1", "--seconds", "900", "--save-dir", phoneDir, "--identity-file", phoneKey, ...(paired ? ["--skip-pairing"] : []), ...extra];
    const out = Bun.file(phoneLogFile);
    writeFileSync(phoneLogFile, "");
    phoneProc = spawn(PHONE_EXE, args, { windowsHide: true, stdio: ["ignore", "pipe", "pipe"] });
    const append = (d: Buffer) => writeFileSync(phoneLogFile, phoneLog() + d.toString("utf8"));
    phoneProc.stdout?.on("data", append);
    phoneProc.stderr?.on("data", append);
    void out;
    await until("the fake phone to connect", () => phoneLog().includes("connected, hello sent"), 15_000);
    await until("Kin to see the phone", () => statusOf().state === "phone_connected", 10_000);
  };
  const statusOf = (): any => {
    try {
      return JSON.parse(readFileSync(join(data, "status.json"), "utf8"));
    } catch {
      return {};
    }
  };
  const waitPhone = (re: RegExp, ms = 15_000) => until(`the phone to log ${re}`, () => re.test(phoneLog()), ms);
  const phoneLines = (re: RegExp) => phoneLog().split(/\r?\n/).filter((l) => re.test(l));

  try {
    const add = spawnSync("reg", ["add", REG_KEY, "/ve", "/t", "REG_SZ", "/d", manifestPath, "/f"], { encoding: "utf8" });
    check(add.status === 0, `reg add failed: ${add.stderr}`);
    registered = true;

    await startKin();
    await startPhone();

    // The dev host, started by the browser: its pipe is the dev pipe of this Kin; it can't start a Kin (none is named).
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

    const page = await ctx.newPage();
    await page.goto(siteUrl + "/");
    const tab = await findTab(siteUrl);
    let deviceId = "";

    await step("extension loads and Kin answers status through the real host and the real pipe", async () => {
      eq(new URL((await getSw()).url()).host, EXPECTED_ID, "extension id");
      const view = await kin("refreshStatus");
      eq(view.ok, true, `view.ok (${JSON.stringify(view)})`);
      eq(view.status.devices.length, 1, "one device");
      eq([view.status.devices[0].name, view.status.devices[0].online], ["Fake Phone", true], "the fake phone is online");
      deviceId = view.status.devices[0].id;
      eq(view.status.defaultDeviceId, deviceId, "default device");
      check(typeof view.status.kinVersion === "string", "kinVersion");
    });

    await step("popup shows Kin running and the phone online", async () => {
      const p = await ctx.newPage();
      await p.goto(popupUrl);
      await p.waitForFunction(() => document.getElementById("state")?.textContent === "Kin is running", null, { timeout: 10_000 });
      check((await p.locator("#devices").innerText()).includes("Fake Phone"), "device row");
      await p.close();
    });

    await step("a link reaches the phone as a link message: url exactly as it is, title cleaned, sourceApp Browser, acked", async () => {
      const out = await click({ menuItemId: `kin:dev:${deviceId}`, linkUrl: `${siteUrl}/other?a=1&t=5#frag`, linkText: "  an \n interesting  link ", pageUrl: siteUrl + "/" }, tab);
      eq(out.outcome, "sent", `outcome (${out.message})`);
      await waitPhone(/received link /);
      const line = phoneLines(/received link /)[0];
      check(line.includes(`url=${siteUrl}/other?a=1&t=5#frag title=an interesting link sourceApp=Browser`), line);
    });

    await step("selected text reaches the phone as a text", async () => {
      const out = await click({ menuItemId: `kin:dev:${deviceId}`, selectionText: "hello from the browser", pageUrl: siteUrl + "/" }, tab);
      eq(out.outcome, "sent", "outcome");
      await waitPhone(/received text .*hello from the browser/);
      check(/sensitive=false/.test(phoneLines(/received text/)[0]), "never sensitive");
    });

    await step("the page goes as a link with its title", async () => {
      const out = await click({ menuItemId: `kin:dev:${deviceId}`, pageUrl: siteUrl + "/" }, tab);
      eq(out.outcome, "sent", "outcome");
      await waitPhone(new RegExp(`received link .* url=${siteUrl}/ title=Kin test page sourceApp=Browser`));
    });

    await step("a bad link is refused before it reaches the host, and a bad one sent straight to Kin is refused by Kin", async () => {
      const before = phoneLines(/received link /).length;
      for (const bad of ["javascript:alert(1)", "file:///C:/Windows/win.ini", "ftp://example.com/x"]) {
        const out = await click({ menuItemId: `kin:dev:${deviceId}`, linkUrl: bad }, tab);
        eq([out.outcome, out.code], ["failed", "BAD_URL"], `outcome for ${bad}`);
      }
      // Straight over the pipe, bypassing the extension: Kin checks again.
      const sw = await getSw();
      const answer = await sw.evaluate(
        (h) => new Promise<any>((res) => chrome.runtime.sendNativeMessage(h, { v: 1, id: "raw-1", type: "send", kind: "link", url: "javascript:alert(1)" }, (r) => res(r))),
        HOST,
      );
      eq([answer?.ok, answer?.code, answer?.ref], [false, "BAD_URL", "raw-1"], "Kin refuses by itself");
      await sleep(500);
      eq(phoneLines(/received link /).length, before, "nothing more reached the phone");
    });

    const imageFile = (name: string) => join(phoneDir, name);
    await step("image: Kin downloads a public image itself and the phone gets the file", async () => {
      const out = await click({ menuItemId: `kin:dev:${deviceId}`, srcUrl: `${siteUrl}/img/ok.png`, mediaType: "image", pageUrl: siteUrl + "/", frameId: 0 }, tab);
      eq(out.outcome, "sent", `outcome (${out.message})`);
      await waitPhone(new RegExp(`downloaded ok.png bytes=${PNG.length} sha256=${sha(PNG)}`));
      check(existsSync(imageFile("ok.png")), "file in the phone's folder");
    });

    await step("image: a sign-in image is refused to Kin (403), fetched inside the page, and arrives as bytes", async () => {
      const out = await click({ menuItemId: `kin:dev:${deviceId}`, srcUrl: `${siteUrl}/img/secret.png`, mediaType: "image", pageUrl: siteUrl + "/", frameId: 0 }, tab);
      eq(out.outcome, "sent", `outcome (${out.message})`);
      await waitPhone(new RegExp(`downloaded secret.png bytes=${PNG.length} sha256=${sha(PNG)}`));
    });

    await step("image: 5 MiB behind a sign-in goes through as one big message and arrives intact", async () => {
      const out = await click({ menuItemId: `kin:dev:${deviceId}`, srcUrl: `${siteUrl}/img/big.png`, mediaType: "image", pageUrl: siteUrl + "/", frameId: 0 }, tab);
      eq(out.outcome, "sent", `outcome (${out.message})`);
      await waitPhone(new RegExp(`downloaded big.png bytes=${BIG.length} sha256=${sha(BIG)}`), 30_000);
    });

    await step("image: a data: URL is sent as bytes", async () => {
      const out = await click({ menuItemId: `kin:dev:${deviceId}`, srcUrl: "data:image/png;base64," + PNG.toString("base64"), mediaType: "image", frameId: 0 }, tab);
      eq(out.outcome, "sent", `outcome (${out.message})`);
      await waitPhone(new RegExp(`downloaded .*png bytes=${PNG.length} sha256=${sha(PNG)}`));
    });

    await step("image: when neither Kin nor the page can get it, the user gets a clear failure", async () => {
      const out = await click({ menuItemId: `kin:dev:${deviceId}`, srcUrl: `${siteUrl}/img/missing.png`, mediaType: "image", pageUrl: siteUrl + "/", frameId: 0 }, tab);
      eq([out.outcome, out.code], ["failed", "IMAGE_UNREACHABLE"], "outcome");
    });

    await step("YouTube: the time is read at click time and the phone gets t=83s", async () => {
      const yt = await ctx.newPage();
      await yt.goto("https://www.youtube.com/watch?v=abcdefghijk&t=5s&list=PLx");
      await yt.waitForFunction(() => (document.querySelector("video")?.currentTime ?? 0) > 83, null, { timeout: 10_000 });
      const ytTab = await findTab("https://www.youtube.com/");
      const out = await kin("sendTab", ytTab, deviceId);
      eq(out.outcome, "sent", "outcome");
      await waitPhone(/received link .* url=https:\/\/www\.youtube\.com\/watch\?v=abcdefghijk&list=PLx&t=83s /);
      await yt.close();
    });

    await step("toolbar popup: Send this tab", async () => {
      const p = await ctx.newPage();
      await p.goto(popupUrl);
      await p.waitForFunction(() => document.getElementById("state")?.textContent === "Kin is running", null, { timeout: 10_000 });
      await page.bringToFront();
      await p.locator("#send-tab").click({ force: true });
      await p.waitForFunction(() => /Sent|queued/i.test(document.getElementById("result")?.textContent ?? ""), null, { timeout: 15_000 });
      check((await p.locator("#history li").count()) >= 1, "history shows the send");
      await p.close();
    });

    await step("a text over 128 KiB reaches the phone as a .txt file", async () => {
      const big = "x".repeat(200 * 1024);
      const out = await click({ menuItemId: `kin:dev:${deviceId}`, selectionText: big, pageUrl: siteUrl + "/" }, tab);
      eq(out.outcome, "sent", `outcome (${out.message})`);
      await waitPhone(new RegExp(`downloaded Text.txt bytes=${big.length} sha256=${sha(Buffer.from(big))}`));
    });

    await step("the phone is away: the send is queued, the popup says offline, and it arrives when the phone is back", async () => {
      await stopPhone();
      await until("Kin to see the phone gone", () => statusOf().state === "phone_offline", 15_000);
      const view = await kin("refreshStatus");
      eq(view.status.devices[0].online, false, "device offline");
      const out = await click({ menuItemId: `kin:dev:${deviceId}`, linkUrl: `${siteUrl}/queued-while-away`, linkText: "queued" }, tab);
      eq(out.outcome, "queued", `outcome (${out.message})`);
      check(/offline/.test(out.message), out.message);
      eq((await kin("refreshStatus")).status.devices[0].queuedCount, 1, "queuedCount");
      await startPhone();
      await waitPhone(new RegExp(`received link .* url=${siteUrl}/queued-while-away title=queued sourceApp=Browser`));
    });

    await step("an old phone (no capabilities) gets the same link as a text, with isUrl set", async () => {
      await startPhone("--old-phone");
      const out = await click({ menuItemId: `kin:dev:${deviceId}`, linkUrl: `${siteUrl}/for-an-old-phone`, linkText: "title is dropped" }, tab);
      eq(out.outcome, "sent", `outcome (${out.message})`);
      await waitPhone(new RegExp(`received text .* isUrl=true sensitive=false text=${siteUrl}/for-an-old-phone`));
      eq(phoneLines(/received link /).length, 0, "no link message to an old phone");
    });

    await step("a link from the phone: opened (--dev-no-open: only logged), then with the setting off only offered; the URL is never in Kin's log", async () => {
      await startPhone("--send-link", "https://phone-secret.example/page?token=PHONELINK", "--send-link-title", "PHONETITLE");
      await until("Kin to handle the link", () => /link fake-link-1 from device .*opened in the default browser/.test(kinLog()), 15_000);
      check(kinLog().includes("would have been opened in the browser"), "the dev opener was used, nothing was opened");
      const set = kinCmd("--dev-set", "openLinksAutomatically", "false");
      eq(set.status, 0, "dev-set");
      await until("the setting to be saved", () => existsSync(join(data, "dev-set-result.json")) && /"ok":true/.test(readFileSync(join(data, "dev-set-result.json"), "utf8")), 10_000);
      // (The first link of this run has the id of the first run's: a repeat, acked and not shown again. The second one is new.)
      await startPhone("--send-link", "https://dup.example/", "--send-link", "https://phone-secret.example/second?token=PHONELINK2");
      await until("Kin to offer the second link", () => /link fake-link-2 from device .*not opened \(the setting is off/.test(kinLog()), 15_000);
      const log = kinLog();
      for (const secret of ["PHONELINK", "PHONETITLE", "phone-secret"]) check(!log.includes(secret), `${secret} is in Kin's log`);
      rmSync(join(data, "view-history.json"), { force: true });
      kinCmd("--dev-dump-view", "history");
      await until("the history view", () => existsSync(join(data, "view-history.json")), 10_000);
      const history = JSON.parse(readFileSync(join(data, "view-history.json"), "utf8"));
      check(history.items.some((i: any) => i.isLink && i.direction === "in" && i.title === "PHONETITLE"), "the history has the link with its title");
    });

    await step("Kin quits: the popup says Kin isn't running, a send says so, Start Kin can't start it here", async () => {
      await stopPhone();
      await quitKin();
      const p = await ctx.newPage();
      await p.goto(popupUrl);
      await p.waitForFunction(() => document.getElementById("state")?.textContent === "Kin isn't running", null, { timeout: 10_000 });
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
    await stopPhone().catch(() => {});
    if (kinProc && kinProc.exitCode === null) await quitKin().catch(() => {});
    if (regQuery()) console.error(`WARNING: ${REG_KEY} is still there!`);
    await sleep(500);
    rmSync(work, { recursive: true, force: true });
  }

  const failed = results.filter((r) => !r.ok);
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
