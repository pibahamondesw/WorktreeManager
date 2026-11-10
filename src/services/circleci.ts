import { invoke } from "@tauri-apps/api/core";

export const circleciTokenSaved = () => invoke<boolean>("circleci_token_saved");

/** `null` removes the saved token. */
export const saveCircleciToken = (value: string | null) =>
  invoke<void>("circleci_token_save", { value });
