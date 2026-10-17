// The one place that talks to the native messaging host.
import { HOST_NAME } from "./shared/config";
import { classifyNativeError, describeError, parseResponse } from "./shared/messages";
import type { HostRequest, HostResult } from "./shared/protocol";

type Ext = typeof chrome;
const g = globalThis as unknown as { browser?: Ext; chrome?: Ext };
export const ext: Ext = (g.browser ?? g.chrome) as Ext;

let hostName = HOST_NAME;

/** Tests point the extension at a host name that doesn't exist. */
export function setHostName(name: string): void {
  hostName = name;
}

function failure(code: string, message?: string): HostResult {
  return { ok: false, code, message: describeError(code, message) };
}

/**
 * Sends one request, waits for its one response. Never throws: a browser error, a timeout or a
 * malformed answer all come back as `{ ok: false, code, message }`.
 */
export async function callHost(request: HostRequest, timeoutMs: number): Promise<HostResult> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    const timeout = new Promise<"timeout">((resolve) => {
      timer = setTimeout(() => resolve("timeout"), timeoutMs);
    });
    const answer = await Promise.race([ext.runtime.sendNativeMessage(hostName, request), timeout]);
    if (answer === "timeout") return failure("TIMEOUT");
    return parseResponse(answer, request);
  } catch (e) {
    const text = e instanceof Error ? e.message : String(e);
    return failure(classifyNativeError(text));
  } finally {
    if (timer !== undefined) clearTimeout(timer);
  }
}
