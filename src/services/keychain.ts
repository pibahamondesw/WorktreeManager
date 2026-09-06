import { invoke } from "@tauri-apps/api/core";
import { SecretBundle, EMPTY_SECRETS } from "../utils";

export type SecretReadResult =
  | { secrets: SecretBundle; error: null }
  | { secrets: null; error: string };

export async function loadSecrets(): Promise<SecretReadResult> {
  let raw: string | null;
  try {
    raw = await invoke<string | null>("keychain_get");
  } catch (error) {
    return { secrets: null, error: String(error) };
  }
  if (raw == null) return { secrets: EMPTY_SECRETS, error: null };
  try {
    const value: unknown = JSON.parse(raw);
    if (
      !value ||
      typeof value !== "object" ||
      !("setup" in value) ||
      !("workspaces" in value) ||
      (value.setup !== null && typeof value.setup !== "string") ||
      !value.workspaces ||
      typeof value.workspaces !== "object" ||
      Array.isArray(value.workspaces) ||
      !Object.values(value.workspaces).every((key) => typeof key === "string")
    ) {
      return { secrets: null, error: "The Linear credentials in Keychain have an invalid format." };
    }
    return { secrets: value as SecretBundle, error: null };
  } catch {
    return { secrets: null, error: "The Linear credentials in Keychain could not be decoded." };
  }
}

export async function saveSecrets(secrets: SecretBundle): Promise<void> {
  await invoke("keychain_set", { value: JSON.stringify(secrets) });
}

/**
 * Write the keys and read them back, so `store.json` is only stripped once the keychain
 * demonstrably holds them. Returns false when the keychain could not be reached, leaving
 * the caller to keep its plaintext copy and retry on the next launch.
 */
export async function saveAndVerifySecrets(secrets: SecretBundle): Promise<boolean> {
  try {
    await saveSecrets(secrets);
  } catch {
    return false;
  }
  const readBack = await loadSecrets();
  return readBack.secrets != null && JSON.stringify(readBack.secrets) === JSON.stringify(secrets);
}
