// bun scripts/build.ts [--dev] [--test]
//   (no flag)  release build:  dist/chromium  dist/firefox            host app.kinrelay.desktop
//   --dev      dev build:      dist/chromium-dev  dist/firefox-dev    host app.kinrelay.dev
//   --test     test build:     dist/chromium-test                     host app.kinrelay.dev, test hooks,
//                                                                    local-server host permissions
import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { buildManifest, hostName, type Flavor, type Target } from "./manifest";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const dist = join(root, "dist");
const args = new Set(process.argv.slice(2));
const flavor: Flavor = args.has("--test") ? "test" : args.has("--dev") ? "dev" : "release";
const targets: Target[] = flavor === "test" ? ["chromium"] : ["chromium", "firefox"];

async function bundle(outDir: string): Promise<void> {
  const result = await Bun.build({
    entrypoints: [join(root, "src/background.ts"), join(root, "src/popup/popup.ts")],
    outdir: outDir,
    naming: { entry: "[dir]/[name].[ext]" },
    root: join(root, "src"),
    target: "browser",
    format: "iife",
    minify: { syntax: true, whitespace: false, identifiers: false },
    define: {
      __HOST_NAME__: JSON.stringify(hostName(flavor)),
      __TEST__: flavor === "test" ? "true" : "false",
    },
  });
  if (!result.success) {
    for (const log of result.logs) console.error(String(log));
    process.exit(1);
  }
}

for (const target of targets) {
  const name = flavor === "release" ? target : `${target}-${flavor}`;
  const out = join(dist, name);
  rmSync(out, { recursive: true, force: true });
  mkdirSync(out, { recursive: true });
  await bundle(out);
  cpSync(join(root, "assets/icons"), join(out, "icons"), { recursive: true });
  for (const f of ["popup.html", "popup.css"]) cpSync(join(root, "src/popup", f), join(out, "popup", f));
  // popup.html lives next to popup.js
  if (!existsSync(join(out, "popup/popup.js"))) throw new Error("popup.js missing from the bundle");
  writeFileSync(join(out, "manifest.json"), JSON.stringify(buildManifest(target, flavor), null, 2) + "\n");
  const size = readFileSync(join(out, "background.js")).length;
  console.log(`built ${name} (${flavor}, host ${hostName(flavor)}, background.js ${(size / 1024).toFixed(1)} KiB)`);
}
