import { AppWindow } from "lucide-react";
import { memo, useState } from "react";

import { appIconUrl } from "@/lib/appIcon";
import { cn } from "@/lib/utils";
import { useSidebar } from "@/store/sidebar";

/**
 * A client app's icon, served by the engine; a window glyph for tools
 * without a bundle (curl, node) and outside the app.
 */
export const AppIcon = memo(function AppIcon({ name, className }: { name: string; className?: string }) {
  const hasIcon = useSidebar((s) => s.icons.has(name));
  const [failed, setFailed] = useState(false);
  const url = hasIcon && !failed ? appIconUrl(name) : null;
  if (url) {
    return <img src={url} alt="" draggable={false} onError={() => setFailed(true)} className={cn("size-3.5 shrink-0", className)} />;
  }
  return <AppWindow className={cn("size-3.5 shrink-0 opacity-70", className)} />;
});
