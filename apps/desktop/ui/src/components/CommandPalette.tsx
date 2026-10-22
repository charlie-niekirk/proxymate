import {
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandShortcut,
} from "@/components/ui/command";
import { COMMANDS, shortcutLabel, type AppCommand, type CommandGroup as Group } from "@/lib/commands";
import { useUi } from "@/store/ui";

const GROUPS: Group[] = ["Flows", "Capture", "Navigate", "Windows", "Settings"];

/** ⌘K: every command, searchable, with its shortcut. */
export function CommandPalette() {
  const open = useUi((s) => s.paletteOpen);
  const setOpen = useUi((s) => s.setPaletteOpen);
  // Which commands apply depends on the selection and the tab, as of opening.
  const commands = open ? COMMANDS.filter((c) => c.enabled?.() !== false) : [];
  const run = (command: AppCommand) => {
    setOpen(false);
    // After the dialog has closed and handed focus back.
    setTimeout(() => void command.run(), 0);
  };
  return (
    <CommandDialog
      open={open}
      onOpenChange={setOpen}
      title="Command palette"
      description="Run a command"
      className="sm:max-w-[520px]"
    >
      <PaletteBody commands={commands} onRun={run} />
    </CommandDialog>
  );
}

function PaletteBody({ commands, onRun }: { commands: AppCommand[]; onRun: (c: AppCommand) => void }) {
  return (
    <>
      <CommandInput placeholder="Type a command…" data-testid="palette-input" />
      <CommandList data-testid="palette">
        <CommandEmpty>No matching command</CommandEmpty>
        {GROUPS.map((group) => {
          const items = commands.filter((c) => c.group === group);
          if (items.length === 0) return null;
          return (
            <CommandGroup key={group} heading={group}>
              {items.map((c) => (
                <CommandItem key={c.id} value={c.title} keywords={c.keywords} onSelect={() => onRun(c)} data-testid={`palette-${c.id}`}>
                  {c.title}
                  {c.key && <CommandShortcut>{shortcutLabel(c.key)}</CommandShortcut>}
                </CommandItem>
              ))}
            </CommandGroup>
          );
        })}
      </CommandList>
    </>
  );
}
