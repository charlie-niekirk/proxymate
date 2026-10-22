import { CircleAlert, CircleCheck, Info, LoaderCircle, TriangleAlert } from "lucide-react";
import { useSyncExternalStore } from "react";
import { Toaster as Sonner, type ToasterProps } from "sonner";

/** The theme `<html>` currently shows (it follows Settings › Appearance). */
function useDocumentTheme(): "light" | "dark" {
  return useSyncExternalStore(
    (onChange) => {
      const observer = new MutationObserver(onChange);
      observer.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
      return () => observer.disconnect();
    },
    () => (document.documentElement.dataset.theme === "light" ? "light" : "dark"),
  );
}

/** Toasts, styled with the app's tokens (shadcn's sonner without next-themes). */
function Toaster(props: ToasterProps) {
  const theme = useDocumentTheme();
  return (
    <Sonner
      theme={theme}
      className="toaster group"
      position="bottom-right"
      offset={36}
      icons={{
        success: <CircleCheck className="size-4 text-[var(--s-2xx)]" />,
        info: <Info className="size-4" />,
        warning: <TriangleAlert className="size-4 text-[var(--s-4xx)]" />,
        error: <CircleAlert className="size-4 text-[var(--s-failed)]" />,
        loading: <LoaderCircle className="size-4 animate-spin" />,
      }}
      toastOptions={{ classNames: { toast: "!text-xs !py-2.5", description: "!text-muted-foreground" } }}
      style={
        {
          "--normal-bg": "var(--popover)",
          "--normal-text": "var(--popover-foreground)",
          "--normal-border": "var(--border)",
          "--border-radius": "var(--radius)",
        } as React.CSSProperties
      }
      {...props}
    />
  );
}

export { Toaster };
