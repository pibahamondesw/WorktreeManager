import { invoke } from "@tauri-apps/api/core";
import { SecretBundle, EMPTY_SECRETS } from "../utils";

/**
 * Read the stored keys. `null` means the keychain could not be read — a denied
 * authorization prompt, or a blob we cannot parse — which is deliberately distinct
 * from an empty bundle: a caller about to drop its own plaintext copy must not treat
 * "could not look" as "nothing was there".
 */
export async function loadSecrets(): Promise<SecretBundle | null> {
  try {
    const raw = await invoke<string | null>("keychain_get");
    if (raw == null) return EMPTY_SECRETS;
    return JSON.parse(raw) as SecretBundle;
  } catch {
    return null;
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
  return readBack != null && JSON.stringify(readBack) === JSON.stringify(secrets);
}
