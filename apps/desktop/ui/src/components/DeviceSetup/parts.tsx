import { CircleAlert, CircleCheck, Copy, LoaderCircle, Wifi } from "lucide-react";
import { useState, type ReactNode } from "react";

import type { DeviceAccess_Serialize } from "@/bindings";
import { Button } from "@/components/ui/button";
import { svgDataUrl, type Progress } from "@/lib/devices";
import { cn } from "@/lib/utils";

export type CheckState = "ok" | "warn" | "pending" | "idle";

/** One line of a live checklist. */
export function Check({ state, label, children }: { state: CheckState; label: ReactNode; children?: ReactNode }) {
  const Icon = state === "ok" ? CircleCheck : state === "pending" ? LoaderCircle : CircleAlert;
  return (
    <div className="flex items-start gap-2" data-state={state}>
      <Icon
        className={cn(
          "mt-px size-4 shrink-0",
          state === "ok" && "text-[var(--s-2xx)]",
          state === "warn" && "text-[var(--s-4xx)]",
          state === "pending" && "text-muted-foreground animate-spin",
          state === "idle" && "text-muted-foreground",
        )}
      />
      <div className="min-w-0 flex-1">
        <div className="font-medium">{label}</div>
        {children && <div className="text-muted-foreground">{children}</div>}
      </div>
    </div>
  );
}

/** The live check for a device's first flows. */
export function ProgressCheck({ progress, who, flows }: { progress: Progress; who: string; flows: number }) {
  switch (progress) {
    case "decrypting":
      return <Check state="ok" label={`Decrypting HTTPS from ${who}`}>{flows} flows so far.</Check>;
    case "untrusted":
      return (
        <Check state="warn" label={`${who} refused Proxymate's certificate`}>
          Traffic arrives, but HTTPS fails: install and trust the certificate, then reopen the app.
        </Check>
      );
    case "connected":
      return (
        <Check state="pending" label={`Waiting for the first HTTPS flow from ${who}…`}>
          {flows} plain flow{flows === 1 ? "" : "s"} seen. Open an https:// site.
        </Check>
      );
    case "waiting":
      return <Check state="pending" label={`Waiting for the first flow from ${who}…`} />;
  }
}

/** Monospace text with a copy button. `display` shows something else (e.g. a masked secret) but copies `value`. */
export function CopyField({
  value,
  display,
  label,
  multiline,
  testId,
}: {
  value: string;
  display?: string;
  label?: string;
  multiline?: boolean;
  testId?: string;
}) {
  const [copied, setCopied] = useState(false);
  const copy = () =>
    void navigator.clipboard?.writeText(value).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1200);
    });
  return (
    <div className="bg-muted/50 flex items-start gap-1 rounded-md border px-2 py-1">
      {multiline ? (
        <pre className="min-w-0 flex-1 overflow-x-auto py-0.5 font-mono text-[11px] leading-4">{display ?? value}</pre>
      ) : (
        <code className="min-w-0 flex-1 truncate py-0.5 font-mono text-[11px]">{display ?? value}</code>
      )}
      <Button variant="ghost" size="icon-xs" aria-label={label ?? "Copy"} onClick={copy} className="-mr-1" data-testid={testId}>
        {copied ? <CircleCheck className="text-[var(--s-2xx)]" /> : <Copy />}
      </Button>
    </div>
  );
}

/** A numbered list of steps. */
export function Steps({ children }: { children: ReactNode }) {
  return <ol className="flex list-decimal flex-col gap-1.5 pl-4 marker:text-[var(--muted-foreground)]">{children}</ol>;
}

/** A setting path, e.g. Settings › Wi-Fi. */
export function Path({ children }: { children: ReactNode }) {
  return <span className="text-foreground font-medium">{children}</span>;
}

export function Section({ title, children, className }: { title?: ReactNode; children: ReactNode; className?: string }) {
  return (
    <section className={cn("flex flex-col gap-2", className)}>
      {title && <h3 className="text-foreground text-[13px] font-semibold">{title}</h3>}
      {children}
    </section>
  );
}

/** The setup page's QR code and URL, or why there is none yet. */
export function QrCard({ access, device }: { access: DeviceAccess_Serialize | undefined; device: string }) {
  if (!access?.enabled || !access.setup_url || !access.qr_svg) {
    return (
      <div className="bg-muted/40 text-muted-foreground flex items-center gap-2 rounded-md border border-dashed p-3">
        <Wifi className="size-4 shrink-0" />
        <span>
          Turn on <span className="text-foreground font-medium">Allow devices on your network</span> above so your{" "}
          {device} can reach this Mac. The QR code appears here.
        </span>
      </div>
    );
  }
  return (
    <div className="flex items-center gap-3 rounded-md border p-2.5" data-testid="device-qr">
      <img
        src={svgDataUrl(access.qr_svg)}
        alt={`QR code for ${access.setup_url}`}
        className="size-[132px] shrink-0 rounded-sm bg-white"
        draggable={false}
      />
      <div className="flex min-w-0 flex-col gap-1.5">
        <div className="text-muted-foreground">
          Scan with the {device}&apos;s camera, or open this in its browser once its proxy is set:
        </div>
        <CopyField value={access.setup_url} label="Copy setup URL" />
        <div className="text-muted-foreground">
          Through the proxy, <code className="font-mono">http://proxy.mate/</code> works too.
        </div>
      </div>
    </div>
  );
}
