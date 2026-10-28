import type { BodyPart, DecodedMessage_Serialize, ProtoMapping_Serialize } from "@/bindings";
import { emptyMatcher, type Matcher } from "@/lib/rules";

/** A decoded message's badges, grouped by JSON pointer for the tree. */
export function badgeMap(message: Pick<DecodedMessage_Serialize, "badges">): Map<string, string[]> {
  const map = new Map<string, string[]>();
  for (const b of message.badges) {
    const list = map.get(b.pointer);
    if (list) list.push(b.label);
    else map.set(b.pointer, [b.label]);
  }
  return map;
}

/** One line of a message for the list: its JSON squashed, cut at `max`. */
export function preview(json: string, max = 120): string {
  const flat = json.replace(/\s*\n\s*/g, " ");
  return flat.length > max ? `${flat.slice(0, max - 1)}…` : flat;
}

/** The matcher "Decode as…" saves: this flow's host and path, exactly. */
export function flowMatcher(url: string): Matcher {
  const u = new URL(url);
  return {
    ...emptyMatcher(),
    host: { kind: "exact", value: u.hostname, case_insensitive: true },
    path: { kind: "exact", value: u.pathname, case_insensitive: false },
  };
}

/** The mapping that applies to this flow's part, as "Decode as…" set it. */
export function mappingFor(mappings: ProtoMapping_Serialize[], url: string, part: BodyPart): ProtoMapping_Serialize | undefined {
  const m = flowMatcher(url);
  return mappings.find(
    (x) => x.part === part && x.matcher.host?.value === m.host?.value && x.matcher.path?.value === m.path?.value,
  );
}

/** `mappings` with this flow's part decoding as `type` (or as its default, with `null`). */
export function withMapping(
  mappings: ProtoMapping_Serialize[],
  url: string,
  part: BodyPart,
  type: string | null,
  id: string,
): ProtoMapping_Serialize[] {
  const current = mappingFor(mappings, url, part);
  const rest = mappings.filter((m) => m !== current);
  if (type === null) return rest;
  return [...rest, { id: current?.id ?? id, matcher: flowMatcher(url), part, message_type: type }];
}
