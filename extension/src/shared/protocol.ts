// Types of the extension <-> native host contract. See docs/extension.md.

export const PROTOCOL_VERSION = 1;

export const ERROR_CODES = [
  "KIN_NOT_RUNNING",
  "HOST_NOT_INSTALLED",
  "HOST_FORBIDDEN",
  "HOST_EXITED",
  "TIMEOUT",
  "NOT_PAIRED",
  "DEVICE_OFFLINE_QUEUED",
  "TOO_LARGE",
  "BAD_URL",
  "BAD_REQUEST",
  "UNSUPPORTED_TYPE",
  "UNSUPPORTED_VERSION",
  "IMAGE_NEEDS_BYTES",
  "IMAGE_UNREACHABLE",
  "NOT_AN_IMAGE",
  "INTERNAL",
] as const;
export type ErrorCode = (typeof ERROR_CODES)[number];

export const LIMITS = {
  /** Largest message the browser lets the extension send to the host (Chrome). */
  requestBytes: 64 * 1024 * 1024,
  /** Text of a `text` send, UTF-8 bytes. */
  textBytes: 4 * 1024 * 1024,
  /** Raw bytes of an image the extension fetched itself (base64 makes it 4/3 bigger). */
  imageBytes: 32 * 1024 * 1024,
  urlBytes: 8192,
  titleChars: 300,
} as const;

export interface DeviceInfo {
  id: string;
  name: string;
  online: boolean;
  queuedCount: number;
}

export interface StatusInfo {
  running: boolean;
  kinVersion?: string;
  hostVersion?: string;
  devices: DeviceInfo[];
  defaultDeviceId?: string;
  paused?: boolean;
}

export type RequestType = "status" | "start" | "open" | "send";

export interface BaseRequest {
  v: 1;
  id: string;
  type: RequestType;
}

export type SendBody =
  | { kind: "text"; text: string }
  | { kind: "link"; url: string; title?: string; timestampSec?: number }
  | { kind: "page"; url: string; title?: string }
  | { kind: "image"; url: string; pageUrl?: string; name?: string }
  | { kind: "image"; data: string; mime: string; name: string; retryOf?: string };

export type SendRequest = BaseRequest & { type: "send"; deviceId?: string } & SendBody;
export type HostRequest = BaseRequest | SendRequest | (BaseRequest & { type: "open"; target: "main" | "pairing" });

/** A response after the extension has checked and normalised it. */
export interface HostResult {
  ok: boolean;
  code?: string;
  message?: string;
  queued?: boolean;
  deviceId?: string;
  deviceName?: string;
  status?: StatusInfo;
}
