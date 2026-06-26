import { LinearClient } from "@linear/sdk";
import { IssueLinearInfo, LinearIssue, PullRequestInfo } from "../types";

// ---- Standalone: does not require an initialized client ----

export async function validateLinearToken(
  apiKey: string
): Promise<{ valid: boolean; name?: string; error?: string }> {
  try {
    const tempClient = new LinearClient({ apiKey });
    const viewer = await tempClient.viewer;
    return { valid: true, name: viewer.name };
  } catch (e) {
    return {
      valid: false,
      error: e instanceof Error ? e.message : "Invalid API key",
    };
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
  project: { name: string } | null;
}

interface ViewerIdResponse {
  viewer: { id: string };
}

interface SearchIssuesResponse {
  searchIssues: {
    nodes: (GqlIssueNode & { assignee: { id: string } | null })[];
  };
}

interface AssignedIssuesResponse {
  viewer: {
    assignedIssues: { nodes: GqlIssueNode[] };
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
      attachments: { nodes: GqlAttachmentNode[] };
    }>;
  };
}

// ---- GraphQL queries ----

const ASSIGNED_ISSUES_QUERY = `
  query AssignedIssues($filter: IssueFilter, $first: Int) {
    viewer {
      assignedIssues(
        filter: $filter
        first: $first
        orderBy: updatedAt
      ) {
        nodes {
          id
          identifier
          title
          branchName
          description
          priority
          updatedAt
          state { name type }
          project { name }
        }
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
        project { name }
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
    issues(filter: $filter) {
      nodes {
        id
        state { name type }
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
    projectName: node.project?.name ?? undefined,
    stateName: node.state?.name ?? undefined,
    stateType: node.state?.type ?? undefined,
    priority: node.priority,
    updatedAt: node.updatedAt,
  };
}

export function extractPrFromAttachments(attachments: GqlAttachmentNode[]): PullRequestInfo | null {
  for (const att of attachments) {
    if (!att.url) continue;
    const prMatch = att.url.match(/github\.com\/[^/]+\/[^/]+\/pull\/(\d+)/);
    if (prMatch) {
      let state = "open";
      const meta = att.metadata;
      if (meta) {
        const parsed = typeof meta === "string" ? JSON.parse(meta) : meta;
        const metaState = parsed?.status ?? parsed?.state;
        if (typeof metaState === "string") {
          const s = metaState.toLowerCase();
          if (s.includes("merge")) state = "merged";
          else if (s.includes("close")) state = "closed";
        }
      }
      if (state === "open") {
        const sub = (att.subtitle ?? "").toLowerCase();
        if (sub.includes("merged")) state = "merged";
        else if (sub.includes("closed")) state = "closed";
      }

      return {
        url: att.url,
        title: att.title ?? `PR #${prMatch[1]}`,
        state,
        number: parseInt(prMatch[1], 10),
      };
    }
  }
  return null;
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

  async fetchAssignedIssues(query?: string): Promise<LinearIssue[]> {
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

    const res = await this.gql.rawRequest<AssignedIssuesResponse, Record<string, unknown>>(ASSIGNED_ISSUES_QUERY, {
      filter: {
        state: { type: { nin: ["completed", "canceled"] } },
      },
      first: 50,
    });
    const nodes = res.data!.viewer.assignedIssues.nodes;
    return nodes.map(mapIssueNode);
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

  async getIssueStatus(
    issueId: string
  ): Promise<{ name: string; type: string } | null> {
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

  async getIssuePullRequest(issueId: string): Promise<PullRequestInfo | null> {
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
      return extractPrFromAttachments(attachments);
    } catch {
      return null;
    }
  }

  async fetchIssueLinearInfoBatch(
    issueIds: string[]
  ): Promise<Record<string, IssueLinearInfo>> {
    if (issueIds.length === 0) return {};

    try {
      const res = await this.gql.rawRequest<IssuesBatchResponse, Record<string, unknown>>(ISSUES_BATCH_QUERY, {
        filter: { id: { in: issueIds } },
      });

      const nodes = res.data!.issues.nodes;

      const result: Record<string, IssueLinearInfo> = {};
      for (const node of nodes) {
        result[node.id] = {
          status: node.state ? { name: node.state.name, type: node.state.type } : null,
          pr: extractPrFromAttachments(node.attachments?.nodes ?? []),
        };
      }
      return result;
    } catch {
      return {};
    }
  }
}
