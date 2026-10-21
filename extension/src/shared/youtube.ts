// The "t=" timestamp for YouTube links. Pure functions; the video time is read in inject.ts.

const HOSTS = new Set(["youtube.com", "www.youtube.com", "m.youtube.com", "music.youtube.com", "youtu.be"]);

function parse(url: string): URL | null {
  try {
    return new URL(url);
  } catch {
    return null;
  }
}

export function isYoutubeHost(hostname: string): boolean {
  return HOSTS.has(hostname.toLowerCase());
}

/** The video ID of a watch page (watch?v=, youtu.be/ID, /live/ID), else null. Shorts and channels have none. */
export function youtubeVideoId(url: string): string | null {
  const u = parse(url);
  if (!u || (u.protocol !== "https:" && u.protocol !== "http:") || !isYoutubeHost(u.hostname)) return null;
  const id = /^[A-Za-z0-9_-]{6,20}$/;
  if (u.hostname.toLowerCase() === "youtu.be") {
    const first = u.pathname.split("/").filter(Boolean)[0];
    return first && id.test(first) ? first : null;
  }
  if (u.pathname === "/watch") {
    const v = u.searchParams.get("v");
    return v && id.test(v) ? v : null;
  }
  const live = /^\/live\/([A-Za-z0-9_-]{6,20})\/?$/.exec(u.pathname);
  return live ? live[1]! : null;
}

/**
 * Sets `t=<seconds>s`, replacing an existing `t`. The rest of the query is kept as it is
 * (no re-encoding), and so is the fragment.
 */
export function withTimestamp(url: string, seconds: number): string {
  const u = parse(url);
  if (!u) return url;
  const s = Math.max(0, Math.floor(seconds));
  const kept = u.search
    .replace(/^\?/, "")
    .split("&")
    .filter((p) => p !== "" && p.split("=")[0] !== "t");
  kept.push(`t=${s}s`);
  u.search = "?" + kept.join("&");
  return u.href;
}

export interface VideoState {
  time: number;
  /** false for a live stream, whose time is not a position. */
  seekable: boolean;
  ad: boolean;
}

/** Whether a link should get the current time of the video on screen. */
export function shouldStamp(targetUrl: string, tabUrl: string | undefined): boolean {
  if (!tabUrl) return false;
  const a = youtubeVideoId(targetUrl);
  const b = youtubeVideoId(tabUrl);
  return a !== null && a === b;
}

/** The time to write into the URL, or null when it would be wrong or pointless. */
export function usableTime(state: VideoState | null | undefined): number | null {
  if (!state || state.ad || !state.seekable) return null;
  if (!Number.isFinite(state.time)) return null;
  const s = Math.floor(state.time);
  return s >= 1 ? s : null;
}
