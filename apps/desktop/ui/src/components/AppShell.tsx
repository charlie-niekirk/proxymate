import { BreakpointEditor } from "@/components/Breakpoints/BreakpointEditor";
import { CertWizard } from "@/components/CertWizard";
import { CommandPalette } from "@/components/CommandPalette";
import { ComposeDialog } from "@/components/Compose/ComposeDialog";
import { DetailPane } from "@/components/DetailPane";
import { DeviceAllowPrompt } from "@/components/DeviceAllowPrompt";
import { DeviceSetupSheet } from "@/components/DeviceSetup/DeviceSetupSheet";
import { DocumentTabs } from "@/components/DocumentTabs";
import { FilterBar } from "@/components/FilterBar";
import { FlowTable } from "@/components/FlowTable/FlowTable";
import { Footer } from "@/components/Footer";
import { RewriteDialog } from "@/components/Rewrite/RewriteDialog";
import { RulesEditor } from "@/components/Rules/RulesEditor";
import { ScriptsWindow } from "@/components/Scripts/ScriptsWindow";
import { SettingsDialog } from "@/components/Settings/SettingsDialog";
import { Sidebar } from "@/components/Sidebar";
import { Toolbar } from "@/components/Toolbar";
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from "@/components/ui/resizable";
import { Toaster } from "@/components/ui/sonner";
import { useAppMenu } from "@/hooks/useAppMenu";
import { useNativeMenuPolicy } from "@/hooks/useNativeMenuPolicy";
import { useShortcuts } from "@/hooks/useShortcuts";
import { startStorage } from "@/store/storage";
import { useEffect } from "react";

/**
 * Proxyman-style layout: full-height vibrant sidebar | toolbar, filter row,
 * flow table over the request/response inspector, footer.
 */
export function AppShell() {
  useAppMenu();
  useShortcuts();
  useNativeMenuPolicy();
  useEffect(startStorage, []);
  return (
    <>
    <ResizablePanelGroup orientation="horizontal" className="h-full">
      <ResizablePanel defaultSize={220} minSize={160} maxSize={360}>
        <Sidebar />
      </ResizablePanel>
      <ResizableHandle className="bg-sidebar-border" />
      <ResizablePanel minSize={600}>
        <main className="bg-background flex h-full min-w-0 flex-col">
          <Toolbar />
          <DocumentTabs />
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
    <CertWizard />
    <RulesEditor />
    <RewriteDialog />
    <ScriptsWindow />
    <ComposeDialog />
    <BreakpointEditor />
    <DeviceSetupSheet />
    <SettingsDialog />
    <DeviceAllowPrompt />
    <CommandPalette />
    <Toaster />
    </>
  );
}
