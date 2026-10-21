import { QueryClientProvider } from "@tanstack/react-query";

import { AppShell } from "@/components/AppShell";
import { TooltipProvider } from "@/components/ui/tooltip";
import { queryClient } from "@/lib/query";

export default function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <TooltipProvider delayDuration={400}>
        <AppShell />
      </TooltipProvider>
    </QueryClientProvider>
  );
}
