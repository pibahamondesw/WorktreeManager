import { describe, expect, it } from "vitest";
import {
  filterCommands,
  insertCommand,
  insertMention,
  localCommand,
  menuTrigger,
  promptMemory,
  reviewTarget,
  matchSlashCommands,
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
    { id: "opus", label: "Opus", efforts: ["low", "high"], images: true },
    { id: "haiku", label: "Haiku", efforts: [], images: true },
  ],
  model: "opus",
  commands: [
    command("model", "Choose the model", { kind: "model" }),
    command("effort", "Choose the effort", { kind: "effort" }),
    command("plan", "Plan first", { kind: "mode", mode: "plan" }),
    command("compact", "Free up context", { kind: "compact" }),
    command("status", "Show plan usage", { kind: "status" }),
    command("review", "Review a pull request", { kind: "insert", text: "/review " }),
    command("simplify", "Clean up the model layer", { kind: "insert", text: "/simplify " }),
  ],
};

describe("menuTrigger", () => {
  it("opens commands for a slash word and files for an @ token", () => {
    expect(menuTrigger("/mo", 3)).toEqual({ kind: "slash", query: "mo", start: 0 });
    expect(menuTrigger("/model x", 8)).toBeNull();
    expect(menuTrigger("see /mo", 7)).toEqual({ kind: "slash", query: "mo", start: 4 });
    expect(menuTrigger("and/or", 6)).toBeNull();
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

describe("matchSlashCommands", () => {
  const names = (start: number, query: string) =>
    matchSlashCommands(controls.commands, start, query).map((c) => c.name);

  it("offers every command at the start and only inserting ones mid-message", () => {
    expect(names(0, "")).toEqual(controls.commands.map((c) => c.name));
    expect(names(4, "")).toEqual(["review", "simplify"]);
  });

  it("matches descriptions only at the start of the message", () => {
    expect(names(0, "layer")).toEqual(["simplify"]);
    expect(names(4, "layer")).toEqual([]);
    expect(names(4, "simp")).toEqual(["simplify"]);
  });
});

describe("insertCommand", () => {
  it("replaces the slash query in place and moves the caret past it", () => {
    expect(insertCommand("use /sim then go", 4, 8, "/simplify ")).toEqual({
      text: "use /simplify then go",
      caret: 13,
    });
    expect(insertCommand("use /sim", 4, 8, "/simplify ")).toEqual({
      text: "use /simplify ",
      caret: 14,
    });
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
    expect(localCommand("/status", controls)).toEqual({ kind: "status" });
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

describe("native review", () => {
  const codex: ChatControls = {
    ...EMPTY_CONTROLS,
    commands: [command("review", "Review changes", { kind: "review" })],
  };

  it("parses each review target and opens the picker otherwise", () => {
    expect(localCommand("/review", codex)).toEqual({ kind: "reviewPicker" });
    expect(localCommand("/review uncommitted", codex)).toEqual({
      kind: "review",
      target: { type: "uncommittedChanges" },
    });
    expect(localCommand("/review branch main", codex)).toEqual({
      kind: "review",
      target: { type: "baseBranch", branch: "main" },
    });
    expect(localCommand("/review commit abc123", codex)).toEqual({
      kind: "review",
      target: { type: "commit", sha: "abc123", title: null },
    });
    expect(localCommand("/review custom check the auth flow", codex)).toEqual({
      kind: "review",
      target: { type: "custom", instructions: "check the auth flow" },
    });
    expect(localCommand("/review branch", codex)).toEqual({ kind: "reviewPicker" });
    expect(reviewTarget("commit a b")).toBeNull();
  });

  it("keeps sending /review as text when the agent has no native review", () => {
    expect(localCommand("/review 12", controls)).toBeNull();
  });
});
