// Build-time settings. `bun scripts/build.ts` replaces the __X__ names; under `bun test` they are
// undefined and the dev values apply.
declare const __HOST_NAME__: string;

export const HOST_NAME: string = typeof __HOST_NAME__ !== "undefined" ? __HOST_NAME__ : "app.kinrelay.dev";

export const STATUS_TIMEOUT_MS = 8_000;
export const START_TIMEOUT_MS = 15_000;
export const SEND_TIMEOUT_MS = 90_000;

export const HISTORY_MAX = 10;
export const BADGE_MS = 4_000;
