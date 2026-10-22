import { openUrl } from "@tauri-apps/plugin-opener";
import { Task, TaskMember, Workspace } from "../types";
import { taskLinearStore } from "./taskLinear";

export async function openCreatePr(branchName: string, member: TaskMember): Promise<boolean> {
  try {
    const slug = await taskLinearStore.resolveSlug(member.localPath);
    if (!slug) return false;
    await openUrl(`https://github.com/${slug}/compare/${branchName}?expand=1`);
    return true;
  } catch {
    console.error("Could not determine remote URL");
    return false;
  }
}

/** Open the Linear-attached PR for this member repo, or GitHub's create-PR compare URL. */
export async function openPrForMember(
  task: Task,
  member: TaskMember,
  workspace: Workspace
): Promise<boolean> {
  await Promise.all([
    task.linearIssueId
      ? taskLinearStore.refreshIssues(workspace, [task.linearIssueId]).catch(() => undefined)
      : Promise.resolve(),
    taskLinearStore.resolveSlug(member.localPath),
  ]);
  const pr = taskLinearStore.memberPrs(task, member, workspace)[0];
  if (pr) {
    await openUrl(pr.url);
    return true;
  }
  return openCreatePr(task.branchName, member);
}
