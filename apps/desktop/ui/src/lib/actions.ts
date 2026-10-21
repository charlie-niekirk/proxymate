import { backend } from "@/lib/backend";
import { caInfoKey, proxyStatusKey, queryClient, sslSettingsKey } from "@/lib/query";
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

/** Stops decrypting `host`; its next CONNECTs are tunnelled. */
export async function addSslBypass(host: string) {
  queryClient.setQueryData(sslSettingsKey, await backend.addSslBypass(host));
}

/** Accepts (or stops accepting) `host`'s upstream certificate unverified. */
export async function setInsecureUpstream(host: string, allow: boolean) {
  queryClient.setQueryData(sslSettingsKey, await backend.setInsecureUpstream(host, allow));
}

/** Replaces the root CA; clients must trust the new one. */
export async function regenerateCa() {
  queryClient.setQueryData(caInfoKey, await backend.regenerateCa());
}
