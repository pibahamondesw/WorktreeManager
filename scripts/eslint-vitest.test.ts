import { ESLint, type Linter } from "eslint";
import tseslint from "typescript-eslint";
import { describe, expect, it } from "vitest";

const eslint = new ESLint({ overrideConfig: tseslint.configs.disableTypeChecked });

async function lint(source: string, filePath = "src/utils.test.ts") {
  const [result] = await eslint.lintText(source, { filePath });
  return result;
}

describe("Vitest ESLint rules", () => {
  it.each(["src/utils.test.ts", "src/utils.ts"])(
    "preserves typed production rules for %s outside the fixture engine",
    async (filePath) => {
      const productionESLint = new ESLint();
      const config = (await productionESLint.calculateConfigForFile(filePath)) as Linter.Config;
      expect(config.rules?.["@typescript-eslint/no-floating-promises"]).toEqual([2]);
      if (filePath.endsWith(".test.ts")) {
        expect(config.rules?.["vitest/no-focused-tests"]).toEqual(expect.arrayContaining([2]));
        expect(config.rules?.["vitest/valid-expect"]).toEqual(expect.arrayContaining([2]));
      } else {
        expect(config.rules?.["vitest/no-focused-tests"]).toBeUndefined();
        expect(config.rules?.["vitest/valid-expect"]).toBeUndefined();
      }
    }
  );

  it.each(["it", "test", "describe"])(
    "rejects focused %s blocks until corrected",
    async (testFunction) => {
      const focused = `import { ${testFunction} } from "vitest"; ${testFunction}.only("example", () => {});`;
      const result = await lint(focused);
      expect(result.messages).toEqual([
        expect.objectContaining({
          ruleId: "vitest/no-focused-tests",
          severity: 2,
        }),
      ]);
      expect((await lint(focused.replace(".only", ""))).messages).toEqual([]);
    }
  );

  it.each(["expect()", "expect(1)", "expect(1).toBe"])(
    "rejects incomplete assertion %s until corrected",
    async (assertion) => {
      const result = await lint(
        `import { expect, it } from "vitest"; it("example", () => { ${assertion}; });`
      );
      expect(result.messages).toContainEqual(
        expect.objectContaining({ ruleId: "vitest/valid-expect", severity: 2 })
      );
      expect(
        (
          await lint(
            'import { expect, it } from "vitest"; it("example", () => { expect(1).toBe(1); });'
          )
        ).messages
      ).toEqual([]);
    }
  );

  it.each(["src/utils.test.ts", "src/hooks/useAgentSessions.test.tsx", "scripts/knip.test.ts"])(
    "applies both rules to %s",
    async (filePath) => {
      const result = await lint(
        'import { expect, it } from "vitest"; it.only("example", () => { expect(1); });',
        filePath
      );
      expect(result.messages.map(({ ruleId }) => ruleId)).toEqual([
        "vitest/no-focused-tests",
        "vitest/valid-expect",
      ]);
    }
  );

  it.each(["src/utils.ts", "scripts/bump-version.mjs", "vitest.config.ts"])(
    "leaves %s outside the Vitest rules",
    async (filePath) => {
      const result = await lint(
        'import { expect, it } from "vitest"; it.only("example", () => { expect(1); });',
        filePath
      );
      expect(result.messages).toEqual([]);
    }
  );

  it("accepts Vitest 4 async assertions, custom messages and expect.poll", async () => {
    const result = await lint(`
      import { expect, it } from "vitest";
      it("example", async () => {
        expect(1, "custom message").toBe(1);
        await expect(Promise.resolve(1)).resolves.toBe(1);
        await expect(Promise.reject(new Error("failure"))).rejects.toThrow("failure");
        await expect.poll(() => 1).toBe(1);
      });
    `);
    expect(result.messages).toEqual([]);
  });
});
