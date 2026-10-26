import { describe, it, expect, vi } from "vitest";
import { extractPrsFromAttachments, GqlAttachmentNode, LinearService } from "./linear";

describe("extractPrsFromAttachments", () => {
  it("returns an empty array for empty attachments", () => {
    expect(extractPrsFromAttachments([])).toEqual([]);
  });

  it("returns an empty array when no attachment has a PR URL", () => {
    const attachments: GqlAttachmentNode[] = [
      { url: "https://example.com/some-page", title: "Docs" },
      { url: "https://github.com/org/repo/issues/42", title: "Issue" },
    ];
    expect(extractPrsFromAttachments(attachments)).toEqual([]);
  });

  it("extracts PR info from a GitHub PR URL", () => {
    const attachments: GqlAttachmentNode[] = [
      { url: "https://github.com/org/repo/pull/123", title: "Fix bug" },
    ];
    expect(extractPrsFromAttachments(attachments)).toEqual([
      {
        url: "https://github.com/org/repo/pull/123",
        title: "Fix bug",
        state: "open",
        number: 123,
        repoSlug: "org/repo",
      },
    ]);
  });

  it("uses a fallback title when attachment has no title", () => {
    const attachments: GqlAttachmentNode[] = [{ url: "https://github.com/org/repo/pull/7" }];
    expect(extractPrsFromAttachments(attachments)[0]?.title).toBe("PR #7");
  });

  it("detects merged state from metadata object", () => {
    const attachments: GqlAttachmentNode[] = [
      {
        url: "https://github.com/org/repo/pull/10",
        title: "Feature",
        metadata: { status: "Merged" },
      },
    ];
    expect(extractPrsFromAttachments(attachments)[0]?.state).toBe("merged");
  });

  it("detects merged state from JSON string metadata", () => {
    const attachments: GqlAttachmentNode[] = [
      {
        url: "https://github.com/org/repo/pull/10",
        title: "Feature",
        metadata: JSON.stringify({ state: "MERGED" }),
      },
    ];
    expect(extractPrsFromAttachments(attachments)[0]?.state).toBe("merged");
  });

  it.each(["null", "42", '"merged"', "[]", '{"status": 42}'])(
    "ignores non-status metadata: %s",
    (metadata) => {
      expect(
        extractPrsFromAttachments([{ url: "https://github.com/org/repo/pull/1", metadata }])[0]
          ?.state
      ).toBe("open");
    }
  );

  it("detects closed state from metadata", () => {
    const attachments: GqlAttachmentNode[] = [
      {
        url: "https://github.com/org/repo/pull/5",
        title: "Abandoned",
        metadata: { status: "Closed" },
      },
    ];
    expect(extractPrsFromAttachments(attachments)[0]?.state).toBe("closed");
  });

  it("falls back to subtitle for merged detection when metadata has no state", () => {
    const attachments: GqlAttachmentNode[] = [
      {
        url: "https://github.com/org/repo/pull/20",
        title: "Done",
        subtitle: "Merged by user",
      },
    ];
    expect(extractPrsFromAttachments(attachments)[0]?.state).toBe("merged");
  });

  it("falls back to subtitle for closed detection", () => {
    const attachments: GqlAttachmentNode[] = [
      {
        url: "https://github.com/org/repo/pull/21",
        title: "Nope",
        subtitle: "Closed without merge",
      },
    ];
    expect(extractPrsFromAttachments(attachments)[0]?.state).toBe("closed");
  });

  it.each([[{ status: "open", draft: true }], [JSON.stringify({ status: "Draft" })]])(
    "detects draft state from metadata %j",
    (metadata) => {
      expect(
        extractPrsFromAttachments([{ url: "https://github.com/org/repo/pull/1", metadata }])[0]
          ?.state
      ).toBe("draft");
    }
  );

  it("prefers merged over draft when both are present", () => {
    const metadata = { status: "merged", draft: true };
    expect(
      extractPrsFromAttachments([{ url: "https://github.com/org/repo/pull/1", metadata }])[0]?.state
    ).toBe("merged");
  });

  it("returns all PRs when multiple attachments have PR URLs", () => {
    const attachments: GqlAttachmentNode[] = [
      { url: "https://example.com/docs" },
      { url: "https://github.com/org/repo-a/pull/1", title: "First" },
      { url: "https://github.com/org/repo-b/pull/2", title: "Second" },
    ];
    const result = extractPrsFromAttachments(attachments);
    expect(result.map((p) => p.number)).toEqual([1, 2]);
    expect(result.map((p) => p.repoSlug)).toEqual(["org/repo-a", "org/repo-b"]);
  });

  it("dedupes attachments pointing at the same PR", () => {
    const attachments: GqlAttachmentNode[] = [
      { url: "https://github.com/org/repo/pull/1", title: "First" },
      { url: "https://github.com/Org/Repo/pull/1", title: "Duplicate" },
    ];
    expect(extractPrsFromAttachments(attachments)).toHaveLength(1);
  });

  it("skips attachments with no url", () => {
    const attachments: GqlAttachmentNode[] = [
      { title: "No URL" },
      { url: "https://github.com/org/repo/pull/99", title: "Has URL" },
    ];
    expect(extractPrsFromAttachments(attachments)[0]?.number).toBe(99);
  });
});

it("loads projects in bounded batches and preserves successful batches on failure", async () => {
  const service = new LinearService("test-key");
  const request = vi.fn(
    async (_query: string, variables: { filter: { id: { in: string[] } } }) => ({
      data: {
        issues: {
          nodes: variables.filter.id.in.map((id: string) => ({
            id,
            project: { id: "project", name: "API" },
            state: null,
            attachments: { nodes: [] },
          })),
        },
      },
    })
  );
  Object.defineProperty(service, "gql", { value: { rawRequest: request } });
  const ids = Array.from({ length: 101 }, (_, index) => `issue-${index}`);
  const result = await service.fetchIssueLinearInfoBatch(ids);
  expect(request.mock.calls.map((call) => call[1].filter.id.in.length)).toEqual([50, 50, 1]);
  expect(Object.keys(result)).toHaveLength(101);
  expect(result["issue-100"].project).toEqual({ id: "project", name: "API" });
  request.mockRejectedValueOnce(new Error("offline"));
  const partial = await service.fetchIssueLinearInfoBatch(ids);
  expect(Object.keys(partial)).toHaveLength(51);
  expect(partial["issue-0"]).toBeUndefined();
});

it("creates an assigned issue with nested data returned by the mutation", async () => {
  const service = new LinearService("test-key");
  const issue = {
    id: "issue",
    identifier: "WOR-1",
    title: "New thing",
    branchName: "me/wor-1-new-thing",
    description: "Details",
    priority: 0,
    updatedAt: "2026-10-04T00:00:00.000Z",
    project: { id: "project", name: "App" },
    state: { name: "Todo", type: "unstarted" },
  };
  const request = vi
    .fn()
    .mockResolvedValueOnce({ data: { viewer: { id: "viewer" } } })
    .mockResolvedValueOnce({ data: { issueCreate: { success: true, issue } } });
  Object.defineProperty(service, "gql", { value: { rawRequest: request } });
  await expect(
    service.createIssue({ teamId: "team", title: "New thing", description: "Details" })
  ).resolves.toEqual({
    id: "issue",
    identifier: "WOR-1",
    title: "New thing",
    branchName: "me/wor-1-new-thing",
    description: "Details",
    priority: 0,
    updatedAt: issue.updatedAt,
    projectId: "project",
    projectName: "App",
    stateName: "Todo",
    stateType: "unstarted",
  });
  expect(request).toHaveBeenCalledTimes(2);
  expect(request).toHaveBeenLastCalledWith(expect.stringContaining("mutation CreateIssue"), {
    input: { teamId: "team", title: "New thing", description: "Details", assigneeId: "viewer" },
  });
});

it.each([
  { success: false, issue: null },
  { success: true, issue: null },
])("rejects unsuccessful creation payloads: %j", async (payload) => {
  const service = new LinearService("test-key");
  const request = vi
    .fn()
    .mockResolvedValueOnce({ data: { viewer: { id: "viewer" } } })
    .mockResolvedValueOnce({ data: { issueCreate: payload } });
  Object.defineProperty(service, "gql", { value: { rawRequest: request } });
  await expect(service.createIssue({ teamId: "team", title: "New thing" })).rejects.toThrow(
    "Linear rejected the issue creation"
  );
});

describe("Linear issue suggestions", () => {
  const me = { isMe: { eq: true } };
  const filters = [
    { assignee: me },
    { creator: me },
    { project: { lead: me } },
    { project: { members: { some: me } } },
    { project: { creator: me } },
  ];
  const node = (id: string, priority = 0, updatedAt = "2026-10-01T00:00:00Z") => ({
    id,
    identifier: id,
    title: id,
    branchName: id,
    description: null,
    priority,
    updatedAt,
    project: { id: "project", name: "App" },
    state: { name: "Todo", type: "unstarted" },
  });
  const page = (nodes: ReturnType<typeof node>[], next: string | null = null) => ({
    nodes,
    pageInfo: { hasNextPage: next !== null, endCursor: next },
  });
  const setup = () => {
    const service = new LinearService("test-key");
    const request = vi.fn();
    Object.defineProperty(service, "gql", { value: { rawRequest: request } });
    return { service, request };
  };

  it("orders all five groups by relevance, then urgency, and keeps overlaps in the first group", async () => {
    const { service, request } = setup();
    for (const [index] of filters.entries()) {
      request.mockResolvedValueOnce({
        data: {
          issues: page([
            node(`${index}-none`),
            node(`${index}-low`, 4),
            node(`${index}-medium`, 3),
            node(`${index}-high`, 2),
            node(`${index}-urgent`, 1),
            node("overlap", 2),
          ]),
        },
      });
    }
    const issues = await service.fetchSuggestedIssues();
    expect(issues.map((issue) => issue.id)).toEqual(
      filters.flatMap((_, index) => [
        `${index}-urgent`,
        `${index}-high`,
        ...(index === 0 ? ["overlap"] : []),
        `${index}-medium`,
        `${index}-low`,
        `${index}-none`,
      ])
    );
    expect(issues[0]).toMatchObject({
      projectId: "project",
      projectName: "App",
      stateName: "Todo",
    });
    expect(request).toHaveBeenCalledTimes(5);
    filters.forEach((filter, index) => {
      expect(request).toHaveBeenNthCalledWith(
        index + 1,
        expect.stringContaining("query IssueSuggestions"),
        {
          filter: { ...filter, state: { type: { nin: ["completed", "canceled"] } } },
          first: 50,
          after: null,
        }
      );
    });
  });

  it("loads later pages before sorting and uses recency to break priority ties", async () => {
    const { service, request } = setup();
    request.mockImplementation(
      async (
        _query: string,
        { filter, after }: { filter: Record<string, unknown>; after: string | null }
      ) => ({
        data: {
          issues: filter.assignee
            ? after
              ? page([node("urgent", 1), node("recent", 2, "2026-10-20T00:00:00Z")])
              : page([node("old", 2)], "next")
            : page([]),
        },
      })
    );
    expect((await service.fetchSuggestedIssues()).map((issue) => issue.id)).toEqual([
      "urgent",
      "recent",
      "old",
    ]);
    expect(request).toHaveBeenCalledWith(
      expect.any(String),
      expect.objectContaining({ after: "next" })
    );
    expect(request).toHaveBeenCalledTimes(6);
  });

  it("falls back to issues in all viewer teams only when every personal group is empty", async () => {
    const { service, request } = setup();
    for (let index = 0; index < filters.length; index++)
      request.mockResolvedValueOnce({ data: { issues: page([]) } });
    request
      .mockResolvedValueOnce({
        data: { viewer: { teams: { ...page([], "next-team"), nodes: [{ id: "team-a" }] } } },
      })
      .mockResolvedValueOnce({
        data: { viewer: { teams: { ...page([]), nodes: [{ id: "team-b" }] } } },
      })
      .mockResolvedValueOnce({ data: { issues: page([node("low", 4), node("urgent", 1)]) } });
    expect((await service.fetchSuggestedIssues()).map((issue) => issue.id)).toEqual([
      "urgent",
      "low",
    ]);
    expect(request).toHaveBeenNthCalledWith(7, expect.stringContaining("query ViewerTeams"), {
      after: "next-team",
    });
    expect(request).toHaveBeenLastCalledWith(expect.any(String), {
      filter: {
        team: { id: { in: ["team-a", "team-b"] } },
        state: { type: { nin: ["completed", "canceled"] } },
      },
      first: 50,
      after: null,
    });
  });

  it("returns no suggestions when the viewer has no teams", async () => {
    const { service, request } = setup();
    for (let index = 0; index < filters.length; index++)
      request.mockResolvedValueOnce({ data: { issues: page([]) } });
    request.mockResolvedValueOnce({ data: { viewer: { teams: page([]) } } });
    await expect(service.fetchSuggestedIssues()).resolves.toEqual([]);
    expect(request).toHaveBeenCalledTimes(6);
  });

  it("propagates load failures rather than treating them as empty results", async () => {
    const { service, request } = setup();
    request.mockRejectedValue(new Error("offline"));
    await expect(service.fetchSuggestedIssues()).rejects.toThrow("offline");
    expect(request).toHaveBeenCalledTimes(5);
  });

  it("preserves remote search with assigned issues first", async () => {
    const { service, request } = setup();
    request.mockResolvedValueOnce({ data: { viewer: { id: "viewer" } } }).mockResolvedValueOnce({
      data: {
        searchIssues: {
          nodes: [
            { ...node("other"), assignee: null },
            { ...node("mine"), assignee: { id: "viewer" } },
          ],
        },
      },
    });
    expect((await service.fetchSuggestedIssues("  issue  ")).map((issue) => issue.id)).toEqual([
      "mine",
      "other",
    ]);
    expect(request).toHaveBeenLastCalledWith(expect.stringContaining("query SearchIssues"), {
      query: "issue",
      first: 50,
    });
  });
});
