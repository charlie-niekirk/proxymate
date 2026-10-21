import { useQuery } from "@tanstack/react-query";
import { CircleAlert, CircleCheck, Copy, LoaderCircle, RefreshCw, ShieldCheck, Trash2 } from "lucide-react";
import { useState, type ReactNode } from "react";

import type { TrustStatus_Serialize } from "@/bindings";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { useTrustStatus } from "@/hooks/useTrustStatus";
import { copyTerminalEnv, installCaTrust, uninstallCaTrust } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { caInfoKey, queryClient, trustStatusKey } from "@/lib/query";
import { cn } from "@/lib/utils";
import { useUi } from "@/store/ui";

type Busy = "install" | "uninstall" | "check" | null;

/**
 * Certificate wizard: installs the Proxymate CA into the login keychain as
 * a trusted root (macOS asks for a password or Touch ID), proves it with a
 * real handshake through the proxy, and can remove it again.
 */
export function CertWizard() {
  const open = useUi((s) => s.certWizardOpen);
  const setOpen = useUi((s) => s.setCertWizardOpen);
  const { data: trust } = useTrustStatus();
  const { data: ca } = useQuery({ queryKey: caInfoKey, queryFn: () => backend.getCaInfo() });
  const [busy, setBusy] = useState<Busy>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const run = (kind: Exclude<Busy, null>, action: () => Promise<unknown>) => {
    setBusy(kind);
    setError(null);
    action()
      .catch((e: unknown) => {
        const message = e instanceof Error ? e.message : String(e);
        setError(message === "cancelled" ? "Cancelled. Nothing was changed." : message);
      })
      .finally(() => setBusy(null));
  };
  const recheck = () => run("check", () => queryClient.refetchQueries({ queryKey: trustStatusKey }));
  const installed = trust?.keychain === "trusted";
  const verified = trust?.verified ?? false;

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogContent
        className="sm:max-w-[480px]"
        data-testid="cert-wizard"
        // Don't ring the first button (Copy terminal env) on open.
        onOpenAutoFocus={(e) => e.preventDefault()}
      >
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <ShieldCheck className="size-4" />
            Trust the Proxymate certificate
          </DialogTitle>
          <DialogDescription>
            To decrypt HTTPS, apps on this Mac must trust Proxymate&apos;s root CA. It is added to your login keychain
            for your user only; macOS asks for your password or Touch ID.
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-1.5 rounded-md border p-2.5">
          <Check ok={installed} pending={!trust} label="Trusted in the login keychain">
            {trust?.keychain === "denied" && "Set to “Never Trust” in Keychain Access."}
            {trust?.keychain === "unsupported" && "Not available on this platform yet."}
          </Check>
          <Check ok={verified} pending={!trust} label="Verified with a real HTTPS handshake">
            {trust && !verified && <TrustDetail trust={trust} />}
          </Check>
          {ca && <div className="text-muted-foreground pl-6 font-mono text-[11px] break-all">{ca.common_name}</div>}
        </div>

        {verified ? (
          <p className="text-[var(--s-2xx)]">
            Safari, Chrome and other apps that use the macOS trust store will now accept Proxymate&apos;s
            certificates.
          </p>
        ) : (
          <p className="text-muted-foreground">
            Click <span className="text-foreground font-medium">Install &amp; Trust</span>, then approve the macOS
            prompt. Proxymate then checks the result with a real handshake.
          </p>
        )}

        <div className="bg-muted/40 flex flex-col gap-1.5 rounded-md p-2.5">
          <div className="font-medium">Firefox and command-line tools</div>
          <p className="text-muted-foreground">
            Firefox keeps its own certificate store. Open <code className="font-mono">about:config</code>, set{" "}
            <code className="font-mono">security.enterprise_roots.enabled</code> to{" "}
            <code className="font-mono">true</code> and restart Firefox so it trusts the keychain.
          </p>
          <p className="text-muted-foreground">
            Node, Python, curl and friends read CA files instead. Copy the terminal environment and paste it into a
            shell.
          </p>
          <div>
            <Button
              variant="outline"
              size="xs"
              onClick={() =>
                void copyTerminalEnv().then(() => {
                  setCopied(true);
                  setTimeout(() => setCopied(false), 1500);
                })
              }
            >
              <Copy />
              {copied ? "Copied" : "Copy terminal env"}
            </Button>
          </div>
        </div>

        {error && <div className="text-[var(--s-failed)]">{error}</div>}

        <DialogFooter className="items-center gap-2 sm:justify-between">
          <Button
            variant="ghost"
            size="sm"
            className="text-muted-foreground"
            disabled={!installed || busy !== null}
            onClick={() => run("uninstall", uninstallCaTrust)}
            data-testid="uninstall-ca"
          >
            {busy === "uninstall" ? <LoaderCircle className="animate-spin" /> : <Trash2 />}
            Remove from keychain
          </Button>
          <div className="flex gap-2">
            <Button variant="outline" size="sm" disabled={busy !== null} onClick={recheck}>
              <RefreshCw className={cn(busy === "check" && "animate-spin")} />
              Check again
            </Button>
            {verified ? (
              <Button size="sm" onClick={() => setOpen(false)}>
                Done
              </Button>
            ) : (
              <Button
                size="sm"
                disabled={busy !== null || trust?.keychain === "unsupported"}
                onClick={() => run("install", installCaTrust)}
                data-testid="install-ca"
              >
                {busy === "install" && <LoaderCircle className="animate-spin" />}
                {busy === "install" ? "Waiting for macOS…" : "Install & Trust"}
              </Button>
            )}
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function Check({ ok, pending, label, children }: { ok: boolean; pending: boolean; label: string; children?: ReactNode }) {
  const Icon = pending ? LoaderCircle : ok ? CircleCheck : CircleAlert;
  return (
    <div className="flex items-start gap-2">
      <Icon
        className={cn(
          "mt-px size-4 shrink-0",
          pending && "text-muted-foreground animate-spin",
          !pending && (ok ? "text-[var(--s-2xx)]" : "text-[var(--s-4xx)]"),
        )}
        data-ok={ok}
      />
      <div className="min-w-0">
        <div className="font-medium">{label}</div>
        {children && <div className="text-muted-foreground">{children}</div>}
      </div>
    </div>
  );
}

function TrustDetail({ trust }: { trust: TrustStatus_Serialize }) {
  if (!trust.detail) return null;
  // The verifier's own wording, minus the rustls error enum.
  const text = trust.detail.replace(/ \(.*\)$/, "");
  return <>{text.charAt(0).toUpperCase() + text.slice(1)}.</>;
}
