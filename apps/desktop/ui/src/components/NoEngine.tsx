import { NO_ENGINE } from "@/lib/backend";

/** What a release build shows when it is opened outside the app (there is no engine to talk to). */
export function NoEngine() {
  return (
    <div className="bg-background text-muted-foreground flex h-screen items-center justify-center text-sm" role="alert">
      {NO_ENGINE}
    </div>
  );
}
