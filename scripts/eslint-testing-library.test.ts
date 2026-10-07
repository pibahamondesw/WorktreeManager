import { ESLint } from "eslint";
import tseslint from "typescript-eslint";
import { describe, expect, it } from "vitest";

const eslint = new ESLint({
  overrideConfig: tseslint.configs.disableTypeChecked,
});
const reactTestPath = "src/components/task/TaskView.test.tsx";
const importQueries = 'import { screen, waitFor, fireEvent } from "@testing-library/react";';

async function testingLibraryMessages(code: string, filePath = reactTestPath) {
  const results = await eslint.lintText(`${importQueries}\n${code}`, { filePath });
  expect(results.every((result) => result.fatalErrorCount === 0)).toBe(true);
  return results.flatMap((result) =>
    result.messages.filter((message) => message.ruleId?.startsWith("testing-library/"))
  );
}

const cases = [
  {
    rule: "await-async-queries",
    invalid: 'screen.findByText("Ready");',
    valid: 'async function test() { await screen.findByText("Ready"); }',
  },
  {
    rule: "await-async-utils",
    invalid: 'waitFor(() => expect(screen.getByText("Ready")).toHaveTextContent("Done"));',
    valid:
      'async function test() { await waitFor(() => expect(screen.getByText("Ready")).toHaveTextContent("Done")); }',
  },
  {
    rule: "no-await-sync-queries",
    invalid: 'async function test() { await screen.getByText("Ready"); }',
    valid: 'screen.getByText("Ready");',
  },
  {
    rule: "no-wait-for-side-effects",
    invalid:
      'async function test() { await waitFor(() => { fireEvent.click(screen.getByRole("button")); }); }',
    valid:
      'async function test() { fireEvent.click(screen.getByRole("button")); await waitFor(() => expect(screen.getByRole("combobox")).toHaveValue("ready")); }',
  },
  {
    rule: "prefer-find-by",
    invalid:
      'async function test() { await waitFor(() => expect(screen.getByText("Ready")).toBeInTheDocument()); }',
    valid:
      'async function test() { expect(await screen.findByText("Ready")).toBeInTheDocument(); }',
  },
];

describe("Testing Library ESLint configuration", () => {
  it.each(cases)("rejects $rule violations", async ({ rule, invalid }) => {
    expect(await testingLibraryMessages(invalid)).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ ruleId: `testing-library/${rule}`, severity: 2 }),
      ])
    );
  });

  it.each(cases)("accepts correct $rule usage", async ({ valid }) => {
    expect(await testingLibraryMessages(valid)).toEqual([]);
  });

  it.each(["src/services/operations.test.ts", "src/components/task/TaskView.tsx"])(
    "excludes %s from Testing Library rules",
    async (filePath) => {
      const config = (await eslint.calculateConfigForFile(filePath)) as {
        rules: Record<string, unknown>;
      };
      expect(
        Object.keys(config.rules).filter((rule) => rule.startsWith("testing-library/"))
      ).toEqual([]);
      expect(await testingLibraryMessages(cases[0].invalid, filePath)).toEqual([]);
    }
  );
});
