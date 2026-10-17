// Builds manifest.json for each target and flavor. Kept apart from build.ts so a test can read it.
import key from "../manifest-key.json";
import pkg from "../package.json";

export type Target = "chromium" | "firefox";
export type Flavor = "release" | "dev" | "test";

/** The only permissions the extension may ask for (a test checks it). */
export const PERMISSIONS = ["contextMenus", "nativeMessaging", "activeTab", "scripting", "storage"] as const;
export const FIREFOX_ID = "kin@kinrelay.app";
export const CHROMIUM_ID = key.id;

export function hostName(flavor: Flavor): string {
  return flavor === "release" ? "app.kinrelay.desktop" : "app.kinrelay.dev";
}

export function buildManifest(target: Target, flavor: Flavor, opts: { withKey?: boolean } = {}): Record<string, unknown> {
  const manifest: Record<string, unknown> = {
    manifest_version: 3,
    name: flavor === "release" ? "Kin Relay" : "Kin Relay (dev)",
    short_name: "Kin",
    version: pkg.version,
    description: "Send links, text, images and pages from your browser to your phone with Kin. Everything stays on your network.",
    icons: { "16": "icons/icon-16.png", "32": "icons/icon-32.png", "48": "icons/icon-48.png", "128": "icons/icon-128.png" },
    permissions: [...PERMISSIONS],
    action: {
      default_title: "Send to Kin",
      default_popup: "popup/popup.html",
      default_icon: { "16": "icons/icon-16.png", "32": "icons/icon-32.png" },
    },
    commands: {
      "send-tab": {
        suggested_key: { default: "Alt+Shift+K" },
        description: "Send the current tab to your phone with Kin",
      },
    },
  };

  if (target === "chromium") {
    manifest.minimum_chrome_version = "116";
    manifest.background = { service_worker: "background.js" };
    // The key gives every unpacked build the same extension ID, which the host's allowed_origins names.
    // The store zips leave it out: the stores assign their own ID.
    if (opts.withKey !== false) manifest.key = key.key;
  } else {
    manifest.background = { scripts: ["background.js"] };
    manifest.browser_specific_settings = {
      gecko: {
        id: FIREFOX_ID,
        strict_min_version: "140.0",
        // Required by addons.mozilla.org for new extensions: Kin collects nothing.
        data_collection_permissions: { required: ["none"] },
      },
      gecko_android: { strict_min_version: "142.0" },
    };
  }

  if (flavor === "test") {
    // Only the test build, only for the local servers of the smoke test: it lets the test inject
    // into its own pages without the click that grants activeTab (headless can't click the toolbar).
    manifest.host_permissions = ["http://127.0.0.1/*", "http://localhost/*", "https://www.youtube.com/*", "https://youtu.be/*"];
  }
  return manifest;
}
