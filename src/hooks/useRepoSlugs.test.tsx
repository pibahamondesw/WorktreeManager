// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, renderHook, waitFor } from "@testing-library/react";
import { githubSlugFromRemote, resetRepoSlugCache, useRepoSlugs } from "./useRepoSlugs";
import { Workspace } from "../types";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));

const workspace = (id: string, repos: Workspace["repos"]): Workspace => ({ id, name: id, repos });
const repo = (id: string, localPath: string) => ({
  id,
  name: id,
  localPath,
  worktreeBasePath: `${localPath}/.wt`,
});

beforeEach(() => {
  resetRepoSlugCache();
  mocks.invoke
    .mockReset()
    .mockImplementation(async (_cmd: string, args: { repoPath: string }) =>
      args.repoPath === "/gitlab"
        ? "https://gitlab.com/x/y"
        : `git@github.com:Org/${args.repoPath.slice(1)}.git`
    );
});
afterEach(cleanup);

describe("githubSlugFromRemote", () => {
  it("normalises ssh and https remotes and rejects other hosts", () => {
    expect(githubSlugFromRemote("git@github.com:Fintoc/Rails.git")).toBe("fintoc/rails");
    expect(githubSlugFromRemote("https://github.com/fintoc/rails")).toBe("fintoc/rails");
    expect(githubSlugFromRemote("https://github.com/fintoc/fintoc.js")).toBe("fintoc/fintoc.js");
    expect(githubSlugFromRemote("https://gitlab.com/x/y")).toBeNull();
  });
});

describe("useRepoSlugs", () => {
  it("resolves one slug per repo and skips non-GitHub remotes", async () => {
    const ws = workspace("w", [repo("a", "/api"), repo("b", "/gitlab")]);
    const { result } = renderHook(() => useRepoSlugs(ws));
    expect(result.current).toBeNull();
    await waitFor(() => expect(result.current).toEqual({ a: "org/api" }));
    expect(mocks.invoke).toHaveBeenCalledTimes(2);
  });

  it("reuses resolved slugs when switching back to a workspace", async () => {
    const first = workspace("w1", [repo("a", "/api")]);
    const second = workspace("w2", [repo("b", "/web")]);
    const { result, rerender } = renderHook(({ ws }) => useRepoSlugs(ws), {
      initialProps: { ws: first },
    });
    await waitFor(() => expect(result.current).toEqual({ a: "org/api" }));
    rerender({ ws: second });
    await waitFor(() => expect(result.current).toEqual({ b: "org/web" }));
    rerender({ ws: first });
    await waitFor(() => expect(result.current).toEqual({ a: "org/api" }));
    expect(mocks.invoke).toHaveBeenCalledTimes(2);
  });

  it("returns null without a workspace", () => {
    const { result } = renderHook(() => useRepoSlugs(undefined));
    expect(result.current).toBeNull();
    expect(mocks.invoke).not.toHaveBeenCalled();
  });
});
