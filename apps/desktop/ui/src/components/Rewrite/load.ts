//! Loads what the Rewrite dialog edits: the flow as a `Source`, and an
//! existing rule rebuilt into edits (or why its actions are kept as they are).

import { docInWorker, WORKER_THRESHOLD } from "@/components/BodyViewer/workers/client";
import { backend } from "@/lib/backend";
import { newId } from "@/lib/json/doc";
import { isVisual, parsePairs, type Pins } from "@/lib/rewrite/diff";
import { matcherFor, modeOf, type FlowTarget } from "@/lib/rewrite/matcher";
import { bodySource, headerRows, initialEdits, PREVIEW_LIMIT, TREE_MAX, rawQuery, rebuild, type Edits, type Source } from "@/lib/rewrite/session";
import { normalizeRule, type Action, type Rule } from "@/lib/rules";

import { type MatchChoice } from "./MatcherChips";

export const errorText = (e: unknown) => (e instanceof Error ? e.message : String(e));

/** The first byte that isn't JSON whitespace. */
function firstByte(bytes: Uint8Array): number {
  for (const b of bytes) if (b !== 0x20 && b !== 0x0a && b !== 0x0d && b !== 0x09) return b;
  return 0;
}

export interface Loaded {
  source: Source;
  /** The rule being edited: a saved one, or the Rewrite preset's draft. */
  base: Rule;
  initial: Edits;
  pins: Pins;
  kept: Action[];
  choice: MatchChoice;
  /** Why a saved rule's edits couldn't be rebuilt on this flow, if they couldn't. */
  note: string | null;
}

export async function load(store: number, flow: number, rule: Rule | null): Promise<Loaded> {
  const [summary, detail, draft] = await Promise.all([
    backend.getFlow(store, flow),
    backend.getFlowDetail(store, flow),
    backend.draftRule(store, flow, "rewrite"),
  ]);
  if (!summary || !detail) throw new Error(`flow ${flow + 1} has no recorded messages`);
  const [reqBytes, resBytes] = await Promise.all(
    (["request", "response"] as const).map(async (part) => new Uint8Array(await backend.getBody(store, flow, part, true))),
  );
  // Large JSON bodies become documents off the main thread.
  const preparse = async (bytes: Uint8Array) => {
    if (bytes.length < WORKER_THRESHOLD || bytes.length > TREE_MAX || ![0x7b, 0x5b].includes(firstByte(bytes))) return null;
    return docInWorker(bytes).catch(() => null);
  };
  const [reqDoc, resDoc] = [await preparse(reqBytes!), await preparse(resBytes!)];
  const query = parsePairs(rawQuery(detail.request.url), newId);
  const contentType = (headers: { name: string; value: string }[]) => headers.find((h) => h.name.toLowerCase() === "content-type")?.value ?? null;
  const target: FlowTarget = {
    method: summary.method,
    scheme: summary.scheme,
    host: summary.host,
    port: summary.port,
    path: summary.path,
    query: query.map((r) => [r.name, r.value]),
  };
  const reqType = contentType(detail.request.headers);
  const resType = detail.response ? contentType(detail.response.headers) : null;
  const source: Source = {
    store,
    flow,
    target,
    url: detail.request.url,
    request: {
      headers: headerRows(detail.request.headers),
      query,
      body: bodySource(reqBytes!, reqType, detail.request_body.kind, "request", query, reqDoc),
      contentType: reqType,
    },
    response: detail.response && {
      status: detail.response.status,
      headers: headerRows(detail.response.headers),
      body: bodySource(resBytes!, resType, detail.response_body.kind, "response", [], resDoc),
      contentType: resType,
    },
  };
  if (!rule) {
    return {
      source,
      base: { ...normalizeRule(draft), matcher: matcherFor(target, "path", false) },
      initial: initialEdits(source),
      pins: new Set(),
      kept: [],
      choice: { mode: "path", anyMethod: false, matcher: matcherFor(target, "path", false) },
      note: null,
    };
  }
  const visual = { ...rule, actions: rule.actions.filter(isVisual) };
  const dry = await backend.testRule(store, visual, flow, PREVIEW_LIMIT);
  const chip = modeOf(target, rule.matcher);
  const choice: MatchChoice = chip ? { ...chip, matcher: rule.matcher } : { mode: "advanced", anyMethod: false, matcher: rule.matcher };
  if (!dry.matched) {
    return { source, base: rule, initial: initialEdits(source), pins: new Set(), kept: rule.actions, choice, note: `The rule doesn't match this flow (${dry.mismatch ?? "no match"}): its actions are kept as they are.` };
  }
  const rebuilt = rebuild(source, rule, dry);
  return { source, base: rule, initial: rebuilt.edits, pins: rebuilt.pins, kept: rebuilt.kept, choice, note: null };
}
