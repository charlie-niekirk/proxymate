import { useQuery } from "@tanstack/react-query";

import { CopyMenu } from "@/components/menus/items";
import { Button } from "@/components/ui/button";
import { backend } from "@/lib/backend";
import { proxyStatusKey } from "@/lib/query";
import { useUi } from "@/store/ui";

/** The view couldn't be opened: say why, rather than waiting for traffic that can't arrive. */
export function ViewError({ error, retry }: { error: string; retry: () => void }) {
  return (
    <div
      className="text-muted-foreground absolute inset-0 top-[22px] flex flex-col items-center justify-center gap-2 text-sm"
      role="alert"
      data-testid="view-error"
    >
      <span>Couldn't open the flow list</span>
      <span className="max-w-md text-center text-xs" data-selectable>
        {error}
      </span>
      <Button size="sm" variant="outline" onClick={retry}>
        Try again
      </Button>
    </div>
  );
}

export function EmptyState() {
  const total = useUi((s) => s.stats.total);
  const deep = useUi((s) => s.deep);
  const { data: status } = useQuery({ queryKey: proxyStatusKey, queryFn: () => backend.getProxyStatus() });
  const proxy = status?.listening ?? "127.0.0.1:9090";
  return (
    <div className="text-muted-foreground absolute inset-0 top-[22px] flex flex-col items-center justify-center gap-2 text-sm">
      {deep?.running ? (
        "Searching headers and bodies…"
      ) : total > 0 ? (
        "No flows match the current filter"
      ) : (
        <>
          <span>Waiting for traffic</span>
          <CopyMenu items={[{ label: "Copy command", text: `curl -x http://${proxy} http://example.com/`, what: "command" }]}>
            <code className="bg-muted rounded px-2 py-1 font-mono text-xs" data-selectable data-testid="empty-curl">
              curl -x http://{proxy} http://example.com/
            </code>
          </CopyMenu>
        </>
      )}
    </div>
  );
}
