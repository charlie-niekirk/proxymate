import { useQuery } from "@tanstack/react-query";
import { CircleAlert, CircleCheck, Copy, RefreshCw, ShieldAlert, ShieldCheck } from "lucide-react";
import { useState, type ReactNode } from "react";

import { Button } from "@/components/ui/button";
import { useTrustStatus } from "@/hooks/useTrustStatus";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Switch } from "@/components/ui/switch";
import { regenerateCa } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { copy } from "@/lib/clipboard";
import { caInfoKey, proxyStatusKey } from "@/lib/query";
import { useUi } from "@/store/ui";

/**
 * Toolbar popover for the root CA: whether the OS trusts it (the wizard
 * installs it), where its PEM is (for `curl --cacert` and friends) and
 * "Regenerate".
 */
export function CertificateMenu() {
  const { data: ca } = useQuery({ queryKey: caInfoKey, queryFn: () => backend.getCaInfo() });
  const { data: status } = useQuery({ queryKey: proxyStatusKey, queryFn: () => backend.getProxyStatus() });
  const { data: trust } = useTrustStatus();
  const openWizard = useUi((s) => s.setCertWizardOpen);
  const [open, setOpen] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // Removing the old CA's trust is the default when this Mac trusts it.
  const [untrust, setUntrust] = useState<boolean | null>(null);
  const removeOld = untrust ?? trust?.keychain === "trusted";
  const proxy = status?.listening ? `http://${status.listening}` : "http://127.0.0.1:9090";
  const curl = ca ? `curl -x ${proxy} --cacert "${ca.cert_path}" https://example.com/` : "";

  const onRegenerate = () => {
    if (!confirming) {
      setConfirming(true);
      return;
    }
    setConfirming(false);
    setError(null);
    setBusy(true);
    regenerateCa(removeOld)
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)))
      .finally(() => setBusy(false));
  };

  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        setConfirming(false);
      }}
    >
      <PopoverTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          aria-label="Certificate"
          data-testid="certificate-menu"
          className={trust?.verified ? "text-[var(--s-2xx)]" : undefined}
        >
          {trust && !trust.verified ? <ShieldAlert /> : <ShieldCheck />}
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-96 p-0 text-xs">
        <div className="border-b px-3 py-2">
          <div className="text-sm font-semibold">HTTPS decryption</div>
          <div className="text-muted-foreground">Clients must trust this root CA to accept Proxymate&apos;s certificates.</div>
        </div>
        <div className="flex items-center justify-between gap-2 border-b px-3 py-2">
          <span className="flex items-center gap-1.5">
            {trust?.verified ? (
              <CircleCheck className="size-3.5 text-[var(--s-2xx)]" />
            ) : (
              <CircleAlert className="size-3.5 text-[var(--s-4xx)]" />
            )}
            {trust?.verified ? "Trusted by this Mac" : "Not trusted by this Mac yet"}
          </span>
          <Button
            variant={trust?.verified ? "outline" : "default"}
            size="xs"
            onClick={() => {
              setOpen(false);
              openWizard(true);
            }}
            data-testid="open-cert-wizard"
          >
            {trust?.verified ? "Manage…" : "Install & Trust…"}
          </Button>
        </div>
        {ca && (
          <div className="flex flex-col gap-2 px-3 py-2" data-selectable>
            <Field label="Root CA">{ca.common_name}</Field>
            <Field label="Valid until">{ca.not_after.slice(0, 10)}</Field>
            <Field label="SHA-256">
              <span className="font-mono break-all">{ca.sha256.match(/.{1,2}/g)?.join(":")}</span>
            </Field>
            <Field label="PEM file">
              <CopyRow text={ca.cert_path} />
            </Field>
            <Field label="Try it">
              <CopyRow text={curl} />
            </Field>
          </div>
        )}
        <div className="flex flex-col gap-2 border-t px-3 py-2">
          <div className="flex items-center justify-between gap-2">
            <span className="text-muted-foreground">
              {confirming ? "Every device must trust the new CA again." : "A new CA must be trusted again."}
            </span>
            <Button
              variant={confirming ? "destructive" : "outline"}
              size="xs"
              disabled={busy}
              onClick={onRegenerate}
              data-testid="regenerate-ca"
            >
              <RefreshCw className={busy ? "animate-spin" : undefined} />
              {confirming ? "Replace CA" : "Regenerate…"}
            </Button>
          </div>
          {confirming && (
            <label className="flex items-center gap-2" data-testid="untrust-old-ca">
              <Switch checked={removeOld} onCheckedChange={setUntrust} />
              <span>
                Remove trust for the old CA from this Mac
                <span className="text-muted-foreground block">macOS asks for your password; other devices keep their copy.</span>
              </span>
            </label>
          )}
        </div>
        {error && <div className="border-t px-3 py-2 text-[var(--s-failed)]">{error}</div>}
      </PopoverContent>
    </Popover>
  );
}

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="grid grid-cols-[72px_1fr] gap-2">
      <span className="text-muted-foreground">{label}</span>
      <div className="min-w-0">{children}</div>
    </div>
  );
}

function CopyRow({ text }: { text: string }) {
  return (
    <div className="flex items-start gap-1">
      <code className="bg-muted min-w-0 flex-1 rounded-sm px-1 py-0.5 font-mono text-[11px] break-all">{text}</code>
      <Button
        variant="ghost"
        size="icon-xs"
        aria-label="Copy"
        onClick={() => void copy(text)}
      >
        <Copy />
      </Button>
    </div>
  );
}
