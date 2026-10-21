import { DetailPane } from "@/components/DetailPane";
import { FilterBar } from "@/components/FilterBar";
import { FlowTable } from "@/components/FlowTable/FlowTable";
import { Footer } from "@/components/Footer";
import { Sidebar } from "@/components/Sidebar";
import { Toolbar } from "@/components/Toolbar";
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from "@/components/ui/resizable";

/**
 * Proxyman-style layout: full-height vibrant sidebar | toolbar, filter row,
 * flow table over the request/response inspector, footer.
 */
export function AppShell() {
  return (
    <ResizablePanelGroup orientation="horizontal" className="h-full">
      <ResizablePanel defaultSize={220} minSize={160} maxSize={360}>
        <Sidebar />
      </ResizablePanel>
      <ResizableHandle className="bg-sidebar-border" />
      <ResizablePanel minSize={600}>
        <main className="bg-background flex h-full min-w-0 flex-col">
          <Toolbar />
          <FilterBar />
          <ResizablePanelGroup orientation="vertical" className="min-h-0 flex-1">
            <ResizablePanel defaultSize="58" minSize={120}>
              <FlowTable />
            </ResizablePanel>
            <ResizableHandle />
            <ResizablePanel defaultSize="42" minSize={120}>
              <DetailPane />
            </ResizablePanel>
          </ResizablePanelGroup>
          <Footer />
        </main>
      </ResizablePanel>
    </ResizablePanelGroup>
  );
}
