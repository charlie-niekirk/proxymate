import { useQuery } from "@tanstack/react-query";
import { Copy, Download, Maximize2, Minus, Plus, Scan } from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { isAnimated, jpegOrientation, orientationLabel } from "@/lib/body/exif";
import { formatSize } from "@/lib/flow";
import { cn } from "@/lib/utils";

import { Bar, Centered, Note } from "./parts";
import { formatLabel } from "./util";

const CHECKER = {
  backgroundColor: "var(--checker-b)",
  backgroundImage:
    "linear-gradient(45deg, var(--checker-a) 25%, transparent 25%, transparent 75%, var(--checker-a) 75%)," +
    "linear-gradient(45deg, var(--checker-a) 25%, transparent 25%, transparent 75%, var(--checker-a) 75%)",
  backgroundSize: "16px 16px",
  backgroundPosition: "0 0, 8px 8px",
};

function Info({ items }: { items: (string | null | false | undefined | 0)[] }) {
  return (
    <span className="text-muted-foreground truncate tabular-nums" data-testid="media-info">
      {items.filter(Boolean).join(" · ")}
    </span>
  );
}

function IconButton({ label, onClick, children, disabled }: { label: string; onClick: () => void; children: ReactNode; disabled?: boolean }) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button variant="ghost" size="icon-xs" onClick={onClick} aria-label={label} disabled={disabled}>
          {children}
        </Button>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );
}

type Zoom = "fit" | number;

/**
 * Images (SVG included, rendered as `<img>` so its scripts can't run) on a
 * checkerboard, with fit / 100% / zoom, dimensions, format, size and EXIF
 * orientation, plus save and copy.
 */
export function ImageView({ url, mime, size, onSave }: { url: string; mime: string; size: number; onSave?: () => void }) {
  const [zoom, setZoom] = useState<Zoom>("fit");
  const [dims, setDims] = useState<{ w: number; h: number } | null>(null);
  const [failed, setFailed] = useState(false);
  const [copied, setCopied] = useState<string | null>(null);
  const img = useRef<HTMLImageElement>(null);
  const meta = useQuery({
    queryKey: ["image-meta", url],
    enabled: size < 16 * 1024 * 1024,
    queryFn: async () => {
      const bytes = new Uint8Array(await (await fetch(url)).arrayBuffer());
      return { orientation: mime === "image/jpeg" ? jpegOrientation(bytes) : null, animated: isAnimated(bytes, mime) };
    },
  });
  const scale = zoom === "fit" ? null : zoom;
  const step = (dir: 1 | -1) => {
    const current = scale ?? (img.current && dims ? img.current.clientWidth / dims.w : 1);
    const levels = [0.1, 0.25, 0.5, 0.75, 1, 1.5, 2, 3, 4, 6, 8, 12, 16];
    const next = dir > 0 ? levels.find((l) => l > current + 1e-6) : [...levels].reverse().find((l) => l < current - 1e-6);
    setZoom(next ?? current);
  };
  const copy = async () => {
    try {
      const el = img.current!;
      const canvas = document.createElement("canvas");
      canvas.width = el.naturalWidth;
      canvas.height = el.naturalHeight;
      canvas.getContext("2d")!.drawImage(el, 0, 0);
      const png = new Promise<Blob>((resolve, reject) => canvas.toBlob((b) => (b ? resolve(b) : reject(new Error("encode failed"))), "image/png"));
      await navigator.clipboard.write([new ClipboardItem({ "image/png": png })]);
      setCopied("Copied");
    } catch (err) {
      setCopied(`Couldn't copy: ${err instanceof Error ? err.message : String(err)}`);
    }
  };
  // A load that finished before React committed the element (the viewers
  // render lazily, concurrently) has no onLoad; read the size here instead.
  useEffect(() => {
    const el = img.current;
    if (el?.complete && el.naturalWidth > 0) setDims({ w: el.naturalWidth, h: el.naturalHeight });
  }, [url]);
  useEffect(() => {
    if (!copied) return;
    const t = setTimeout(() => setCopied(null), 1500);
    return () => clearTimeout(t);
  }, [copied]);

  const label = formatLabel(mime);
  return (
    <div className="flex h-full flex-col" data-testid="image-view">
      <Bar>
        <IconButton label="Fit" onClick={() => setZoom("fit")}>
          <Maximize2 />
        </IconButton>
        <IconButton label="Actual size (100%)" onClick={() => setZoom(1)}>
          <Scan />
        </IconButton>
        <IconButton label="Zoom out" onClick={() => step(-1)}>
          <Minus />
        </IconButton>
        <IconButton label="Zoom in" onClick={() => step(1)}>
          <Plus />
        </IconButton>
        <span className="text-muted-foreground w-10 text-center tabular-nums">{scale === null ? "Fit" : `${Math.round(scale * 100)}%`}</span>
        <Info
          items={[
            dims && `${dims.w} × ${dims.h} px`,
            meta.data?.animated ? `${label} (animated)` : label,
            formatSize(size),
            meta.data?.orientation && meta.data.orientation !== 1 ? `EXIF: ${orientationLabel(meta.data.orientation)}` : null,
          ]}
        />
        <span className="ml-auto text-[11px] text-muted-foreground">{copied}</span>
        {onSave && (
          <IconButton label="Save image…" onClick={onSave}>
            <Download />
          </IconButton>
        )}
        <IconButton label="Copy image" onClick={() => void copy()} disabled={!dims}>
          <Copy />
        </IconButton>
      </Bar>
      {mime === "image/svg+xml" && <Note>Rendered as an image: its scripts and external resources never load.</Note>}
      <div className={cn("min-h-0 flex-1 overflow-auto", scale === null && "flex items-center justify-center p-3")}>
        {failed ? (
          <Centered>The webview can&apos;t decode this {label} image.</Centered>
        ) : (
          <div className={cn(scale !== null && "inline-block p-3")}>
            <img
              ref={img}
              src={url}
              alt=""
              crossOrigin="anonymous"
              draggable={false}
              onLoad={(e) => setDims({ w: e.currentTarget.naturalWidth, h: e.currentTarget.naturalHeight })}
              onError={() => setFailed(true)}
              className={cn("block", scale === null && "max-h-full max-w-full object-contain", (scale ?? 1) >= 2 && "[image-rendering:pixelated]")}
              style={{ ...CHECKER, ...(scale !== null && dims ? { width: dims.w * scale, height: dims.h * scale, maxWidth: "none" } : {}) }}
              data-testid="image"
            />
          </div>
        )}
      </div>
    </div>
  );
}

function formatTime(s: number): string {
  if (!Number.isFinite(s)) return "live";
  const m = Math.floor(s / 60);
  return `${m}:${(s % 60).toFixed(1).padStart(4, "0")}`;
}

/** `<video>` / `<audio>` with duration, dimensions, format and size. */
export function MediaView({ url, mime, size, kind }: { url: string; mime: string; size: number; kind: "video" | "audio" }) {
  const [meta, setMeta] = useState<{ duration: number; w: number; h: number } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const media = useRef<HTMLVideoElement & HTMLAudioElement>(null);
  const readMeta = (el: HTMLVideoElement | HTMLAudioElement) =>
    setMeta({ duration: el.duration, w: "videoWidth" in el ? el.videoWidth : 0, h: "videoHeight" in el ? el.videoHeight : 0 });
  const onMeta = (e: React.SyntheticEvent<HTMLVideoElement | HTMLAudioElement>) => readMeta(e.currentTarget);
  const onError = () => setError(`The webview can't play this ${formatLabel(mime)} file.`);
  // In a concurrent render (the viewers load lazily) the element exists before
  // React commits it, and React drops media events that arrive in between.
  useEffect(() => {
    const el = media.current;
    if (el?.error) onError();
    else if (el && el.readyState >= HTMLMediaElement.HAVE_METADATA) readMeta(el);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- once per source, after commit.
  }, [url]);
  return (
    <div className="flex h-full flex-col" data-testid={`${kind}-view`}>
      <Bar>
        <Info items={[formatLabel(mime), meta && formatTime(meta.duration), meta && meta.w > 0 && `${meta.w} × ${meta.h} px`, formatSize(size)]} />
      </Bar>
      {error ? (
        <Centered>{error}</Centered>
      ) : (
        <div className="flex min-h-0 flex-1 items-center justify-center bg-[var(--pane-table)] p-3">
          {kind === "video" ? (
            <video ref={media} src={url} controls preload="metadata" onLoadedMetadata={onMeta} onError={onError} className="max-h-full max-w-full" />
          ) : (
            <audio ref={media} src={url} controls preload="metadata" onLoadedMetadata={onMeta} onError={onError} className="w-full max-w-md" />
          )}
        </div>
      )}
    </div>
  );
}

/** The webview's own PDF renderer. */
export function PdfView({ url, size }: { url: string; size: number }) {
  return (
    <div className="flex h-full flex-col" data-testid="pdf-view">
      <Bar>
        <Info items={["PDF", formatSize(size)]} />
      </Bar>
      <iframe title="PDF preview" src={url} className="min-h-0 w-full flex-1 border-0 bg-[var(--pane-table)]" />
    </div>
  );
}

const GLYPHS = "ABCDEFGHIJKLMNOPQRSTUVWXYZ abcdefghijklmnopqrstuvwxyz 0123456789 !@#$%^&*()[]{}<>?/\\|;:'\",.-_+=~`";
const SIZES = [12, 16, 24, 36, 48];

/** Loads the font with the FontFace API and shows sample text in it. */
export function FontView({ url, mime, size, id }: { url: string; mime: string; size: number; id: string }) {
  const family = `pm-preview-${id.replace(/[^a-z0-9-]/gi, "-")}`;
  const [sample, setSample] = useState("The quick brown fox jumps over the lazy dog");
  const font = useQuery({
    queryKey: ["font", url],
    queryFn: async () => {
      const face = new FontFace(family, await (await fetch(url)).arrayBuffer());
      await face.load();
      document.fonts.add(face);
      return face;
    },
    gcTime: 0,
  });
  useEffect(() => {
    const face = font.data;
    return () => {
      if (face) document.fonts.delete(face);
    };
  }, [font.data]);
  return (
    <div className="flex h-full flex-col" data-testid="font-view">
      <Bar>
        <Info items={[formatLabel(mime), formatSize(size)]} />
        <Input value={sample} onChange={(e) => setSample(e.target.value)} className="ml-auto h-5 w-64 text-[11px]" aria-label="Sample text" />
      </Bar>
      {font.isError ? (
        <Centered>Couldn&apos;t load this font: {font.error.message}</Centered>
      ) : !font.data ? (
        <Centered>Loading font…</Centered>
      ) : (
        <div className="min-h-0 flex-1 overflow-auto px-4 py-3" style={{ fontFamily: `"${family}", var(--font-mono)` }} data-selectable>
          {SIZES.map((px) => (
            <div key={px} className="flex items-baseline gap-3 truncate" style={{ fontSize: px, lineHeight: 1.35 }}>
              <span className="text-muted-foreground w-8 shrink-0 text-right font-mono text-[10px]">{px}</span>
              <span className="truncate">{sample}</span>
            </div>
          ))}
          <div className="mt-3 border-t pt-3 text-xl leading-relaxed break-all">{GLYPHS}</div>
        </div>
      )}
    </div>
  );
}
