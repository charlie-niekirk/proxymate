import { Smartphone } from "lucide-react";
import { useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { backend } from "@/lib/backend";
import { startDevices, useDevices } from "@/store/devices";

/** How many connections wait, and how long until they are refused. */
function Countdown({ waiting, until }: { waiting: number; until: number | null }) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 250);
    return () => clearInterval(id);
  }, []);
  const left = until === null ? null : Math.max(0, Math.ceil((until - now) / 1000));
  return (
    <div className="text-muted-foreground flex items-center gap-2" data-testid="device-countdown">
      <span className="min-w-0 flex-1">
        {waiting} connection{waiting === 1 ? "" : "s"} waiting
        {left !== null && (left > 0 ? ` · refused in ${left} s` : " · being refused")}
      </span>
      {left !== null && (
        <span className="bg-muted h-1 w-20 overflow-hidden rounded-full">
          <span className="block h-full bg-[var(--s-4xx)] transition-[width]" style={{ width: `${(left / 30) * 100}%` }} />
        </span>
      )}
    </div>
  );
}

/**
 * "Allow 10.0.0.23?" for the first LAN client nobody has answered for yet.
 * Its connections wait up to 30 s (counted down here); closing the prompt
 * leaves it pending: the toolbar shows it waiting, Settings › Clients lists
 * it, and its connections are refused once they time out.
 */
export function DeviceAllowPrompt() {
  useEffect(startDevices, []);
  const devices = useDevices((s) => s.snapshot.devices);
  const dismissed = useDevices((s) => s.dismissed);
  const dismissDevice = useDevices((s) => s.dismiss);
  const [nickname, setNickname] = useState("");
  const [error, setError] = useState<string | null>(null);
  const pending = devices.find((d) => d.decision === "pending" && !dismissed.has(d.ip));

  const decide = (allow: boolean) => {
    if (!pending) return;
    setError(null);
    backend
      .decideDevice(pending.ip, allow, allow ? nickname.trim() || null : null)
      .then(() => setNickname(""))
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)));
  };
  const dismiss = () => {
    if (pending) dismissDevice(pending.ip);
    setNickname("");
  };

  return (
    <Dialog open={!!pending} onOpenChange={(open) => !open && dismiss()}>
      <DialogContent className="sm:max-w-[400px]" data-testid="device-allow-prompt">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Smartphone className="size-4" />
            Allow {pending?.ip}?
          </DialogTitle>
          <DialogDescription>
            A device on your network wants to use Proxymate as its proxy. Only allow devices you are setting up: an
            allowed device&apos;s traffic is recorded here.
          </DialogDescription>
        </DialogHeader>
        <form
          className="flex flex-col gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            decide(true);
          }}
        >
          <label className="text-muted-foreground" htmlFor="device-nickname">
            Nickname (shown in the Client column)
          </label>
          <Input
            id="device-nickname"
            autoFocus
            placeholder="e.g. Charlie's iPhone"
            value={nickname}
            onChange={(e) => setNickname(e.target.value)}
            data-testid="device-nickname"
          />
          {!!pending?.waiting && <Countdown waiting={pending.waiting} until={pending.held_until_ms ?? null} />}
          {error && <div className="text-[var(--s-failed)]">{error}</div>}
          <DialogFooter className="mt-1">
            <Button type="button" variant="ghost" size="sm" onClick={() => decide(false)} data-testid="device-deny">
              Deny
            </Button>
            <Button type="submit" size="sm" data-testid="device-allow">
              Allow
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
