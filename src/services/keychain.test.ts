import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { loadSecrets, saveAndVerifySecrets } from "./keychain";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
beforeEach(() => vi.mocked(invoke).mockReset());

describe("loadSecrets", () => {
  it("distinguishes an empty Keychain from denied access", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(null);
    expect(await loadSecrets()).toEqual({ secrets: { setup: null, workspaces: {} }, error: null });
    vi.mocked(invoke).mockRejectedValueOnce("Failed to read from keychain: authorization denied");
    expect(await loadSecrets()).toEqual({
      secrets: null,
      error: "Failed to read from keychain: authorization denied",
    });
  });

  it.each(['{"setup":"lin_secret",', '{"setup":"lin_secret","workspaces":null}'])(
    "does not include stored credentials in a decoding error",
    async (raw) => {
      vi.mocked(invoke).mockResolvedValueOnce(raw);
      const result = await loadSecrets();
      expect(result.secrets).toBeNull();
      expect(result.error).toBeTruthy();
      expect(result.error).not.toContain("lin_secret");
    }
  );

  it("does not verify a save when read-back is denied", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(undefined)
      .mockRejectedValueOnce("authorization denied");
    expect(await saveAndVerifySecrets({ setup: "lin_test", workspaces: {} })).toBe(false);
  });
});
