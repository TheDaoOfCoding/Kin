// Generates THIRD_PARTY_NOTICES.md at the repository root.
//
//   bun tools/gen-third-party-notices.ts
//
// What it reads (nothing is downloaded except what cargo and gradle already have):
//   * Rust: `cargo tree` and `cargo metadata` of desktop/src-tauri and desktop/native-host for x86_64-pc-windows-msvc, normal dependencies only
//     (what the Windows build links), and the license files that sit in each crate's source directory.
//   * Android: `gradlew :app:dependencies --configuration releaseRuntimeClasspath` (or the file named by KIN_GRADLE_DEPS_FILE), and the
//     license each artifact's POM declares in the Gradle cache.
//   * JavaScript: @tauri-apps/api in desktop/node_modules (the only runtime dependency of the desktop UI; the extension has none).
// Run `bun install` in desktop/ and a build of both Rust crates first, so the crate sources and node_modules exist.
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";

const root = resolve(import.meta.dir, "..");
const TARGET = "x86_64-pc-windows-msvc";

function run(cmd: string[], cwd: string): string {
  const p = Bun.spawnSync(cmd, { cwd, stdout: "pipe", stderr: "pipe" });
  if (p.exitCode !== 0) throw new Error(`${cmd.join(" ")} failed in ${cwd}: ${p.stderr.toString()}`);
  return p.stdout.toString();
}

// ---- Rust ---------------------------------------------------------------------------------------------------------------------------

type Crate = { name: string; version: string; license: string; repo: string; dir: string; copyrights: string[]; files: string[] };
const crates = new Map<string, Crate>();

for (const rel of ["desktop/src-tauri", "desktop/native-host"]) {
  const cwd = join(root, rel);
  const meta = JSON.parse(run(["cargo", "metadata", "--format-version", "1", "--filter-platform", TARGET, "--locked"], cwd));
  const dirs = new Map<string, string>();
  for (const p of meta.packages) dirs.set(`${p.name}@${p.version}`, dirname(p.manifest_path));
  const tree = run(["cargo", "tree", "-e", "normal", "--target", TARGET, "--prefix", "none", "--format", "{p}|{l}|{r}", "--locked"], cwd);
  for (const raw of tree.split(/\r?\n/)) {
    const line = raw.replace(/ \(\*\)$/, "").replace(/ \(proc-macro\)/, "").trim();
    if (!line || line.includes(":\\") || line.includes("AGPL-3.0-only")) continue; // the workspace's own crates
    const [id, license, repo] = line.split("|");
    const m = /^(\S+) v(\S+)$/.exec(id);
    if (!m) continue;
    const key = `${m[1]}@${m[2]}`;
    if (crates.has(key)) continue;
    const dir = dirs.get(key);
    if (!dir) throw new Error(`no source directory for ${key}`);
    crates.set(key, { name: m[1], version: m[2], license: license || "(see the crate)", repo: repo || "", dir, copyrights: [], files: [] });
  }
}

const LICENSE_FILE = /^(licen[sc]e|copying|unlicense|notice)/i;
const NOT_A_HOLDER = /owner|notice|holders|law|statement|ownership|licen[sc]e|\[yyyy\]|the copyright/i;
for (const c of crates.values()) {
  const seen = new Set<string>();
  for (const f of readdirSync(c.dir).filter((f) => LICENSE_FILE.test(f)).sort()) {
    c.files.push(f);
    for (const l of readFileSync(join(c.dir, f), "utf8").split(/\r?\n/)) {
      const t = l.replace(/^[\s/*#>-]+/, "").replace(/\s+/g, " ").trim();
      if (/^(copyright|\(c\)|©)/i.test(t) && /\d{4}|©|authors|contributors|developers/i.test(t) && !NOT_A_HOLDER.test(t) && t.length < 220) seen.add(t);
    }
  }
  c.copyrights = [...seen].slice(0, 4);
}

// ---- Android ------------------------------------------------------------------------------------------------------------------------

type Artifact = { id: string; version: string; license: string };
function androidArtifacts(): Artifact[] {
  const file = process.env.KIN_GRADLE_DEPS_FILE;
  const out = file
    ? readFileSync(file, "utf8")
    : run(["cmd", "/c", "gradlew.bat", "--console=plain", "-q", ":app:dependencies", "--configuration", "releaseRuntimeClasspath"], join(root, "android"));
  const found = new Map<string, string>();
  for (const l of out.replace(/\0/g, "").split(/\r?\n/)) {
    const m = /--- ([\w.\-]+:[\w.\-]+)(?::([^\s]+))?(?: -> ([^\s]+))?( \((c|n)\))?/.exec(l);
    if (!m || m[4]) continue; // constraints and not-resolved lines do not ship
    const version = m[3] ?? m[2]; // "a:b:1.0 -> 2.0" resolved to 2.0; every line of one artifact ends at the same version
    if (version && !version.startsWith("{")) found.set(m[1], version);
  }
  const gradleCache = join(process.env.GRADLE_USER_HOME ?? join(homedir(), ".gradle"), "caches", "modules-2", "files-2.1");
  return [...found.entries()]
    .map(([id, version]) => ({ id, version }))
    .sort((a, b) => a.id.localeCompare(b.id))
    .map((a) => ({ ...a, license: androidLicense(a.id, a.version, gradleCache) }));
}

function androidLicense(id: string, version: string, cache: string): string {
  const [group, name] = id.split(":");
  const base = join(cache, group, name, version);
  if (existsSync(base)) {
    for (const hash of readdirSync(base)) {
      const dir = join(base, hash);
      for (const f of readdirSync(dir).filter((f) => f.endsWith(".pom"))) {
        const m = /<licenses>[\s\S]*?<name>([^<]+)<\/name>/.exec(readFileSync(join(dir, f), "utf8"));
        if (m) return normalizeAndroidLicense(m[1].trim());
      }
    }
  }
  // Artifacts with no license in their own POM (AndroidX parents, bills of materials, the Guava helper jars).
  if (group.startsWith("androidx") || group.startsWith("org.jetbrains") || group.startsWith("com.squareup") || group.startsWith("com.google")) return "Apache-2.0";
  return "(see the project)";
}

function normalizeAndroidLicense(name: string): string {
  if (/apache/i.test(name)) return "Apache-2.0";
  if (/^the mit/i.test(name) || /^mit/i.test(name)) return "MIT";
  return name;
}

// ---- JavaScript ---------------------------------------------------------------------------------------------------------------------

const tauriApi = JSON.parse(readFileSync(join(root, "desktop", "node_modules", "@tauri-apps", "api", "package.json"), "utf8"));

// ---- License texts ------------------------------------------------------------------------------------------------------------------

const registry = (() => {
  const base = join(process.env.CARGO_HOME ?? join(homedir(), ".cargo"), "registry", "src");
  return join(base, readdirSync(base)[0]);
})();
const crateFile = (crate: string, file: string): string => {
  const c = [...crates.values()].find((x) => x.name === crate);
  if (!c) throw new Error(`crate ${crate} is not in the dependency list any more: pick another source for its license text`);
  return readFileSync(join(c.dir, file), "utf8").replace(/\r\n/g, "\n").trim();
};

// A permissive license with its copyright line(s) taken out: the holders are listed per package above.
const withoutCopyright = (text: string): string =>
  "Copyright (c) <year> <copyright holders>\n\n" +
  text
    .split("\n")
    .filter((l) => !/^\s*(\/\/\s*)?copyright\s*(\(c\)|©|\d{4})/i.test(l))
    .map((l) => l.replace(/^\/\/ ?/, ""))
    .join("\n")
    .trim();

const TEXTS: [string, string][] = [
  ["MIT", "Copyright (c) <year> <copyright holders>\n\n" + crateFile("itoa", "LICENSE-MIT")],
  ["Apache-2.0", crateFile("itoa", "LICENSE-APACHE")],
  ["BSD-2-Clause", withoutCopyright(crateFile("zerocopy", "LICENSE-BSD"))],
  ["BSD-3-Clause", withoutCopyright(crateFile("subtle", "LICENSE"))],
  ["ISC", withoutCopyright(crateFile("untrusted", "LICENSE.txt"))],
  ["0BSD", withoutCopyright(crateFile("adler2", "LICENSE-0BSD"))],
  ["Zlib", withoutCopyright(crateFile("foldhash", "LICENSE"))],
  ["Unlicense", crateFile("memchr", "UNLICENSE")],
  ["CC0-1.0", crateFile("dunce", "LICENSE")],
  ["Unicode-3.0", crateFile("unicode-ident", "LICENSE-UNICODE")],
  ["MPL-2.0", crateFile("cssparser", "LICENSE")],
  ["CDLA-Permissive-2.0", crateFile("webpki-roots", "LICENSE")],
];
const WHOLE_FILES: [string, string[]][] = [
  ["aws-lc-sys (AWS-LC, BoringSSL, OpenSSL and others)", [crateFile("aws-lc-sys", "LICENSE")]],
  ["ring", ["LICENSE", "LICENSE-other-bits", "LICENSE-BoringSSL"].map((f) => `--- ${f} ---\n\n` + crateFile("ring", f))],
];

// ---- Output -------------------------------------------------------------------------------------------------------------------------

const md = (s: string) => s.replace(/\|/g, "\\|");
const fence = (s: string) => "```text\n" + s + "\n```\n";
const out: string[] = [];
const sorted = [...crates.values()].sort((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version));
const android = androidArtifacts();

out.push(`# Third-party notices

Kin Relay itself is licensed under the [GNU AGPL-3.0](LICENSE). It includes and depends on the third-party software listed below, under their own licenses. This file is generated by \`tools/gen-third-party-notices.ts\`; regenerate it when dependencies change.

- [The Windows app and the native messaging host (Rust)](#the-windows-app-and-the-native-messaging-host-rust)
- [The Windows installer (NSIS template and plugin)](#the-windows-installer-nsis-template-and-plugin)
- [The Windows app's user interface (JavaScript)](#the-windows-apps-user-interface-javascript)
- [The Android app](#the-android-app)
- [The browser extension](#the-browser-extension)
- [Full license texts](#full-license-texts)

Where a package offers a choice of licenses ("MIT OR Apache-2.0"), you may use it under either. The packages under MPL-2.0 are used unmodified; their source is available from the repositories listed.

## The Windows app and the native messaging host (Rust)

These are the ${sorted.length} crates (name, version, license and where to get the source) that the Windows build of \`desktop/src-tauri\` and \`desktop/native-host\` links, as reported by \`cargo tree -e normal --target ${TARGET}\`, with the copyright lines found in each crate's license files. Some are only used at build time (procedural macros); they are listed to be safe.

| Crate | Version | License | Copyright | Source |
|---|---|---|---|---|`);
for (const c of sorted) {
  out.push(`| ${md(c.name)} | ${c.version} | ${md(c.license)} | ${c.copyrights.length ? md(c.copyrights.join("; ")) : "see the crate's license files"} | ${c.repo ? `<${c.repo}>` : ""} |`);
}

out.push(`
## The Windows installer (NSIS template and plugin)

- **Tauri's NSIS installer template.** \`desktop/src-tauri/windows/installer.nsi\` is a modified copy of the NSIS installer template of tauri-bundler (as shipped with tauri-cli 2.12.2), <https://github.com/tauri-apps/tauri>. Copyright (c) Tauri Programme within The Commons Conservancy. Licensed under **MIT OR Apache-2.0**; the full texts are under [Full license texts](#full-license-texts) (MIT and Apache-2.0). Kin Relay's changes are marked \`KIN:\` in the file and are licensed under AGPL-3.0-only.
- **nsis_tauri_utils** (an NSIS plugin that the Tauri bundler puts into the installer), <https://github.com/tauri-apps/nsis-tauri-utils>. Copyright (c) Tauri Programme within The Commons Conservancy. **MIT OR Apache-2.0**.
- **NSIS** (Nullsoft Scriptable Install System), <https://nsis.sourceforge.io>. The installer executable contains NSIS's installer stub, which is under the zlib/libpng license (see Zlib below), plus bzip2 (Copyright 1996-2010 Julian Seward, BSD-style license) and the LZMA SDK (public domain), as described in <https://nsis.sourceforge.io/NSIS_License>.

## The Windows app's user interface (JavaScript)

| Package | Version | License | Source |
|---|---|---|---|
| @tauri-apps/api | ${tauriApi.version} | ${tauriApi.license} | <https://github.com/tauri-apps/tauri> |

Copyright (c) Tauri Programme within The Commons Conservancy. Licensed under MIT OR Apache-2.0 (see below). The build tools (TypeScript, Vite) are not part of the shipped app.

## The Android app

The runtime dependencies of the release build (\`gradlew :app:dependencies --configuration releaseRuntimeClasspath\`, versions as resolved). The licenses are those declared in each artifact's POM. The list is conservative: the release build shrinks away what it does not use. Kin Relay's own code, resources and icons are covered by the AGPL-3.0.

| Library | Version | License |
|---|---|---|`);
for (const a of android) out.push(`| ${md(a.id)} | ${md(a.version)} | ${md(a.license)} |`);

out.push(`
Copyright notices: the AndroidX and Jetpack Compose libraries are Copyright (c) The Android Open Source Project; Kotlin and kotlinx are Copyright (c) JetBrains s.r.o. and Kotlin contributors; OkHttp and Okio are Copyright (c) Square, Inc.; ZXing is Copyright (c) ZXing authors; Guava and the Google annotation libraries are Copyright (c) Google LLC; the Checker Framework's \`checker-qual\` is Copyright (c) The Checker Framework developers (MIT); Dagger is Copyright (c) The Dagger Authors; \`javax.inject\` and \`jakarta.inject\` are Copyright (c) their authors; JSpecify is Copyright (c) The JSpecify Authors.

## The browser extension

The extension has **no runtime dependencies**: the code in \`extension/src\` is Kin Relay's own, and its build tools (TypeScript, bun types, Playwright) are used only for development and testing.

## Full license texts

The text of each license that appears above, once. For the permissive licenses (MIT, BSD, ISC, 0BSD, Zlib) the copyright notices of each package are the ones listed in the tables above.
`);
for (const [id, text] of TEXTS) out.push(`### ${id}\n\n${fence(text)}`);
out.push(`### Licenses reproduced in full for packages that carry several\n`);
for (const [name, parts] of WHOLE_FILES) out.push(`#### ${name}\n\n${fence(parts.join("\n\n"))}`);

writeFileSync(join(root, "THIRD_PARTY_NOTICES.md"), out.join("\n").replace(/\n{3,}/g, "\n\n") + "\n");
console.log(`wrote THIRD_PARTY_NOTICES.md: ${sorted.length} crates, ${android.length} Android artifacts`);
const licenses = new Set(sorted.flatMap((c) => c.license.split(/\s+(?:OR|AND)\s+|\s*\/\s*|[()]/).map((s) => s.trim()).filter(Boolean)));
const covered = new Set(TEXTS.map(([id]) => id));
const missing = [...licenses].filter((l) => !covered.has(l));
if (missing.length) console.log(`license ids with no text above (check they are only alternatives or covered by the whole-file section): ${missing.join(", ")}`);
