import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { PullRequestInfo, Task, TaskMember, Workspace } from "../types";
import { githubSlugFromRemote } from "../utils";
import { LinearService } from "./linear";

export async function openCreatePr(branchName: string, member: TaskMember): Promise<boolean> {
  try {
    const remoteUrl = await invoke<string>("git_remote_url", { repoPath: member.localPath });
    await openUrl(`${remoteUrl}/compare/${branchName}?expand=1`);
    return true;
  } catch {
    console.error("Could not determine remote URL");
    return false;
  }
}

async function findMemberPr(
  task: Task,
  member: TaskMember,
  workspace: Workspace
): Promise<PullRequestInfo | undefined> {
  if (!task.linearIssueId || !workspace.linearApiKey) return undefined;
  try {
    const info = await new LinearService(workspace.linearApiKey).fetchIssueLinearInfoBatch([
      task.linearIssueId,
    ]);
    const prs = info[task.linearIssueId]?.prs ?? [];
    const remoteUrl = await invoke<string>("git_remote_url", { repoPath: member.localPath });
    const slug = githubSlugFromRemote(remoteUrl);
    if (!slug) return undefined;
    return prs.find((p) => p.repoSlug.toLowerCase() === slug);
  } catch {
    return undefined;
  }
}

/** Open the Linear-attached PR for this member repo, or GitHub's create-PR compare URL. */
export async function openPrForMember(
  task: Task,
  member: TaskMember,
  workspace: Workspace
): Promise<boolean> {
  const pr = await findMemberPr(task, member, workspace);
  if (pr) {
    await openUrl(pr.url);
    return true;
  }
  return openCreatePr(task.branchName, member);
}
