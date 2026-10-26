import { toast } from "sonner";

import { bodyText } from "@/lib/http";

/*
 * The one way the UI writes to the clipboard: menus, buttons and commands
 * all copy through here, so every copy confirms (or explains a failure) the
 * same way.
 */

export interface CopyOpts {
  /** No success toast: for buttons that show their own tick. Failures still toast. */
  silent?: boolean;
}

/** One toast id, so a burst of copies replaces the toast instead of stacking. */
const TOAST_ID = "copied";

function done(what: string | undefined, opts: CopyOpts) {
  if (!opts.silent) toast.success(what ? `Copied ${what}` : "Copied", { id: TOAST_ID, duration: 1200 });
}

function failed(err: unknown) {
  toast.error("Couldn't copy", { id: TOAST_ID, description: err instanceof Error ? err.message : String(err) });
}

/**
 * Copies `text`. `what` names it in the toast ("Copied header value").
 * Resolves false (after a toast) if the webview refused.
 */
export async function copy(text: string, what?: string, opts: CopyOpts = {}): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
  } catch (err) {
    failed(err);
    return false;
  }
  done(what, opts);
  return true;
}

/**
 * Copies text that has to be fetched first (a body, a cURL command). A
 * failure to fetch toasts like a failure to copy; null copies nothing.
 */
export async function copyLater(load: () => Promise<string | null>, what?: string, opts: CopyOpts = {}): Promise<boolean> {
  let text: string | null;
  try {
    text = await load();
  } catch (err) {
    failed(err);
    return false;
  }
  return text === null ? false : copy(text, what, opts);
}

/** Text to copy, or a function that fetches it (null copies nothing). */
export type CopySource = string | (() => Promise<string | null>);

export const copyFrom = (text: CopySource, what?: string) => (typeof text === "string" ? copy(text, what) : copyLater(text, what));

/** Copies a body as text; a body that isn't UTF-8 explains itself instead. */
export function copyBodyText(bytes: Uint8Array, what: string) {
  return copyLater(async () => {
    const text = bodyText(bytes);
    if (text === null) throw new Error(`The ${what} isn't UTF-8 text: copy it as Base64 instead`);
    return text;
  }, what);
}

/** Copies an image as PNG (the clipboard only takes PNG for images). */
export async function copyImage(png: Promise<Blob>, what = "image", opts: CopyOpts = {}): Promise<boolean> {
  try {
    await navigator.clipboard.write([new ClipboardItem({ "image/png": png })]);
  } catch (err) {
    failed(err);
    return false;
  }
  done(what, opts);
  return true;
}
