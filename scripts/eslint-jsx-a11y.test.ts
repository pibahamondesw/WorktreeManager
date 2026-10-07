import { ESLint } from "eslint";
import { describe, expect, it } from "vitest";

const eslint = new ESLint({
  overrideConfig: {
    languageOptions: { parserOptions: { projectService: false } },
    rules: {
      "@typescript-eslint/no-floating-promises": "off",
      "@typescript-eslint/no-misused-promises": "off",
      "@typescript-eslint/no-unsafe-argument": "off",
      "@typescript-eslint/no-unsafe-assignment": "off",
      "@typescript-eslint/no-unsafe-call": "off",
      "@typescript-eslint/no-unsafe-member-access": "off",
      "@typescript-eslint/no-unsafe-return": "off",
    },
  },
});

async function accessibilityMessages(code: string, filePath = "src/AccessibilityFixture.tsx") {
  const [result] = await eslint.lintText(code, { filePath });
  expect(result.messages.some((message) => message.fatal)).toBe(false);
  return result.messages.filter((message) => message.ruleId?.startsWith("jsx-a11y/"));
}

const invalidFixtures = [
  ["label-has-associated-control", "<label>Name</label>"],
  ["aria-props", '<div aria-labl="Name" />'],
  ["aria-proptypes", '<div aria-hidden="sometimes" />'],
  ["aria-role", '<div role="unknown-role" />'],
  ["aria-unsupported-elements", '<meta aria-label="Name" />'],
  ["interactive-supports-focus", '<span role="button" onClick={() => {}}>Create PR</span>'],
];

const validFixtures = [
  '<label htmlFor="name">Name</label>',
  "<label>Name<input /></label>",
  "<label>Name<Input /></label>",
  '<div aria-label="Name" aria-hidden="true" role="status" />',
  "<button onClick={() => {}}>Create PR</button>",
  "<Button onClick={() => {}}>Create PR</Button>",
];

describe("JSX accessibility configuration", () => {
  it.each(invalidFixtures)("rejects %s violations", async (rule, code) => {
    expect(await accessibilityMessages(code)).toEqual([
      expect.objectContaining({ ruleId: `jsx-a11y/${rule}`, severity: 2 }),
    ]);
  });

  it.each(validFixtures)("accepts %s", async (code) => {
    expect(await accessibilityMessages(code)).toEqual([]);
  });

  it("keeps synthetic test markup outside production accessibility rules", async () => {
    expect(
      await accessibilityMessages("<label>Name</label>", "src/AccessibilityFixture.test.tsx")
    ).toEqual([]);
  });
});
