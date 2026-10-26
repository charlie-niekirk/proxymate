import { useEffect } from "react";

import { isEditable } from "@/lib/commands";

/**
 * Where the WebView's own right-click menu may show. Our Radix menus
 * prevent it wherever they open; of what's left, text fields, editors and
 * selected text keep it (Copy, Paste, Look Up…), and everything else gets
 * nothing rather than "Reload". Debug builds keep it for Inspect Element.
 */
export function useNativeMenuPolicy() {
  useEffect(() => {
    if (!import.meta.env.PROD) return;
    // On window, so it runs after React's root listener: a menu that opened has already prevented the event.
    const onContextMenu = (e: MouseEvent) => {
      if (e.defaultPrevented || isEditable(e.target)) return;
      if (window.getSelection()?.isCollapsed === false) return;
      e.preventDefault();
    };
    window.addEventListener("contextmenu", onContextMenu);
    return () => window.removeEventListener("contextmenu", onContextMenu);
  }, []);
}
