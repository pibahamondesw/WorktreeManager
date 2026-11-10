import { LinearClient } from "@linear/sdk";
import { IssueLinearInfo, LinearIssue, LinearTeam, PullRequestInfo } from "../types";

// ---- Standalone: does not require an initialized client ----

export async function validateLinearToken(
  apiKey: string
): Promise<{ valid: boolean; name?: string; orgUrlKey?: string; error?: string }> {
  try {
    const tempClient = new LinearClient({ apiKey });
    const [viewer, orgUrlKey] = await Promise.all([tempClient.viewer, fetchOrgUrlKey(tempClient)]);
    return { valid: true, name: viewer.name, orgUrlKey: orgUrlKey ?? undefined };
  } catch (e) {
    return {
      valid: false,
      error: e instanceof Error ? e.message : "Invalid API key",
    };
  }
}

/** Org slug used to build issue links. Best-effort: never invalidates a good key. */
async function fetchOrgUrlKey(client: LinearClient): Promise<string | null> {
  try {
    return (await client.organization).urlKey;
  } catch {
    return null;
  }
}

// ---- GraphQL response types ----

export interface GqlAttachmentNode {
  url?: string;
  title?: string;
  subtitle?: string;
  metadata?: string | Record<string, unknown>;
}

interface GqlIssueNode {
  id: string;
  identifier: string;
  title: string;
  branchName: string;
  description: string | null;
  priority: number;
  updatedAt: string;
  state: { name: string; type: string } | null;
  project: { id: string; name: string } | null;
}

interface ViewerIdResponse {
  viewer: { id: string };
}

interface SearchIssuesResponse {
  searchIssues: {
    nodes: (GqlIssueNode & { assignee: { id: string } | null })[];
  };
}

interface IssueSuggestionsResponse {
  issues: {
    nodes: GqlIssueNode[];
    pageInfo: { hasNextPage: boolean; endCursor: string | null };
  };
}

interface ViewerTeamsResponse {
  viewer: {
    teams: {
      nodes: { id: string }[];
      pageInfo: { hasNextPage: boolean; endCursor: string | null };
    };
  };
}

interface IssueStateResponse {
  issue: { state: { name: string; type: string } | null } | null;
}

interface IssueAttachmentsResponse {
  issue: { attachments: { nodes: GqlAttachmentNode[] } } | null;
}

interface IssuesBatchResponse {
  issues: {
    nodes: Array<{
      id: string;
      state: { name: string; type: string } | null;
      project?: { id: string; name: string } | null;
      attachments: { nodes: GqlAttachmentNode[] };
    }>;
  };
}

/** A Linear issue an automation keeps through the marker in its description. */
export interface MarkedIssueInput {
  marker: string;
  /** Team key, id or name. */
  team: string;
  title: string;
  /** Project name, slug id or id. */
  project?: string | null;
  /** `me`, or a user's email or display name. */
  assignee: string;
  /** Label names; blank or unknown ones are skipped with a warning. */
  labels: string[];
  /** Milestone name in the project; blank or unknown is skipped with a warning. */
  milestone?: string | null;
}

export interface MarkedIssue {
  id: string;
  identifier: string;
  /** The title the upsert leaves on the issue. */
  title: string;
  url: string;
  /** The branch Linear suggests for the issue; per-item tasks of a tracked campaign use it. */
  branchName: string;
  created: boolean;
  updated: boolean;
  /** Labels or milestone that could not be applied. */
  warnings: string[];
}

interface MarkedIssueNode {
  id: string;
  identifier: string;
  url: string;
  branchName: string;
  title: string;
  state: { id: string; name: string } | null;
  labels: { nodes: { id: string }[] };
  projectMilestone: { id: string } | null;
  team: { id: string; key: string; states: { nodes: { id: string; name: string }[] } };
}

interface MarkedIssuesResponse {
  issues: { nodes: MarkedIssueNode[] };
}

interface UpsertContextResponse {
  viewer: { id: string };
  teams: { nodes: { id: string; key: string; name: string }[] };
  issueLabels: { nodes: { id: string; name: string; team: { id: string } | null }[] };
}

interface ProjectNode {
  id: string;
  projectMilestones: { nodes: { id: string; name: string }[] };
}

type NodesResponse = Record<string, { nodes: { id: string }[] }>;

// ---- GraphQL queries ----

const MARKED_ISSUE_QUERY = `
  query MarkedIssue($marker: String!) {
    issues(filter: { description: { contains: $marker } }, first: 1) {
      nodes {
        id identifier url branchName title
        state { id name }
        labels(first: 250) { nodes { id } }
        projectMilestone { id }
        team { id key states(first: 250) { nodes { id name } } }
      }
    }
  }
`;

const UPSERT_CONTEXT_QUERY = `
  query UpsertContext($labels: [String!]) {
    viewer { id }
    teams(first: 250) { nodes { id key name } }
    issueLabels(filter: { name: { in: $labels } }, first: 250) { nodes { id name team { id } } }
  }
`;

const PROJECT_QUERY = `
  query Project($project: String!) {
    projects(filter: { or: [{ name: { eqIgnoreCase: $project } }, { slugId: { eq: $project } }] }, first: 1) {
      nodes { id projectMilestones(first: 250) { nodes { id name } } }
    }
  }
`;

const USER_QUERY = `
  query User($user: String!) {
    users(filter: { or: [{ email: { eq: $user } }, { displayName: { eqIgnoreCase: $user } }] }, first: 1) {
      nodes { id }
    }
  }
`;

const ISSUE_SUGGESTIONS_QUERY = `
  query IssueSuggestions($filter: IssueFilter, $first: Int, $after: String) {
    issues(filter: $filter, first: $first, after: $after, orderBy: updatedAt) {
      nodes {
        id
        identifier
        title
        branchName
        description
        priority
        updatedAt
        state { name type }
        project { id name }
      }
      pageInfo { hasNextPage endCursor }
    }
  }
`;

const VIEWER_TEAMS_QUERY = `
  query ViewerTeams($after: String) {
    viewer {
      teams(first: 50, after: $after) {
        nodes { id }
        pageInfo { hasNextPage endCursor }
      }
    }
  }
`;

const SEARCH_ISSUES_QUERY = `
  query SearchIssues($query: String!, $first: Int) {
    searchIssues(term: $query, includeArchived: false, first: $first) {
      nodes {
        id
        identifier
        title
        branchName
        description
        priority
        updatedAt
        state { name type }
        project { id name }
        assignee { id }
      }
    }
  }
`;

const VIEWER_ID_QUERY = `
  query { viewer { id } }
`;

const ISSUES_BATCH_QUERY = `
  query IssuesBatch($filter: IssueFilter) {
    issues(filter: $filter, first: 50) {
      nodes {
        id
        state { name type }
        project { id name }
        attachments { nodes { url title subtitle metadata } }
      }
    }
  }
`;

// ---- Pure helpers ----

function mapIssueNode(node: GqlIssueNode): LinearIssue {
  return {
    id: node.id,
    identifier: node.identifier,
    title: node.title,
    branchName: node.branchName,
    description: node.description ?? undefined,
    projectId: node.project?.id,
    projectName: node.project?.name ?? undefined,
    stateName: node.state?.name ?? undefined,
    stateType: node.state?.type ?? undefined,
    priority: node.priority,
    updatedAt: node.updatedAt,
  };
}

export function extractPrsFromAttachments(attachments: GqlAttachmentNode[]): PullRequestInfo[] {
  const prs: PullRequestInfo[] = [];
  const seen = new Set<string>();
  for (const att of attachments) {
    if (!att.url) continue;
    const prMatch = att.url.match(/github\.com\/([^/]+\/[^/]+)\/pull\/(\d+)/);
    if (!prMatch) continue;

    const key = `${prMatch[1].toLowerCase()}#${prMatch[2]}`;
    if (seen.has(key)) continue;
    seen.add(key);

    let state = "open";
    const meta = att.metadata;
    if (meta) {
      const parsed: unknown = typeof meta === "string" ? JSON.parse(meta) : meta;
      const metadata = parsed && typeof parsed === "object" ? parsed : {};
      const metaState =
        ("status" in metadata ? metadata.status : undefined) ??
        ("state" in metadata ? metadata.state : undefined);
      const status = typeof metaState === "string" ? metaState.toLowerCase() : "";
      const isDraft = ("draft" in metadata && metadata.draft === true) || status.includes("draft");
      if (status.includes("merge")) state = "merged";
      else if (status.includes("close")) state = "closed";
      else if (isDraft) state = "draft";
    }
    if (state === "open") {
      const sub = (att.subtitle ?? "").toLowerCase();
      if (sub.includes("merged")) state = "merged";
      else if (sub.includes("closed")) state = "closed";
    }

    prs.push({
      url: att.url,
      title: att.title ?? `PR #${prMatch[2]}`,
      state,
      number: parseInt(prMatch[2], 10),
      repoSlug: prMatch[1],
    });
  }
  return prs;
}

// ---- Service class: encapsulates the LinearClient ----

export class LinearService {
  private client: LinearClient;

  constructor(apiKey: string) {
    this.client = new LinearClient({ apiKey });
  }

  private get gql() {
    return this.client.client;
  }

  fetchOrgUrlKey(): Promise<string | null> {
    return fetchOrgUrlKey(this.client);
  }

  async fetchSuggestedIssues(query?: string): Promise<LinearIssue[]> {
    if (query && query.trim().length > 0) {
      const [viewerRes, searchRes] = await Promise.all([
        this.gql.rawRequest<ViewerIdResponse, Record<string, unknown>>(VIEWER_ID_QUERY, {}),
        this.gql.rawRequest<SearchIssuesResponse, Record<string, unknown>>(SEARCH_ISSUES_QUERY, {
          query: query.trim(),
          first: 50,
        }),
      ]);
      const viewerId = viewerRes.data!.viewer.id;
      const nodes = searchRes.data!.searchIssues.nodes;

      const mine: LinearIssue[] = [];
      const others: LinearIssue[] = [];
      for (const n of nodes) {
        (n.assignee?.id === viewerId ? mine : others).push(mapIssueNode(n));
      }
      return [...mine, ...others];
    }

    const me = { isMe: { eq: true } };
    const groups = await Promise.all(
      [
        { assignee: me },
        { creator: me },
        { project: { lead: me } },
        { project: { members: { some: me } } },
        { project: { creator: me } },
      ].map((filter) => this.fetchSuggestedIssueGroup(filter))
    );
    const seen = new Set<string>();
    const issues = groups.flat().filter((issue) => {
      if (seen.has(issue.id)) return false;
      seen.add(issue.id);
      return true;
    });
    if (issues.length > 0) return issues;

    // Only fall back to teams the viewer belongs to, rather than all visible teams.
    const teamIds: string[] = [];
    let after: string | null = null;
    do {
      const res = await this.gql.rawRequest<ViewerTeamsResponse, Record<string, unknown>>(
        VIEWER_TEAMS_QUERY,
        { after }
      );
      const teams: ViewerTeamsResponse["viewer"]["teams"] = res.data!.viewer.teams;
      teamIds.push(...teams.nodes.map((team) => team.id));
      after = teams.pageInfo.hasNextPage ? teams.pageInfo.endCursor : null;
    } while (after);
    return teamIds.length > 0
      ? this.fetchSuggestedIssueGroup({ team: { id: { in: teamIds } } })
      : [];
  }

  private async fetchSuggestedIssueGroup(filter: Record<string, unknown>): Promise<LinearIssue[]> {
    const issues: LinearIssue[] = [];
    let after: string | null = null;
    do {
      const res = await this.gql.rawRequest<IssueSuggestionsResponse, Record<string, unknown>>(
        ISSUE_SUGGESTIONS_QUERY,
        {
          filter: {
            ...filter,
            state: { type: { nin: ["completed", "canceled"] } },
          },
          first: 50,
          after,
        }
      );
      const page: IssueSuggestionsResponse["issues"] = res.data!.issues;
      issues.push(...page.nodes.map(mapIssueNode));
      after = page.pageInfo.hasNextPage ? page.pageInfo.endCursor : null;
    } while (after);

    // Linear's 0 means no priority, so it belongs after Low (4).
    return issues.sort(
      (a, b) => (a.priority || 5) - (b.priority || 5) || b.updatedAt.localeCompare(a.updatedAt)
    );
  }

  async getIssue(id: string): Promise<{
    id: string;
    identifier: string;
    title: string;
    projectId?: string;
    projectName?: string;
  }> {
    const issue = await this.client.issue(id);
    const project = await issue.project;
    return {
      id: issue.id,
      identifier: issue.identifier,
      title: issue.title,
      projectId: project?.id,
      projectName: project?.name,
    };
  }

  async listTeams(): Promise<LinearTeam[]> {
    const teams = await this.client.teams();
    return teams.nodes.map((team) => ({ id: team.id, key: team.key, name: team.name }));
  }

  async createIssue(input: {
    teamId: string;
    title: string;
    description?: string;
  }): Promise<LinearIssue> {
    const viewer = await this.gql.rawRequest<ViewerIdResponse, Record<string, unknown>>(
      VIEWER_ID_QUERY,
      {}
    );
    const result = await this.gql.rawRequest<
      { issueCreate: { success: boolean; issue: GqlIssueNode | null } },
      Record<string, unknown>
    >(
      `
      mutation CreateIssue($input: IssueCreateInput!) {
        issueCreate(input: $input) {
          success
          issue {
            id identifier title branchName description priority updatedAt
            project { id name }
            state { name type }
          }
        }
      }
    `,
      {
        input: {
          teamId: input.teamId,
          title: input.title,
          description: input.description || undefined,
          assigneeId: viewer.data!.viewer.id,
        },
      }
    );
    const payload = result.data?.issueCreate;
    if (!payload?.success || !payload.issue) throw new Error("Linear rejected the issue creation");
    return mapIssueNode(payload.issue);
  }

  private async markedIssue(marker: string): Promise<MarkedIssueNode | null> {
    const res = await this.gql.rawRequest<MarkedIssuesResponse, Record<string, unknown>>(
      MARKED_ISSUE_QUERY,
      { marker }
    );
    return res.data!.issues.nodes[0] ?? null;
  }

  private async updateMarkedIssue(id: string, input: Record<string, unknown>): Promise<void> {
    const res = await this.gql.rawRequest<
      { issueUpdate: { success: boolean } },
      Record<string, unknown>
    >(
      `mutation($id: String!, $input: IssueUpdateInput!) { issueUpdate(id: $id, input: $input) { success } }`,
      {
        id,
        input,
      }
    );
    if (!res.data?.issueUpdate.success) throw new Error("Linear rejected the issue update");
  }

  /** The id of the only node a lookup query returns, or an error naming what is missing. */
  private async lookup(query: string, variables: Record<string, string>, missing: string) {
    const res = await this.gql.rawRequest<NodesResponse, Record<string, unknown>>(query, variables);
    const id = Object.values(res.data!)[0].nodes[0]?.id;
    if (!id) throw new Error(missing);
    return id;
  }

  private async project(name: string): Promise<ProjectNode> {
    const res = await this.gql.rawRequest<
      { projects: { nodes: ProjectNode[] } },
      Record<string, unknown>
    >(PROJECT_QUERY, { project: name });
    const project = res.data!.projects.nodes[0];
    if (!project) throw new Error(`Linear project ${name} not found`);
    return project;
  }

  private async upsertContext(labels: string[]): Promise<UpsertContextResponse> {
    const res = await this.gql.rawRequest<UpsertContextResponse, Record<string, unknown>>(
      UPSERT_CONTEXT_QUERY,
      { labels }
    );
    return res.data!;
  }

  /**
   * Reuses the issue whose description carries the marker, retitling it and adding the labels and
   * milestone it lacks, or creates one.
   */
  async upsertMarkedIssue(input: MarkedIssueInput): Promise<MarkedIssue> {
    const warnings: string[] = [];
    const labels = labelNames(input.labels, warnings);
    const existing = await this.markedIssue(input.marker);
    if (existing) return this.reconcileMarkedIssue(existing, input, labels, warnings);
    const { viewer, teams, issueLabels } = await this.upsertContext(labels);
    const wanted = input.team.toLowerCase();
    const team = teams.nodes.find((candidate) =>
      [candidate.id, candidate.key.toLowerCase(), candidate.name.toLowerCase()].includes(wanted)
    );
    if (!team) throw new Error(`Linear team ${input.team} not found`);
    const labelIds = labelIdsInTeam(labels, issueLabels.nodes, team, warnings);
    const [project, assigneeId] = await Promise.all([
      input.project ? this.project(input.project) : undefined,
      input.assignee === "me"
        ? viewer.id
        : this.lookup(
            USER_QUERY,
            { user: input.assignee },
            `Linear user ${input.assignee} not found`
          ),
    ]);
    const projectMilestoneId = milestoneId(input.milestone, project, warnings);
    const res = await this.gql.rawRequest<
      {
        issueCreate: {
          success: boolean;
          issue: { id: string; identifier: string; url: string; branchName: string } | null;
        };
      },
      Record<string, unknown>
    >(
      `mutation($input: IssueCreateInput!) { issueCreate(input: $input) { success issue { id identifier url branchName } } }`,
      {
        input: {
          teamId: team.id,
          title: input.title,
          description: input.marker,
          assigneeId,
          labelIds,
          ...(project ? { projectId: project.id } : {}),
          ...(projectMilestoneId ? { projectMilestoneId } : {}),
        },
      }
    );
    const created = res.data?.issueCreate;
    if (!created?.success || !created.issue) throw new Error("Linear rejected the issue creation");
    return { ...created.issue, title: input.title, created: true, updated: false, warnings };
  }

  private async reconcileMarkedIssue(
    existing: MarkedIssueNode,
    input: MarkedIssueInput,
    labels: string[],
    warnings: string[]
  ): Promise<MarkedIssue> {
    const known = labels.length ? (await this.upsertContext(labels)).issueLabels.nodes : [];
    const current = new Set(existing.labels.nodes.map((label) => label.id));
    const addedLabelIds = labelIdsInTeam(labels, known, existing.team, warnings).filter(
      (id) => !current.has(id)
    );
    const project =
      input.milestone?.trim() && input.project ? await this.project(input.project) : undefined;
    const projectMilestoneId = milestoneId(input.milestone, project, warnings);
    const changes = {
      ...(existing.title !== input.title ? { title: input.title } : {}),
      ...(addedLabelIds.length ? { addedLabelIds } : {}),
      ...(projectMilestoneId && projectMilestoneId !== existing.projectMilestone?.id
        ? { projectMilestoneId }
        : {}),
    };
    const updated = Object.keys(changes).length > 0;
    if (updated) await this.updateMarkedIssue(existing.id, changes);
    const { id, identifier, url, branchName } = existing;
    return {
      id,
      identifier,
      title: input.title,
      url,
      branchName,
      created: false,
      updated,
      warnings,
    };
  }

  /** Moves the issue whose description carries the marker to the team's state named `state`. */
  async moveMarkedIssue(
    marker: string,
    state: string
  ): Promise<{ identifier: string; changed: boolean }> {
    const issue = await this.markedIssue(marker);
    if (!issue) throw new Error(`No Linear issue carries the marker ${marker}`);
    const named = (candidate: { name: string } | null) =>
      candidate?.name.toLowerCase() === state.toLowerCase();
    if (named(issue.state)) return { identifier: issue.identifier, changed: false };
    const target = issue.team.states.nodes.find(named);
    if (!target) throw new Error(`Linear state ${state} not found in the issue's team`);
    await this.updateMarkedIssue(issue.id, { stateId: target.id });
    return { identifier: issue.identifier, changed: true };
  }

  async updateIssueTitle(issueId: string, title: string): Promise<void> {
    const result = await this.client.updateIssue(issueId, { title });
    if (!result.success) throw new Error("Linear rejected the title update");
  }

  async startIssue(issueId: string): Promise<void> {
    const issue = await this.client.issue(issueId);
    const currentState = await issue.state;

    const startableTypes = ["triage", "backlog", "unstarted"];
    if (currentState && !startableTypes.includes(currentState.type)) {
      return;
    }

    const team = await issue.team;
    if (!team) return;

    const states = await team.states();
    const inProgress = states.nodes.find(
      (s) => s.type === "started" && s.name.toLowerCase() === "in progress"
    );

    if (inProgress) {
      await this.client.updateIssue(issueId, { stateId: inProgress.id });
    }
  }

  async getIssueStatus(issueId: string): Promise<{ name: string; type: string } | null> {
    try {
      const res = await this.gql.rawRequest<IssueStateResponse, Record<string, unknown>>(
        `query($id: String!) { issue(id: $id) { state { name type } } }`,
        { id: issueId }
      );
      const state = res.data?.issue?.state;
      return state ? { name: state.name, type: state.type } : null;
    } catch {
      return null;
    }
  }

  async getIssuePullRequests(issueId: string): Promise<PullRequestInfo[]> {
    try {
      const res = await this.gql.rawRequest<IssueAttachmentsResponse, Record<string, unknown>>(
        `query($id: String!) {
          issue(id: $id) {
            attachments { nodes { url title subtitle metadata } }
          }
        }`,
        { id: issueId }
      );
      const attachments = res.data?.issue?.attachments?.nodes ?? [];
      return extractPrsFromAttachments(attachments);
    } catch {
      return [];
    }
  }

  async fetchIssueLinearInfoBatch(issueIds: string[]): Promise<Record<string, IssueLinearInfo>> {
    if (issueIds.length === 0) return {};
    if (issueIds.length > 50) {
      const result: Record<string, IssueLinearInfo> = {};
      for (let offset = 0; offset < issueIds.length; offset += 50) {
        Object.assign(
          result,
          await this.fetchIssueLinearInfoBatch(issueIds.slice(offset, offset + 50))
        );
      }
      return result;
    }

    try {
      const res = await this.gql.rawRequest<IssuesBatchResponse, Record<string, unknown>>(
        ISSUES_BATCH_QUERY,
        {
          filter: { id: { in: issueIds } },
        }
      );

      const nodes = res.data!.issues.nodes;

      const result: Record<string, IssueLinearInfo> = {};
      for (const node of nodes) {
        result[node.id] = {
          project: node.project,
          status: node.state ? { name: node.state.name, type: node.state.type } : null,
          prs: extractPrsFromAttachments(node.attachments?.nodes ?? []),
        };
      }
      return result;
    } catch {
      return {};
    }
  }
}

function labelNames(labels: string[], warnings: string[]): string[] {
  const names = labels.map((label) => label.trim());
  if (names.includes("")) warnings.push("A Linear label rendered empty; skipped it");
  return names.filter(Boolean);
}

function labelIdsInTeam(
  names: string[],
  labels: UpsertContextResponse["issueLabels"]["nodes"],
  team: { id: string; key: string },
  warnings: string[]
): string[] {
  return names.flatMap((name) => {
    const label = labels.find(
      (candidate) => candidate.name === name && (!candidate.team || candidate.team.id === team.id)
    );
    if (label) return [label.id];
    warnings.push(`Linear label ${name} not found in team ${team.key}; skipped it`);
    return [];
  });
}

function milestoneId(
  name: string | null | undefined,
  project: ProjectNode | undefined,
  warnings: string[]
): string | undefined {
  if (name == null) return undefined;
  const wanted = name.trim();
  if (!wanted) {
    warnings.push("The Linear milestone rendered empty; skipped it");
    return undefined;
  }
  if (!project) {
    warnings.push(`Linear milestone ${wanted} needs a project; skipped it`);
    return undefined;
  }
  const milestone = project.projectMilestones.nodes.find(
    (candidate) => candidate.name.toLowerCase() === wanted.toLowerCase()
  );
  if (!milestone) warnings.push(`Linear milestone ${wanted} not found in the project; skipped it`);
  return milestone?.id;
}
