import { describe, expect, it } from "vitest";
import automationFields from "../../src-tauri/src/commands/automations/definition/fields.rs?raw";
import {
  AUTOMATION_VARIABLES,
  TASK_VARIABLES,
  effectiveFirstPromptTemplate,
  firstPromptProblem,
  firstPromptVariables,
  renderFirstPrompt,
  seededMessage,
} from "./firstPrompt";

describe("renderFirstPrompt", () => {
  it("fills variables and writes doubled braces literally", () => {
    expect(
      renderFirstPrompt("{issue.identifier}: {issue.title} {{json}}", {
        "issue.identifier": "ENG-1",
        "issue.title": "Fix login",
      })
    ).toBe("ENG-1: Fix login {json}");
  });

  it("leaves out lines whose variables are all empty and keeps plain lines", () => {
    const template = "Issue: {issue.identifier} {issue.title}\nBranch: {task.branch}\n\nThanks";
    expect(renderFirstPrompt(template, { "task.branch": "feature" })).toBe(
      "Branch: feature\n\nThanks"
    );
  });

  it("keeps a line when one of its variables has a value", () => {
    expect(renderFirstPrompt("{issue.identifier} {issue.title}", { "issue.title": "Fix" })).toBe(
      "Fix"
    );
  });

  it("renders multi-line values whole", () => {
    expect(renderFirstPrompt("Repos:\n{repos}", { repos: "- api: /a\n- web: /w" })).toBe(
      "Repos:\n- api: /a\n- web: /w"
    );
  });

  it("renders empty when nothing has a value", () => {
    expect(renderFirstPrompt("{issue.title}\n{issue.url}", {})).toBe("");
  });
});

describe("firstPromptProblem", () => {
  it.each([
    "",
    "{task.branch} {{literal}}",
    "{automation} {run.id} {item.data.labels}",
    "{issue.description}\n{note.path}",
  ])("accepts %j", (template) => {
    expect(firstPromptProblem(template)).toBeNull();
  });

  it.each([
    ["{nope}", "The template uses the unknown variable {nope}."],
    ["{item.data.}", "The template uses the unknown variable {item.data.}."],
    ["{task.branch", "The template has an unclosed {; write {{ for a literal brace."],
    ["oops}", "The template has an unmatched }; write }} for a literal brace."],
  ])("rejects %j", (template, problem) => {
    expect(firstPromptProblem(template)).toBe(problem);
  });
});

it("lists the variables a template names", () => {
  expect(firstPromptVariables("{a} {{b}} {issue.description}")).toEqual(["a", "issue.description"]);
});

it("prefers a workspace template that is not blank", () => {
  expect(effectiveFirstPromptTemplate("global", "workspace")).toBe("workspace");
  expect(effectiveFirstPromptTemplate("global", "  ")).toBe("global");
  expect(effectiveFirstPromptTemplate(undefined, undefined)).toBe("");
});

it("puts the seed ahead of the message", () => {
  expect(seededMessage("Context", "Do it")).toBe("Context\n\nDo it");
  expect(seededMessage(null, "Do it")).toBe("Do it");
});

it("knows every variable an automation template has", () => {
  const list = /const TEMPLATE_VARIABLES: &\[&str\] = &\[([^\]]*)\]/.exec(automationFields)![1];
  const automation = [...list.matchAll(/"([^"]+)"/g)].map((match) => match[1]);
  expect([...AUTOMATION_VARIABLES, ...TASK_VARIABLES]).toEqual(expect.arrayContaining(automation));
});
