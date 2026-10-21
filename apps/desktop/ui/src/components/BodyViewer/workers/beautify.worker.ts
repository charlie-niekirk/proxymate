/// <reference lib="webworker" />
// Beautifies JS and CSS with prettier, loaded only when someone asks.
import * as babel from "prettier/plugins/babel";
import * as estree from "prettier/plugins/estree";
import * as postcss from "prettier/plugins/postcss";
import { format } from "prettier/standalone";

export interface BeautifyRequest {
  id: number;
  text: string;
  lang: "javascript" | "css";
}

export type BeautifyResponse = { id: number; ok: true; text: string } | { id: number; ok: false; error: string };

self.onmessage = async (e: MessageEvent<BeautifyRequest>) => {
  const { id, text, lang } = e.data;
  try {
    const out = await format(text, {
      parser: lang === "css" ? "css" : "babel",
      plugins: [babel, estree, postcss],
      printWidth: 100,
    });
    postMessage({ id, ok: true, text: out } satisfies BeautifyResponse);
  } catch (err) {
    postMessage({ id, ok: false, error: err instanceof Error ? err.message.split("\n")[0]! : String(err) } satisfies BeautifyResponse);
  }
};
