import { lazy, Suspense } from "react";

import { loadViewers } from "./code/load";
import { Centered } from "./parts";
import type { ViewerId } from "./registry";

export interface ViewerInput {
  viewer: ViewerId;
  /** The decoded body, for "bytes" viewers. */
  bytes: Uint8Array | null;
  /** Where to load the body from, for "url" viewers. */
  url: string | null;
  mime: string;
  size: number;
  contentType: string | null;
  highlight: boolean;
  /** Unique per body, for names that must not clash (fonts). */
  bodyId: string;
  onSave?: () => void;
}

const ViewerSwitch = lazy(() => loadViewers().then((m) => ({ default: m.ViewerSwitch })));

/**
 * Renders one body with one viewer from the registry. The viewers (and
 * CodeMirror behind them) load on first use, outside the startup bundle.
 */
export function Viewer(props: ViewerInput) {
  return (
    <Suspense fallback={<Centered>Loading…</Centered>}>
      <ViewerSwitch {...props} />
    </Suspense>
  );
}
