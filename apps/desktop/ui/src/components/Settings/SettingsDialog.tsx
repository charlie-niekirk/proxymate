import {
  Bot,
  Boxes,
  Braces,
  HardDrive,
  Laptop,
  Network,
  Palette,
  RefreshCw,
  Settings,
  ShieldCheck,
  SlidersHorizontal,
  Wifi,
  type LucideIcon,
} from "lucide-react";

import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { cn } from "@/lib/utils";
import { useUi, type SettingsPage } from "@/store/ui";

import { AdvancedPage } from "./AdvancedPage";
import { AppearancePage } from "./AppearancePage";
import { ClientsPage } from "./ClientsPage";
import { McpPage } from "./McpPage";
import { MockingPage } from "./MockingPage";
import { NetworkPage } from "./NetworkPage";
import { ProtobufPage } from "./ProtobufPage";
import { ProxyPage } from "./ProxyPage";
import { SslPage } from "./SslPage";
import { StoragePage } from "./StoragePage";
import { UpdatesPage } from "./UpdatesPage";

const PAGES: { id: SettingsPage; label: string; icon: LucideIcon; page: () => React.ReactNode }[] = [
  { id: "proxy", label: "Proxy", icon: Wifi, page: () => <ProxyPage /> },
  { id: "network", label: "Network", icon: Network, page: () => <NetworkPage /> },
  { id: "ssl", label: "SSL Proxying", icon: ShieldCheck, page: () => <SslPage /> },
  { id: "clients", label: "Clients", icon: Laptop, page: () => <ClientsPage /> },
  { id: "storage", label: "Storage", icon: HardDrive, page: () => <StoragePage /> },
  { id: "protobuf", label: "Protobuf", icon: Braces, page: () => <ProtobufPage /> },
  { id: "mocking", label: "Mocking", icon: Boxes, page: () => <MockingPage /> },
  { id: "appearance", label: "Appearance", icon: Palette, page: () => <AppearancePage /> },
  { id: "advanced", label: "Advanced", icon: SlidersHorizontal, page: () => <AdvancedPage /> },
  { id: "mcp", label: "MCP", icon: Bot, page: () => <McpPage /> },
  { id: "updates", label: "Updates", icon: RefreshCw, page: () => <UpdatesPage /> },
];

/** Settings: a page list beside the chosen page. */
export function SettingsDialog() {
  const { open, page } = useUi((s) => s.settings);
  const openSettings = useUi((s) => s.openSettings);
  const close = useUi((s) => s.closeSettings);
  const current = PAGES.find((p) => p.id === page) ?? PAGES[0]!;
  return (
    <Dialog open={open} onOpenChange={(o) => !o && close()}>
      <DialogContent
        className="flex h-[80vh] flex-col gap-0 p-0 sm:max-w-[880px]"
        onOpenAutoFocus={(e) => e.preventDefault()}
        data-testid="settings"
      >
        <DialogHeader className="border-b px-4 py-2.5">
          <DialogTitle className="flex items-center gap-2 text-sm">
            <Settings className="size-4" />
            Settings
          </DialogTitle>
          <DialogDescription className="sr-only">Proxymate settings</DialogDescription>
        </DialogHeader>
        <div className="flex min-h-0 flex-1">
          <nav className="bg-muted/30 flex w-[168px] shrink-0 flex-col gap-0.5 border-r p-2" aria-label="Settings pages">
            {PAGES.map((p) => (
              <button
                key={p.id}
                onClick={() => openSettings(p.id)}
                aria-current={page === p.id ? "page" : undefined}
                className={cn(
                  "flex h-7 items-center gap-2 rounded-md px-2 text-left text-xs",
                  page === p.id ? "bg-accent text-accent-foreground font-medium" : "hover:bg-accent/60",
                )}
                data-testid={`settings-page-${p.id}`}
              >
                <p.icon className="size-3.5" />
                {p.label}
              </button>
            ))}
          </nav>
          {/* A plain scroller: Radix's viewport lets wide code blocks widen the page. */}
          <div className="min-h-0 min-w-0 flex-1 overflow-y-auto">{open && current.page()}</div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
