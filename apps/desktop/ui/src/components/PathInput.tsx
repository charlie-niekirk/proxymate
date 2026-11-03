import { FolderOpen } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { reportError } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { cn } from "@/lib/utils";

/**
 * A path field with "Choose…", which asks with the OS file (or folder)
 * dialog. The text stays editable, for pasting a path or tweaking one.
 */
export function PathInput({
  value,
  onChange,
  title,
  folder = false,
  extensions = [],
  placeholder,
  className,
  testId,
}: {
  value: string;
  onChange: (path: string) => void;
  /** The dialog's title, e.g. "Map Local file". */
  title: string;
  folder?: boolean;
  /** Without the dot; empty offers every file. */
  extensions?: string[];
  placeholder?: string;
  className?: string;
  testId?: string;
}) {
  const choose = async () => {
    try {
      const path = await backend.choosePath(title, folder, extensions, value || null);
      if (path) onChange(path);
    } catch (e) {
      reportError(folder ? "Couldn't choose the folder" : "Couldn't choose the file", e);
    }
  };
  return (
    <div className="flex min-w-0 flex-1 items-center gap-1.5">
      <Input
        value={value}
        placeholder={placeholder}
        spellCheck={false}
        onChange={(e) => onChange(e.target.value)}
        className={cn("min-w-0 flex-1 font-mono", className)}
        data-testid={testId}
      />
      <Button
        type="button"
        variant="outline"
        size="xs"
        onClick={() => void choose()}
        title={folder ? "Choose a folder" : "Choose a file"}
        data-testid={testId && `${testId}-choose`}
      >
        <FolderOpen />
        Choose…
      </Button>
    </div>
  );
}
