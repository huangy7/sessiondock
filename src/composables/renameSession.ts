import { invoke } from "@tauri-apps/api/core";
import type { SessionIdentity } from "../types/session";

export function renameSession(identity: SessionIdentity, newName: string) {
  return invoke("rename_session", {
    cliId: identity.cliId,
    filePath: identity.filePath,
    newName,
  });
}
