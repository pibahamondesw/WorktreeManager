import { describe, it, expect } from "vitest";
import {
  parseQuery,
  parsePaletteInput,
  scopeValues,
  sessionValues,
  withScope,
  withoutScope,
  withActiveSession,
  withoutActiveSession,
} from "./query";

describe("parseQuery", () => {
  it("collects free-text terms lowercased", () => {
    expect(parseQuery("WOR-49 Search").terms).toEqual(["wor-49", "search"]);
  });

  it("keeps quoted phrases as one term", () => {
    expect(parseQuery('"quick search" fix').terms).toEqual(["quick search", "fix"]);
  });

  it("parses field filters and splits OR values", () => {
    expect(parseQuery("in:web|api").filters).toEqual([{ field: "in", values: ["web", "api"] }]);
    expect(parseQuery("session:active").filters).toEqual([
      { field: "session", values: ["active"] },
    ]);
  });

  it("parses negated terms and filters", () => {
    const parsed = parseQuery("-fix -in:web");
    expect(parsed.negTerms).toEqual(["fix"]);
    expect(parsed.negFilters).toEqual([{ field: "in", values: ["web"] }]);
    expect(parsed.terms).toEqual([]);
    expect(parsed.filters).toEqual([]);
  });

  it("treats unknown keys as free text", () => {
    const parsed = parseQuery("foo:bar");
    expect(parsed.filters).toEqual([]);
    expect(parsed.terms).toEqual(["foo:bar"]);
  });

  it("ignores a bare dash and empty filter values", () => {
    expect(parseQuery("-").terms).toEqual(["-"]);
    expect(parseQuery("in:").terms).toEqual(["in:"]);
  });

  it("returns empty parts for blank input", () => {
    expect(parseQuery("   ")).toEqual({
      terms: [],
      negTerms: [],
      filters: [],
      negFilters: [],
    });
  });
});

describe("active session filter", () => {
  it("reads and toggles the active-session filter without changing the rest of the query", () => {
    expect(sessionValues(parseQuery("session:active repo:web"))).toEqual(["active"]);
    expect(withActiveSession("repo:web")).toBe("session:active repo:web");
    expect(withoutActiveSession("repo:web session:active branch:main")).toBe(
      "repo:web branch:main"
    );
  });
});

describe("scopeValues", () => {
  it("returns the in: values", () => {
    expect(scopeValues(parseQuery("in:web|api thing"))).toEqual(["web", "api"]);
  });

  it("is empty without an in: filter", () => {
    expect(scopeValues(parseQuery("repo:web"))).toEqual([]);
  });
});

describe("withScope / withoutScope", () => {
  it("scopes an empty query with a trailing space", () => {
    expect(withScope("", "Web")).toBe("in:Web ");
  });

  it("quotes workspace names with spaces", () => {
    expect(withScope("", "My Workspace")).toBe('in:"My Workspace" ');
    expect(scopeValues(parseQuery(withScope("", "My Workspace")))).toEqual(["my workspace"]);
  });

  it("keeps the rest of the query and replaces an existing scope", () => {
    expect(withScope("wor-49", "Web")).toBe("in:Web wor-49");
    expect(withScope("in:api wor-49", "Web")).toBe("in:Web wor-49");
    expect(withScope('in:"Old One" wor-49', "Web")).toBe("in:Web wor-49");
  });

  it("strips scope tokens anywhere, negated or quoted", () => {
    expect(withoutScope("in:web wor-49")).toBe("wor-49");
    expect(withoutScope("wor-49 -in:web")).toBe("wor-49");
    expect(withoutScope('wor-49 in:"My Workspace" fix')).toBe("wor-49 fix");
    expect(withoutScope("in:web")).toBe("");
  });

  it("leaves other filters alone", () => {
    expect(withoutScope("repo:web in:api")).toBe("repo:web");
  });
});

describe("parsePaletteInput", () => {
  it("treats a leading > as commands-only", () => {
    expect(parsePaletteInput(">theme")).toEqual({ commandsOnly: true, query: "theme" });
    expect(parsePaletteInput("  > theme indigo")).toEqual({
      commandsOnly: true,
      query: "theme indigo",
    });
  });

  it("leaves task search queries alone", () => {
    expect(parsePaletteInput("wor-67")).toEqual({ commandsOnly: false, query: "wor-67" });
    expect(parsePaletteInput("")).toEqual({ commandsOnly: false, query: "" });
  });
});
