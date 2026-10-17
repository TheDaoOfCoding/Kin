// Functions that run INSIDE a web page, handed to scripting.executeScript({ func }).
// The browser serialises each one with toString(), so every function here must be completely
// self-contained: no imports, no references to anything outside its own body.

export interface PageVideoState {
  time: number;
  seekable: boolean;
  ad: boolean;
}

export function readVideoState(): PageVideoState | null {
  const video = document.querySelector<HTMLVideoElement>("video.html5-main-video") ?? document.querySelector<HTMLVideoElement>("video");
  if (!video) return null;
  const player = document.getElementById("movie_player") ?? document.querySelector(".html5-video-player");
  const ad = !!player && player.classList.contains("ad-showing");
  const seekable = Number.isFinite(video.duration) && video.duration > 0;
  return { time: video.currentTime, seekable, ad };
}

export type PageFetchResult =
  | { ok: true; data: string; mime: string }
  | { ok: false; reason: "http" | "too_large" | "not_image" | "fetch"; status?: number };

/** Fetches an image with the page's own cookies and returns it as base64. */
export async function fetchImageAsBase64(url: string, maxBytes: number): Promise<PageFetchResult> {
  try {
    const resp = await fetch(url, { credentials: "include" });
    if (!resp.ok) return { ok: false, reason: "http", status: resp.status };
    const declared = Number(resp.headers.get("content-length"));
    if (Number.isFinite(declared) && declared > maxBytes) return { ok: false, reason: "too_large" };
    const blob = await resp.blob();
    if (blob.size > maxBytes) return { ok: false, reason: "too_large" };
    const type = (blob.type || resp.headers.get("content-type") || "").split(";")[0]!.trim().toLowerCase();
    // Servers often label images application/octet-stream; Kin checks the bytes itself.
    if (type !== "" && type !== "application/octet-stream" && !type.startsWith("image/")) return { ok: false, reason: "not_image" };
    const dataUrl = await new Promise<string>((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => resolve(String(reader.result));
      reader.onerror = () => reject(reader.error);
      reader.readAsDataURL(blob);
    });
    const comma = dataUrl.indexOf(",");
    return { ok: true, data: comma >= 0 ? dataUrl.slice(comma + 1) : "", mime: type.startsWith("image/") ? type : "application/octet-stream" };
  } catch {
    return { ok: false, reason: "fetch" };
  }
}
