import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useRef } from "react";

import { formatSize } from "@/lib/flow";

import { Bar } from "./parts";

const ROW = 18;
const WIDTH = 16;
const HEX = Array.from({ length: 256 }, (_, i) => i.toString(16).padStart(2, "0"));

/** Offset / hex / ASCII, 16 bytes a row, virtualised so any size scrolls. */
export function HexView({ bytes }: { bytes: Uint8Array }) {
  const scroller = useRef<HTMLDivElement>(null);
  const rows = Math.ceil(bytes.length / WIDTH);
  // eslint-disable-next-line react-hooks/incompatible-library -- TanStack Virtual is not compiler-safe; rows re-render through their props.
  const virtualizer = useVirtualizer({
    count: rows,
    getScrollElement: () => scroller.current,
    estimateSize: () => ROW,
    overscan: 30,
  });
  const digits = Math.max(8, bytes.length.toString(16).length);
  return (
    <div className="flex h-full flex-col" data-testid="hex-view">
      <Bar>
        <span className="text-muted-foreground tabular-nums">
          {bytes.length.toLocaleString()} bytes ({formatSize(bytes.length)})
        </span>
      </Bar>
      <div ref={scroller} className="min-h-0 flex-1 overflow-auto font-mono text-[11.5px]" data-selectable>
        <div className="text-muted-foreground sticky top-0 z-10 flex h-[18px] items-center bg-[var(--pane-header)] px-2 whitespace-pre">
          <span style={{ width: `${digits + 2}ch` }}>Offset</span>
          <span>{HEX.slice(0, WIDTH).map((h, i) => (i === 8 ? ` ${h}` : h)).join(" ")}</span>
          <span className="ml-[2ch]">ASCII</span>
        </div>
        <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
          {virtualizer.getVirtualItems().map((item) => (
            <HexRow key={item.key} bytes={bytes} row={item.index} top={item.start} digits={digits} />
          ))}
        </div>
      </div>
    </div>
  );
}

const HexRow = memo(function HexRow({ bytes, row, top, digits }: { bytes: Uint8Array; row: number; top: number; digits: number }) {
  const offset = row * WIDTH;
  const slice = bytes.subarray(offset, offset + WIDTH);
  const cells: string[] = [];
  let ascii = "";
  for (let i = 0; i < WIDTH; i++) {
    const b = slice[i];
    cells.push(b === undefined ? "  " : HEX[b]!);
    ascii += b === undefined ? "" : b >= 0x20 && b < 0x7f ? String.fromCharCode(b) : "·";
  }
  return (
    <div className="absolute left-0 flex h-[18px] w-full items-center px-2 whitespace-pre hover:bg-[var(--code-row-hover)]" style={{ top }}>
      <span className="text-[var(--syntax-comment)]" style={{ width: `${digits + 2}ch` }}>
        {offset.toString(16).padStart(digits, "0")}
      </span>
      <span>
        {cells.map((c, i) => (
          <span key={i} className={c === "00" ? "text-muted-foreground/60" : undefined}>
            {i === 8 ? " " : ""}
            {c}
            {i < WIDTH - 1 ? " " : ""}
          </span>
        ))}
      </span>
      <span className="ml-[2ch] text-[var(--syntax-string)]">{ascii}</span>
    </div>
  );
});
