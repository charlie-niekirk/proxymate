import { backend } from "@/lib/backend";
import { proxyStatusKey, queryClient } from "@/lib/query";
import { useUi } from "@/store/ui";

/** Clears every flow. The view's structural delta then empties the table. */
export async function clearFlows() {
  useUi.getState().select(null);
  await backend.clearFlows();
  queryClient.removeQueries({ queryKey: ["body"] });
  queryClient.removeQueries({ queryKey: ["flow"] });
}

/** Pauses or resumes recording; traffic is forwarded either way. */
export async function setRecording(on: boolean) {
  queryClient.setQueryData(proxyStatusKey, await backend.setRecording(on));
}
