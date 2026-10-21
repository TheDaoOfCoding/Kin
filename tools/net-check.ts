// Phase 0, step 5: can the phone reach this PC over Wi-Fi?
//
// Serves one line of plain text on port 47821 (all interfaces), prints a URL for each LAN address,
// and logs every request. Stops on Ctrl+C or after --minutes N (default 10).
//
//   bun tools/net-check.ts
//   bun tools/net-check.ts --minutes 30
//
// Usage notes are in tools/README.md.
import { hostname, networkInterfaces } from "node:os";
import type { NetworkInterfaceInfo } from "node:os";

const PORT = 47821;
const DEFAULT_MINUTES = 10;

// Adapters a phone on the home Wi-Fi can't reach: Hyper-V/WSL, VirtualBox, VMware, Docker, VPN overlays.
const VIRTUAL_NAME =
  /vethernet|hyper-v|wsl|virtualbox|vbox|vmware|vmnet|docker|virbr|tailscale|zerotier|hamachi|npcap|bluetooth|loopback/i;

// Anything from a request that gets logged: control characters (escape codes, newlines) must not reach the terminal.
const CONTROL_CHARS = /[\u0000-\u001f\u007f-\u009f\u2028\u2029]/g;
export const cleanForLog = (s: string): string => s.replace(CONTROL_CHARS, "?");

export interface Candidate {
  name: string;
  address: string;
  skipped?: string; // why it was skipped; absent means usable
}

// Pure, so it can be checked without listening on anything.
// `routeAddress` is the address on the default route. It is never skipped for a virtual-looking adapter name,
// because a Hyper-V external switch ("vEthernet (...)") can be the real LAN.
export function listAddresses(
  interfaces: NodeJS.Dict<NetworkInterfaceInfo[]> = networkInterfaces(),
  routeAddress: string | null = null,
): Candidate[] {
  const out: Candidate[] = [];
  for (const [name, infos] of Object.entries(interfaces)) {
    for (const info of infos ?? []) {
      if (String(info.family) !== "IPv4" && String(info.family) !== "4") continue;
      const c: Candidate = { name, address: info.address };
      if (info.internal || info.address.startsWith("127.")) c.skipped = "loopback";
      else if (info.address.startsWith("169.254.")) c.skipped = "link-local";
      else if (VIRTUAL_NAME.test(name) && info.address !== routeAddress) c.skipped = "virtual adapter";
      out.push(c);
    }
  }
  return out;
}

// The local address of the lowest-metric default route, or null if we can't tell.
// Windows only: `route print` is local, sends nothing and opens no sockets.
export function defaultRouteAddress(): string | null {
  if (process.platform !== "win32") return null;
  try {
    const r = Bun.spawnSync(["route", "print", "-4", "0.0.0.0"]);
    let best: { address: string; metric: number } | null = null;
    // Data lines look like: 0.0.0.0  0.0.0.0  192.168.0.1  192.168.0.101  25
    for (const line of r.stdout.toString().split(/\r?\n/)) {
      const m = line.match(/^\s*0\.0\.0\.0\s+0\.0\.0\.0\s+\S+\s+(\d+\.\d+\.\d+\.\d+)\s+(\d+)\s*$/);
      if (m && (!best || Number(m[2]) < best.metric)) best = { address: m[1], metric: Number(m[2]) };
    }
    return best?.address ?? null;
  } catch {
    return null;
  }
}

function parseMinutes(args: string[]): number {
  let raw: string | undefined;
  for (let i = 0; i < args.length; i++) {
    const a = args[i];
    if (a === "-h" || a === "--help") {
      console.log("Usage: bun tools/net-check.ts [--minutes N]\nServes on port 47821 until Ctrl+C or N minutes (default 10).");
      process.exit(0);
    } else if (a === "--minutes") raw = args[++i];
    else if (a.startsWith("--minutes=")) raw = a.slice("--minutes=".length);
    else {
      console.error(`Unknown argument: ${a}\nUsage: bun tools/net-check.ts [--minutes N]`);
      process.exit(2);
    }
  }
  if (raw === undefined) return DEFAULT_MINUTES;
  const n = Number(raw);
  if (!Number.isFinite(n) || n <= 0) {
    console.error(`--minutes needs a number above 0, got "${raw}".`);
    process.exit(2);
  }
  return n;
}

const clock = () => new Date().toLocaleTimeString("en-GB"); // HH:MM:SS, local time

function main() {
  const minutes = parseMinutes(process.argv.slice(2));
  const host = hostname();
  let requests = 0;

  let server: ReturnType<typeof Bun.serve>;
  try {
    server = Bun.serve({
      hostname: "0.0.0.0",
      port: PORT,
      fetch(req, srv) {
        requests++;
        const remote = srv.requestIP(req)?.address.replace(/^::ffff:/, "") ?? "unknown";
        const ua = cleanForLog((req.headers.get("user-agent") ?? "").slice(0, 70));
        const path = cleanForLog(new URL(req.url).pathname.slice(0, 200));
        console.log(`[${clock()}] ${remote}  ${cleanForLog(req.method)} ${path}  ${ua}`);
        return new Response(`Kin Relay network check OK\nHost: ${host}\n`, {
          headers: { "content-type": "text/plain; charset=utf-8" },
        });
      },
    });
  } catch (err) {
    if ((err as { code?: string }).code === "EADDRINUSE") {
      console.error(`Port ${PORT} is already in use. Is another net-check still running?`);
    } else {
      console.error(`Could not start the server: ${err}`);
    }
    process.exit(1);
  }

  const route = defaultRouteAddress();
  const all = listAddresses(networkInterfaces(), route);
  const usable = all.filter((c) => !c.skipped);
  const skipped = all.filter((c) => c.skipped);
  const likely = usable.length > 1 ? route : null; // with one address there is nothing to pick

  console.log(`Kin Relay network check, on ${host}`);
  console.log(`Listening on port ${PORT}, all interfaces. Stops after ${minutes} minute(s), or press Ctrl+C.\n`);
  if (usable.length === 0) {
    console.log("No usable LAN address found. Is this PC connected to the network?");
  } else {
    console.log("Open one of these in the phone's browser, with the phone on Wi-Fi:");
    for (const c of usable) {
      const mark = c.address === likely ? "   <-- most likely (default route)" : "";
      console.log(`  http://${c.address}:${PORT}   (${c.name})${mark}`);
    }
  }
  if (skipped.length > 0) {
    console.log("\nSkipped: " + skipped.map((c) => `${c.name} ${c.address} (${c.skipped})`).join(", "));
  }
  console.log("\nWaiting for requests...");

  let stopping = false;
  const stop = (why: string) => {
    if (stopping) return;
    stopping = true;
    console.log(`\n[${clock()}] ${why}. ${requests} request(s) served.`);
    server.stop(true);
    process.exit(0);
  };
  process.on("SIGINT", () => stop("Stopped (Ctrl+C)"));
  process.on("SIGTERM", () => stop("Stopped (terminated)"));
  setTimeout(() => stop(`Stopped (${minutes} minute limit)`), minutes * 60_000);
}

if (import.meta.main) main();
