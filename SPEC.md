# WorktreeManager — Application Specification

A minimalistic macOS desktop app for managing Git worktrees with Linear integration. Designed for developers who use worktree-based workflows with Cursor IDE.

---

## Overview

WorktreeManager lets you:

1. Register local Git repositories as "projects"
2. Browse your assigned Linear issues
3. Create a Git worktree for a selected issue (automatically fetching latest, branching from origin/main, and setting the issue as "started")
4. Open the worktree in Cursor IDE with one click
5. Track active worktrees with live Linear status and PR links

No GitHub integration required — PR info is retrieved from Linear's issue attachments (Linear's native GitHub integration handles the linking).

---

## Tech Stack

| Layer             | Technology                                                                         |
| ----------------- | ---------------------------------------------------------------------------------- |
| Desktop framework | **Tauri v2** (Rust backend + webview frontend)                                     |
| Frontend          | **React 19** + **TypeScript**                                                      |
| Styling           | **Tailwind CSS v4** (dark mode only)                                               |
| Linear API        | **@linear/sdk** (use raw GraphQL via `client.client.rawRequest()` for performance) |
| Persistence       | **@tauri-apps/plugin-store** (JSON key-value store)                                |
| File dialogs      | **@tauri-apps/plugin-dialog**                                                      |
| Open URLs         | **@tauri-apps/plugin-opener**                                                      |
| Shell commands    | **@tauri-apps/plugin-shell**                                                       |
| Git operations    | `std::process::Command` in Rust (calling `git` CLI)                                |
| Cursor launch     | `open -a Cursor <path>` via Rust (uses macOS LaunchServices for full permissions)  |

### Key Dependencies (package.json)

```json
{
  "dependencies": {
    "@linear/sdk": "^75.0.0",
    "@tauri-apps/api": "^2.10.1",
    "@tauri-apps/plugin-dialog": "^2.6.0",
    "@tauri-apps/plugin-opener": "^2.5.3",
    "@tauri-apps/plugin-shell": "^2.3.5",
    "@tauri-apps/plugin-store": "^2.4.2",
    "react": "^19.2.0",
    "react-dom": "^19.2.0",
    "uuid": "^13.0.0"
  }
}
```

### Rust Dependencies (Cargo.toml)

```toml
[dependencies]
serde_json = "1"
serde = { version = "1", features = ["derive"] }
log = "0.4"
tauri = { version = "2", features = [] }
tauri-plugin-log = "2"
tauri-plugin-store = "2"
tauri-plugin-opener = "2"
tauri-plugin-dialog = "2"
tauri-plugin-shell = "2"
```

---

## Data Model

```typescript
interface Repo {
  id: string; // UUID
  name: string; // Display name (e.g. "Fintoc Rails")
  localPath: string; // Absolute path to cloned repo (e.g. /Users/.../fintoc-rails)
  worktreeBasePath: string; // Where worktrees are created (e.g. ~/Documents/WorktreeManager/fintoc-rails)
}

interface Worktree {
  id: string;
  repoId: string;
  branchName: string;
  linearIssueId: string;
  linearIssueTitle: string;
  linearIssueIdentifier: string; // e.g. "TSY-3140"
  path: string; // Full path to worktree directory
  createdAt: string; // ISO timestamp
}

interface AppState {
  setup: {
    linearApiKey: string | null;
    isComplete: boolean;
  };
  repos: Repo[];
  worktrees: Worktree[];
  selectedRepoId: string | null;
}

interface LinearIssue {
  id: string;
  identifier: string; // e.g. "TSY-3140"
  title: string;
  branchName: string; // Linear's suggested branch name
  description?: string;
  projectName?: string;
  stateName?: string; // e.g. "In Progress"
  stateType?: string; // e.g. "started", "completed"
  priority: number; // 0=none, 1=urgent, 2=high, 3=medium, 4=low
  updatedAt: string;
}

interface PullRequestInfo {
  url: string;
  title: string;
  state: string; // "open" | "merged" | "closed"
  number: number;
}
```

---

## UI Layout

### Window Configuration

- **Size**: 1000x700, min 800x500, resizable
- **Title bar**: macOS overlay style (`titleBarStyle: "Overlay"`)
- **Theme**: Dark mode only
- **Font**: System font (-apple-system, BlinkMacSystemFont)
- **Root CSS**: `user-select: none` globally, with `.select-text` override for copyable content

### Layout Structure

```
┌─────────────────────────────────────────────────────┐
│  [traffic lights]    WorktreeManager                │  ← 38px titlebar drag area
├──────────┬──────────────────────────────────────────┤  ← 1px divider (full width, absolute positioned at top: 38px)
│ PROJECTS │  Fintoc Rails   3 worktrees   [↻] [+New]│  ← py-3 symmetric padding
│          ├──────────────────────────────────────────┤
│ • Proj A │  ┌─────────────────────────────────────┐ │
│ • Proj B │  │ TSY-3140  [In Progress]             │ │  ← Worktree card (clickable → opens Cursor)
│          │  │ Eliminar columnas discount_status    │ │
│   [+]    │  │ ⑂ tsy-3140-eliminar-columnas...     │ │
│          │  │ ⑃ PR #456: Fix discount   [open]    │ │
│          │  │                              [🗑]    │ │
│          │  └─────────────────────────────────────┘ │
│          │                                          │
│          │  ┌─────────────────────────────────────┐ │
│          │  │ TSY-3142  [Todo]                    │ │
│          │  │ ...                                 │ │
│          │  └─────────────────────────────────────┘ │
├──────────┴──────────────────────────────────────────┤
│  Sidebar: 240px (w-60)  │  Main: flex-1            │
└─────────────────────────────────────────────────────┘
```

### Screens

1. **Setup Wizard** — Shown on first launch. Single step: enter Linear API key. Link to `https://linear.app/settings/api` to generate one. Validates the key by calling `viewer` query.

2. **Main View** — Two-panel layout:
   - **Left sidebar**: Project list with selection indicator, add (+) button, hover-to-show remove (×) button
   - **Right panel**: Worktree list for selected project, with header showing project name, worktree count, refresh button, and "New Worktree" button

3. **Add Project Modal** — Fields:
   - Project name (text input, auto-filled from folder name on browse)
   - Local clone path (read-only, populated via native folder picker)
   - Worktree directory (auto-filled as `~/Documents/WorktreeManager/<project-slug>`, editable)

4. **New Worktree Modal** — Two-phase:
   - **Phase 1**: Search & select a Linear issue. Shows list of assigned issues (not completed/cancelled). Search bar with 300ms debounce.
   - **Phase 2**: After selection, shows issue details, branch name (copyable), worktree path. "Create Worktree" button with progress status.

---

## Linear Integration

### Authentication

- Personal API key (`lin_api_...`) stored in the app's local store
- No OAuth — simpler setup, works without org approval

### Key Design Decision: Use Raw GraphQL, Not SDK Methods

The `@linear/sdk` uses lazy loading for relations — each `.state`, `.project`, `.assignee` access triggers a separate HTTP request. With 50 issues, this means 100+ sequential API calls.

**Solution**: Use `client.client.rawRequest(query, variables)` to fetch everything in a single GraphQL query with nested fields.

### Queries

**Fetch assigned issues (default view)** — 1 API call:

```graphql
query AssignedIssues($filter: IssueFilter, $first: Int) {
  viewer {
    assignedIssues(filter: $filter, first: $first, orderBy: updatedAt) {
      nodes {
        id
        identifier
        title
        branchName
        description
        priority
        updatedAt
        state {
          name
          type
        }
        project {
          name
        }
      }
    }
  }
}
# Variables: { filter: { state: { type: { nin: ["completed", "canceled"] } } }, first: 50 }
```

**Search issues** — 2 parallel API calls (viewer ID + search):

```graphql
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
      state {
        name
        type
      }
      project {
        name
      }
      assignee {
        id
      }
    }
  }
}
# Filter results client-side by assignee.id === viewer.id
```

**Get issue status** (for worktree cards):

```graphql
query ($id: String!) {
  issue(id: $id) {
    state {
      name
      type
    }
  }
}
```

**Get PR from attachments** (Linear's GitHub integration creates these automatically):

```graphql
query ($id: String!) {
  issue(id: $id) {
    attachments {
      nodes {
        url
        title
        subtitle
      }
    }
  }
}
# Match attachment URLs against: /github\.com\/[^/]+\/[^/]+\/pull\/(\d+)/
# Determine state from subtitle: contains "merged" → merged, "closed" → closed, else → open
```

**Start issue** (uses SDK for convenience — mutations are single calls):

```typescript
const issue = await client.issue(issueId);
const team = await issue.team;
const states = await team.states();
const startedState = states.nodes.find((s) => s.type === "started");
await client.updateIssue(issueId, { stateId: startedState.id });
```

### Hybrid Search Strategy

When the New Worktree modal opens:

1. Fetch 50 assigned issues and **cache in memory**
2. As user types, **instantly filter the cache** (substring match on identifier, title, projectName, description, branchName)
3. **In parallel** (after 300ms debounce), call Linear's `searchIssues` API for broader results
4. **Merge**: show local results first, then append any remote results not already in the local set (deduplicated by `id`)

This gives instant partial-ID matching (e.g. "3140" matches "TSY-3140") with zero speed cost.

---

## Rust Backend (Tauri Commands)

### `git_worktree_add(repo_path, worktree_path, branch_name)`

1. `create_dir_all` for the worktree parent directory
2. If worktree already exists (directory + `.git` file), return success (idempotent)
3. `git -C <repo_path> fetch origin` — always fetch latest
4. Detect default branch: check `refs/remotes/origin/HEAD`, fall back to probing `origin/main` then `origin/master`
5. `git -C <repo_path> worktree add <worktree_path> -b <branch_name> origin/<default_branch>`
6. If branch already exists (error contains "already exists"), retry without `-b`: `git worktree add <path> <branch_name>`

### `git_worktree_remove(repo_path, worktree_path)`

- `git -C <repo_path> worktree remove <worktree_path> --force`

### `git_worktree_list(repo_path)`

- `git -C <repo_path> worktree list --porcelain`
- Parse output into `Vec<WorktreeInfo>` (path, branch, head, bare)

### `open_cursor(path)`

- **Critical**: Use `open -a Cursor <path>` (macOS LaunchServices), NOT `Command::new("cursor")`. The latter inherits the Tauri app's restricted environment/PATH, causing permission issues and "command not found" errors.

---

## Worktree Creation Flow

When the user clicks "Create Worktree" after selecting a Linear issue:

1. **Validate** that `repo.worktreeBasePath` is set (guard against legacy data)
2. **Compute path**: `${repo.worktreeBasePath}/${issue.branchName}`
3. **Show status**: "Fetching latest from origin..."
4. **Call** `git_worktree_add` (Rust): fetches origin, creates branch from `origin/main`
5. **Show status**: "Updating Linear issue..."
6. **Call** `startIssue(issueId)`: transitions issue to "started" state
7. **Save worktree record** to store **before** opening Cursor (so a Cursor failure doesn't lose the record)
8. **Show status**: "Opening Cursor..."
9. **Call** `open_cursor(path)` — best-effort, failure is non-fatal
10. **Close modal**

---

## Persistence

Uses `@tauri-apps/plugin-store` with a `store.json` file. Four keys:

- `setup`: `{ linearApiKey, isComplete }`
- `repos`: `Repo[]`
- `worktrees`: `Worktree[]`
- `selectedRepoId`: `string | null`

Auto-save enabled. Load with migration support for schema changes.

---

## Tauri Configuration

```json
{
  "app": {
    "windows": [
      {
        "title": "WorktreeManager",
        "width": 1000,
        "height": 700,
        "minWidth": 800,
        "minHeight": 500,
        "decorations": true,
        "titleBarStyle": "Overlay"
      }
    ],
    "security": {
      "csp": "default-src 'self'; connect-src 'self' https://api.linear.app; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self' data:"
    }
  }
}
```

### Capabilities (permissions)

```json
{
  "permissions": [
    "core:default",
    "store:default",
    "opener:default",
    "dialog:default",
    "shell:allow-open",
    "shell:allow-execute"
  ]
}
```

---

## UI/UX Design Guidelines

- **Dark mode only** — Background: `#0a0a0f`, secondary: `#12121a`, tertiary: `#1a1a25`
- **Accent color**: Indigo (`#6366f1`)
- **Minimal chrome** — No unnecessary borders, subtle hover states
- **Overlay titlebar** — 38px reserved for macOS traffic lights, with a 1px `border` divider spanning full width at that height
- **Scrollbars** — Thin (6px), only visible on scrollable areas
- **Text selection** — Disabled globally (`user-select: none`), but enabled on error messages, branch names, and paths via `.select-text` class with `!important` override
- **Titlebar drag** — `data-tauri-drag-region` attribute on spacer divs and headers; `button`, `input`, `a` elements excluded from drag via `-webkit-app-region: no-drag`
- **Loading states** — Spinner for initial loads, inline status text for multi-step operations (e.g. "Fetching latest from origin...")
- **Error display** — Red border/background container with selectable text
- **Worktree cards** — Clickable (opens Cursor), with hover state, trash icon appears on hover
- **Refresh button** — In the worktree list header, re-fetches Linear status & PR info for all cards

---

## Gotchas & Lessons Learned

1. **macOS .app bundles run with a minimal PATH** — Don't use `Command::new("cursor")` or `Command::new("git")` expecting them to be in PATH. For `git`, it typically works because it's in `/usr/bin/git`. For Cursor, use `open -a Cursor` instead.

2. **Tauri app sandbox restrictions** — Child processes spawned via `std::process::Command` inherit the parent's environment. Use macOS `open -a` to launch apps with their own full permissions.

3. **Linear SDK N+1 problem** — The SDK's lazy-loading pattern causes 100+ HTTP requests when iterating issues. Always use raw GraphQL for bulk fetches.

4. **`homeDir()` from `@tauri-apps/api/path`** — May or may not include a trailing slash depending on the OS. Always normalize: `home.endsWith("/") ? home : home + "/"`.

5. **`user-select: none` on root** — Necessary for a native app feel, but requires explicit `!important` overrides for copyable content. Plain CSS class `.select-text` won't work without `!important` because the root rule has higher specificity through the element selector.

6. **CSP configuration** — Must include `connect-src https://api.linear.app` for the Linear SDK's GraphQL requests to work from the webview.

7. **Worktree creation is not atomic** — Save the worktree record to the store before opening Cursor. If Cursor fails, the worktree still exists on disk and is tracked in the app.

8. **DMG bundling may fail in CI/sandboxed environments** — The `.app` bundle is the important artifact; DMG creation can fail without affecting the app itself.

---

## Project Structure

```
├── src/
│   ├── App.tsx                          # Root layout (sidebar + main panel)
│   ├── index.css                        # Tailwind + theme variables + global styles
│   ├── types.ts                         # All TypeScript interfaces + DEFAULT_STATE
│   ├── hooks/
│   │   ├── useStore.ts                  # Global state management hook
│   │   └── useDebounce.ts              # Debounce hook for search input
│   ├── services/
│   │   ├── linear.ts                    # All Linear API interactions (raw GraphQL)
│   │   └── store.ts                     # Tauri Store load/save functions
│   └── components/
│       ├── ui/
│       │   ├── Button.tsx               # Reusable button (variants: primary, secondary, ghost)
│       │   ├── Badge.tsx                # Status badges (colored by variant)
│       │   ├── Input.tsx                # Text input with label/hint/error
│       │   └── Modal.tsx                # Modal dialog wrapper
│       ├── setup/
│       │   ├── SetupWizard.tsx          # Linear API key setup (single step)
│       │   └── TokenInput.tsx           # Reusable token input with validation
│       ├── sidebar/
│       │   ├── RepoList.tsx             # Project sidebar list
│       │   └── AddRepoModal.tsx         # Add project modal
│       └── worktree/
│           ├── WorktreeList.tsx          # Main panel: worktree cards + header
│           ├── WorktreeCard.tsx          # Single worktree card (status, PR, actions)
│           └── NewWorktreeModal.tsx      # Issue search + worktree creation
├── src-tauri/
│   ├── tauri.conf.json                  # Tauri config (window, CSP, bundle)
│   ├── capabilities/default.json        # Plugin permissions
│   ├── Cargo.toml                       # Rust dependencies
│   └── src/
│       ├── lib.rs                       # Tauri builder setup + plugin registration
│       └── commands/
│           ├── mod.rs                   # pub mod git; pub mod cursor;
│           ├── git.rs                   # git worktree add/remove/list commands
│           └── cursor.rs                # open_cursor command (via macOS `open -a`)
├── package.json
├── tsconfig.json / tsconfig.app.json
└── vite.config.ts
```
