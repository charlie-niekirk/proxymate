import { useQuery } from "@tanstack/react-query";
import { Copy, RefreshCw, ShieldCheck } from "lucide-react";
import { useState, type ReactNode } from "react";

import { Button } from "@/components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { regenerateCa } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { caInfoKey, proxyStatusKey } from "@/lib/query";

/**
 * Toolbar popover for the root CA: where its PEM is (for `curl --cacert`
 * and friends) and "Regenerate". Installing it into the keychain is the
 * certificate wizard (M4).
 */
export function CertificateMenu() {
  const { data: ca } = useQuery({ queryKey: caInfoKey, queryFn: () => backend.getCaInfo() });
  const { data: status } = useQuery({ queryKey: proxyStatusKey, queryFn: () => backend.getProxyStatus() });
  const [confirming, setConfirming] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const proxy = status?.listening ? `http://${status.listening}` : "http://127.0.0.1:9090";
  const curl = ca ? `curl -x ${proxy} --cacert "${ca.cert_path}" https://example.com/` : "";

  const onRegenerate = () => {
    if (!confirming) {
      setConfirming(true);
      return;
    }
    setConfirming(false);
    regenerateCa().catch((e: unknown) => setError(String(e)));
  };

  return (
    <Popover onOpenChange={() => setConfirming(false)}>
      <PopoverTrigger asChild>
        <Button variant="ghost" size="icon" aria-label="Certificate" data-testid="certificate-menu">
          <ShieldCheck />
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-96 p-0 text-xs">
        <div className="border-b px-3 py-2">
          <div className="text-sm font-semibold">HTTPS decryption</div>
          <div className="text-muted-foreground">Clients must trust this root CA to accept Proxymate&apos;s certificates.</div>
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
        <div className="flex items-center justify-between gap-2 border-t px-3 py-2">
          <span className="text-muted-foreground">Keychain install arrives with the certificate wizard.</span>
          <Button
            variant={confirming ? "destructive" : "outline"}
            size="xs"
            onClick={onRegenerate}
            data-testid="regenerate-ca"
          >
            <RefreshCw />
            {confirming ? "Replace CA?" : "Regenerate"}
          </Button>
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
        onClick={() => void navigator.clipboard?.writeText(text)}
      >
        <Copy />
      </Button>
    </div>
  );
}
