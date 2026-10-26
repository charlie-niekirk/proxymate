import { useMemo, useState } from "react";

import { KvMenu } from "@/components/Inspector/KvTable";
import { BodyItems } from "@/components/menus/body";
import { useMenuTarget } from "@/components/menus/useMenuTarget";
import { ContextMenu, ContextMenuTrigger } from "@/components/ui/context-menu";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";

import { CodeView } from "./CodeView";
import { Bar, Centered } from "./parts";
import { useText } from "./util";

/** URL-encoded form fields as a key/value table, repeated keys kept in order. */
export function FormView({ bytes }: { bytes: Uint8Array }) {
  const text = useText(bytes);
  const fields = useMemo(() => (text === null ? [] : [...new URLSearchParams(text.trim())]), [text]);
  const [mode, setMode] = useState<"table" | "raw">("table");
  const menu = useMenuTarget((key) => (fields[Number(key)] ? Number(key) : null));
  if (text === null) return <Centered>Binary body: switch to Hex to see its bytes.</Centered>;
  return (
    <div className="flex h-full flex-col">
      <Bar>
        <ToggleGroup type="single" size="sm" value={mode} onValueChange={(v) => v && setMode(v as "table" | "raw")}>
          <ToggleGroupItem value="table">Fields</ToggleGroupItem>
          <ToggleGroupItem value="raw">Raw</ToggleGroupItem>
        </ToggleGroup>
        <span className="text-muted-foreground ml-auto">
          {fields.length} field{fields.length === 1 ? "" : "s"}
        </span>
      </Bar>
      {mode === "raw" ? (
        <div className="min-h-0 flex-1">
          <CodeView text={text} language="text" wrap />
        </div>
      ) : (
        <div className="min-h-0 flex-1 overflow-auto">
          <ContextMenu>
            <ContextMenuTrigger asChild>
              <table className="w-full font-mono text-xs" data-selectable data-testid="form-table" onContextMenu={menu.onContextMenu}>
                <thead className="text-muted-foreground bg-[var(--pane-header)] text-left text-[10.5px]">
                  <tr>
                    <th className="w-8 px-2 py-1 text-right font-normal">#</th>
                    <th className="w-1/3 px-2 py-1 font-normal">Name</th>
                    <th className="px-2 py-1 font-normal">Value</th>
                  </tr>
                </thead>
                <tbody>
                  {fields.map(([k, v], i) => (
                    <tr key={i} className="border-border/50 border-b" data-menu-key={i}>
                      <td className="text-muted-foreground px-2 py-1 text-right align-top">{i + 1}</td>
                      <td className="px-2 py-1 align-top font-medium break-all text-[var(--syntax-key)]">{k}</td>
                      <td className="px-2 py-1 break-all whitespace-pre-wrap">
                        {v === "" ? <span className="text-muted-foreground italic">empty</span> : v}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </ContextMenuTrigger>
            <KvMenu rows={fields} kind="form" index={menu.target} selection={menu.selection}>
              <BodyItems />
            </KvMenu>
          </ContextMenu>
        </div>
      )}
    </div>
  );
}
