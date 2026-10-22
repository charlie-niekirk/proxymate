import { useEffect } from "react";

import { exportHar, openDocument, refreshDocuments, saveSession } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { useUi } from "@/store/ui";

/**
 * Runs the File menu's items (sent by the native menu as `pm-menu` events)
 * against the active tab: Save Session saves all of its flows, Export HAR
 * what the table shows. Also loads the documents already open (a reload).
 */
export function useAppMenu() {
  useEffect(() => {
    void refreshDocuments();
    let stop: (() => void) | null = null;
    let cancelled = false;
    void backend
      .onMenu((id) => {
        switch (id) {
          case "compose":
            useUi.getState().openCompose();
            break;
          case "open":
            void openDocument(null, false);
            break;
          case "import-har":
            void openDocument(null, true);
            break;
          case "save-session":
            void saveSession();
            break;
          case "export-har":
            void exportHar();
            break;
        }
      })
      .then((unlisten) => {
        if (cancelled) unlisten();
        else stop = unlisten;
      });
    return () => {
      cancelled = true;
      stop?.();
    };
  }, []);
}
