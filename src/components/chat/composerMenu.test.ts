import { describe, expect, it } from "vitest";
import {
  filterCommands,
  insertMention,
  localCommand,
  menuTrigger,
  promptMemory,
} from "./composerMenu";
import { ChatControls, CommandOption, EMPTY_CONTROLS } from "../../services/chat";

const command = (name: string, description: string, action: CommandOption["action"]) => ({
  name,
  description,
  action,
});

const controls: ChatControls = {
  ...EMPTY_CONTROLS,
  models: [
    { id: "opus", label: "Opus", efforts: ["low", "high"] },
    { id: "haiku", label: "Haiku", efforts: [] },
  ],
  model: "opus",
  commands: [
    command("model", "Choose the model", { kind: "model" }),
    command("effort", "Choose the effort", { kind: "effort" }),
    command("plan", "Plan first", { kind: "mode", mode: "plan" }),
    command("compact", "Free up context", { kind: "compact" }),
    command("review", "Review a pull request", { kind: "insert", text: "/review " }),
    command("simplify", "Clean up the model layer", { kind: "insert", text: "/simplify " }),
  ],
};

describe("menuTrigger", () => {
  it("opens commands only for a leading slash word and files for an @ token", () => {
    expect(menuTrigger("/mo", 3)).toEqual({ kind: "slash", query: "mo" });
    expect(menuTrigger("/model x", 8)).toBeNull();
    expect(menuTrigger("see /mo", 7)).toBeNull();
    expect(menuTrigger("fix @src/ch", 11)).toEqual({ kind: "mention", query: "src/ch", start: 4 });
    expect(menuTrigger("mail a@b", 8)).toBeNull();
  });
});

describe("filterCommands", () => {
  it("ranks name prefixes before substrings and descriptions", () => {
    const names = filterCommands(controls.commands, "mod").map((c) => c.name);
    expect(names).toEqual(["model", "simplify"]);
    expect(filterCommands(controls.commands, "")).toHaveLength(controls.commands.length);
  });
});

describe("insertMention", () => {
  it("replaces the @ query with the path and moves the caret past it", () => {
    expect(insertMention("fix @ch now", 4, 7, "src/chat.ts")).toEqual({
      text: "fix @src/chat.ts  now",
      caret: 17,
    });
  });
});

describe("localCommand", () => {
  it("handles UI commands locally and passes the rest through", () => {
    expect(localCommand("/model", controls)).toEqual({ kind: "picker", picker: "model" });
    expect(localCommand("/model haiku", controls)).toEqual({
      kind: "configure",
      setting: "model",
      value: "haiku",
    });
    expect(localCommand("/model nope", controls)).toEqual({ kind: "picker", picker: "model" });
    expect(localCommand("/effort high", controls)).toEqual({
      kind: "configure",
      setting: "effort",
      value: "high",
    });
    expect(localCommand("/plan", controls)).toEqual({
      kind: "configure",
      setting: "mode",
      value: "plan",
    });
    expect(localCommand("/compact", controls)).toEqual({ kind: "compact" });
    expect(localCommand("/review 12", controls)).toBeNull();
    expect(localCommand("/unknown", controls)).toBeNull();
    expect(localCommand("please /model", controls)).toBeNull();
  });
});

describe("promptMemory", () => {
  it("keeps recent prompts without duplicates and drops empty drafts", () => {
    promptMemory.remember("k", "a");
    promptMemory.remember("k", "b");
    promptMemory.remember("k", "a");
    expect(promptMemory.history("k")).toEqual(["b", "a"]);
    promptMemory.saveDraft("k", "half");
    expect(promptMemory.draft("k")).toBe("half");
    promptMemory.saveDraft("k", "");
    expect(promptMemory.draft("k")).toBe("");
  });
});
