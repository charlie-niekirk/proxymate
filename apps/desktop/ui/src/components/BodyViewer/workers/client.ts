import type { BeautifyRequest, BeautifyResponse } from "./beautify.worker";
import { jsonOp } from "./jsonOp";
import type { JsonRequest, JsonResponse } from "./json.worker";

/** Bodies below this are parsed on the main thread: a worker round trip costs more. */
export const WORKER_THRESHOLD = 256 * 1024;

type Pending<T> = Map<number, { resolve: (v: T) => void; reject: (e: Error) => void }>;

function rpc<Req extends { id: number }, Res extends { id: number; ok: boolean }, Out>(
  create: () => Worker,
  unwrap: (res: Res) => Out,
): (req: Omit<Req, "id">, transfer?: Transferable[]) => Promise<Out> {
  let worker: Worker | null = null;
  let next = 0;
  const pending: Pending<Out> = new Map();
  return (req, transfer = []) => {
    if (!worker) {
      worker = create();
      worker.onmessage = (e: MessageEvent<Res>) => {
        const p = pending.get(e.data.id);
        pending.delete(e.data.id);
        if (!p) return;
        try {
          p.resolve(unwrap(e.data));
        } catch (err) {
          p.reject(err as Error);
        }
      };
    }
    const id = ++next;
    return new Promise<Out>((resolve, reject) => {
      pending.set(id, { resolve, reject });
      worker!.postMessage({ ...req, id }, transfer);
    });
  };
}

const jsonRpc = rpc<JsonRequest, JsonResponse, unknown>(
  () => new Worker(new URL("./json.worker.ts", import.meta.url), { type: "module" }),
  (res) => {
    if (!res.ok) throw new Error(res.error);
    return res.result;
  },
);

/** Parses JSON (or pretty-prints it, with `"pretty"`), in a worker for large bodies. */
export async function jsonInWorker(bytes: Uint8Array, op: "parse" | "pretty"): Promise<unknown> {
  if (bytes.length < WORKER_THRESHOLD) return jsonOp(new TextDecoder().decode(bytes), op);
  const copy = bytes.slice();
  return jsonRpc({ bytes: copy, op }, [copy.buffer]);
}

const beautifyRpc = rpc<BeautifyRequest, BeautifyResponse, string>(
  () => new Worker(new URL("./beautify.worker.ts", import.meta.url), { type: "module" }),
  (res) => {
    if (!res.ok) throw new Error(res.error);
    return res.text;
  },
);

export function beautify(text: string, lang: "javascript" | "css"): Promise<string> {
  return beautifyRpc({ text, lang });
}

const ids = new WeakMap<object, number>();
let nextId = 0;

/** A stable query-key id for an object (a body's bytes). */
export function objectId(o: object): number {
  let id = ids.get(o);
  if (id === undefined) {
    id = ++nextId;
    ids.set(o, id);
  }
  return id;
}
