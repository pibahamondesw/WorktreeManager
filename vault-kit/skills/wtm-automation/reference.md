# Automation reference

A condensed v1 summary for authors. The contract, with every rule and default, is [docs/automations.md](https://github.com/pibahamondesw/WorktreeManager/blob/main/docs/automations.md); when they disagree, the spec wins.

## Files

```
~/.worktreemanager/automations/   # the automations directory (an app preference)
  <name>/
    AUTOMATION.md                 # YAML frontmatter + Markdown instructions
    scripts/                      # selectors, verifiers, helpers
    schemas/                      # JSON schemas for structured agent results
```

- The folder name equals `name`. Paths in the definition are relative to the folder and may not leave it.
- The body is trusted instructions included in every agent step (≤ 15,000 characters). It may be empty only if every agent step has a `prompt` or `skill`.
- Keys are `snake_case`; unknown keys are errors except `x-…`. Durations: `90s`, `45m`, `2h`, `7d`.
- Templates (`{var}`, only in fields marked templated): `{automation}`, `{run.id}`, `{run.short_id}`, `{run.scheduled_for}`, `{item.id}`, `{item.version}`, `{item.title}`, `{item.data.<key>}`, `{tracker.issue}`, `{task.id}`, `{task.branch}`, `{pr.number}`, `{pr.url}`.
- Machine state (enabled, runs, cursors, decisions) lives in app data, never in the file. A new automation starts paused.

## Top-level fields

| Field           | Default                                                  | Notes                                                                          |
| --------------- | -------------------------------------------------------- | ------------------------------------------------------------------------------ |
| `schema`        | required                                                 | `1`.                                                                           |
| `name`          | required                                                 | `^[a-z0-9][a-z0-9-]{0,62}$`, equals the folder name.                           |
| `description`   | none                                                     | ≤ 500 characters.                                                              |
| `trigger`       | `{manual: true}`                                         | See below.                                                                     |
| `concurrency`   | `{key: "{automation}", max_runs: 1}`                     | Runs sharing a key serialize; extra firings skip `already_running`.            |
| `target`        | required                                                 | `workspace`, `repos`, `base`, `task`, `existing_task`, `branch`.               |
| `agent`         | required with agent steps                                | Default agent for agent steps.                                                 |
| `steps`         | one agent step `main`                                    | The pipeline every item runs.                                                  |
| `selector`      | none                                                     | Present ⇒ campaign.                                                            |
| `tracker`       | none                                                     | `linear: {team, project, assignee, labels, milestone, title, marker, states}`. |
| `pools`         | `{active: 1, open_prs: 3}`                               | Built-in caps plus global lease pools from `~/.worktreemanager/pools.toml`.    |
| `budgets`       | see below                                                | Per-item limits.                                                               |
| `authorization` | `{allow: [], deny: []}`                                  | Verbs; `caps` per verb (default 20 per item).                                  |
| `report`        | `{inbox: on_findings, archive_noop: true, notify: true}` | When runs reach the Inbox.                                                     |
| `completion`    | none                                                     | `selector_empty`, `stop_after`, `max_runs`.                                    |

## Routine or campaign

- **Routine**: no `selector`; each run has one implicit item. `target.task`: `per-run` (new task, branch `wtm/{automation}/{run.short_id}`) or `existing` (with `existing_task`, or the event's task).
- **Campaign**: `selector.run` is an argv run in the automation folder. It reads `{free, active, busy_files, cursor, run}` on stdin and prints `{"items": [{"id", "version", "title"?, "data"?}], "cursor"?}` or `{"run": false, "reason"?}`. `target.task: per-item`; with `tracker`, each item gets a Linear issue and works on its branch. Other selector fields: `timeout` (`5m`), `max_new_items`, `exclusive_on` (`item.data` keys that may not overlap between active items).

## Triggers

Present triggers are OR-ed; at least one must allow firing.

- `manual` (default `true`): Run now and `wtm automation run`. Test runs are always available.
- `schedule`: a cron string or `{cron, tz (default America/Santiago), catch_up: latest | skip}`; slots closer than 15 minutes are an error.
- `cli: true`: `wtm automation fire <name> --key <key> [--payload <json|@file>]`, once per key.
- `event`: a list of `{source, type, where}`:
  - `app`: `task.created`, `task.deleted`, `agent.done`, `agent.waiting`, `pr.merged`, `pr.closed`, `pr.reopened`, `pr.ready`, `pr.checks_passed`, `pr.checks_failed`, `pr.approved`, `pr.changes_requested`; `where: {workspace, agent}`.
  - `linear`: `issue.created`, `issue.label_added`, `issue.state_changed`; `where: {team, label, assignee, state}`.
  - `github`: `pr.opened`, `pr.updated`, `pr.ci_failed`, `pr.review_received`, `pr.mentioned`; `where: {repo, author, branch, label}`.
  - `circleci`: `workflow.failed`, `workflow.finished`, `job.failed`, `job.finished`; `where: {project, branch, author}`.

Payloads from events and the CLI are untrusted data.

## Agent

`id` (from the agent catalog, such as `claude` or `codex`), `model` (`default`), `effort`, `permission` (`auto` | `strict`, or `{mode, allow, deny, pending_timeout: 10m, pending_default: deny}`). Steps override any field. Features need agent capabilities: `readonly` needs `read_only`, `result` needs `structured_output`, rules and granted verbs need `tool_rules`.

## Steps

Each step has an `id` (`^[a-z][a-z0-9_]{0,40}$`) and exactly one kind key. Common fields: `label` (board column), `next`, `on_fail` (default `fail`), `timeout`, `uses` (lease pools), `capacity` (`hold` | `release`).

Transitions (`on_fail`, `on_reject`): `fail`, `block`, `defer`, `continue` or `{goto: <step>, max: <n or budget name>}`. Cycles need a `max`.

| Kind       | Fields                                                                                                                                                            | Default timeout |
| ---------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------- |
| `agent`    | `skill`, `prompt` (not templated), agent overrides, `fresh_session`, `readonly` (needs `fresh_session: true`), `result: verdict` or `schemas/….json`, `on_reject` | `60m`           |
| `command`  | argv list of templates (`scripts/…`, absolute path or `PATH` name), `repo`, `env`, `bind: git_fingerprint`, `after`                                               | `30m`           |
| `wait`     | exactly one of `time`, `signal` or `event: {watcher: pr_checks \| pr_integrated \| command, …}`                                                                   | none            |
| `approval` | `title` (template), `detail`, `params`, `default: approve \| reject` (required with the step's `timeout`), `on_reject`                                            | none            |
| `publish`  | one effect or a list, each `name` or `{name: {params}}`                                                                                                           | `30m`           |

- Agent steps of an item share one session unless `fresh_session: true`. The `verdict` result is `{verdict: pass | reject | blocked | defer | noop, summary, findings}`: `reject` takes `on_reject`, `blocked`/`defer` rest the item, `noop` succeeds silently. Without `result`, a final message `NOOP` is a noop.
- Commands get `WTM_AUTOMATION`, `WTM_RUN_ID`, `WTM_ITEM_ID`, `WTM_ITEM_VERSION`, `WTM_ITEM_DATA` (a JSON file), `WTM_TASK_ID`, `WTM_WORKTREE` and lease variables. Exit 0 succeeds.
- Watchers: `pr_checks` (`required`, `ignore`, `expect`, `bot_lgtm`, `unresolved_threads`, `request_review_comment`) waits for green CI on the pushed HEAD; `pr_integrated` waits for the PR to land; `command` polls a script that prints `{"status": "pending" | "success" | "fail"}`.
- Answer signals and approvals with `wtm automation signal <run_id> <step> <json>`.

### Publish effects

| Effect                  | Verb           | Params                                                                 |
| ----------------------- | -------------- | ---------------------------------------------------------------------- |
| `git.push`              | `push`         | `repo`, `force_with_lease` (also needs `force_push`)                   |
| `github.pr_create`      | `pr_create`    | `repo`, `draft` (`true`), `title`, `body_template`, `lang`, `assignee` |
| `github.pr_update`      | `pr_update`    | `repo`, `title`, `body_template`, `lang`, `draft`                      |
| `github.pr_comment`     | `pr_comment`   | `repo`, `body`                                                         |
| `github.pr_ready`       | `pr_ready`     | `repo`                                                                 |
| `linear.upsert`         | `linear_write` | overrides of `tracker.linear` fields                                   |
| `linear.state`          | `linear_write` | `state`                                                                |
| `wtm.task_create`       | `task_create`  | none                                                                   |
| `wtm.task_log`          | `task_log`     | `entry`                                                                |
| `wtm.task_delete_clean` | `task_delete`  | none                                                                   |

Effects are idempotent (reconciled before retries) and staged in Test runs.

## Authorization

Verbs: `commit`, `push`, `force_push`, `pr_create`, `pr_update`, `pr_comment`, `pr_reply`, `pr_ready`, `linear_write`, `task_create`, `task_delete`, `task_log`. `deny` wins over `allow`; `merge` and `no_verify` are never allowed. Every publish effect needs its verb. Allowed verbs also become agent tool rules (for example `commit` allows `git add` and `git commit`); everything else is denied and surfaces in the Inbox.

## Budgets

| Field           | Default | Meaning                                                       |
| --------------- | ------- | ------------------------------------------------------------- |
| `active_time`   | `60m`   | Per item, excluding waits, approvals and pauses.              |
| `repair_rounds` | `3`     | Named budget for `goto` transitions.                          |
| `wait_per_head` | `60m`   | Default timeout for `pr_checks` and `command` waits per HEAD. |
| `run_deadline`  | `2h`    | Routines: deadline of a run.                                  |

Exhausting a budget blocks the item with `budget:<name>`.

## Examples

### Routine: weekly dependency report

`scripts/outdated.sh` prints the repository's outdated dependencies; the agent reads its log from the step context and only reports.

```markdown
---
schema: 1
name: dependency-report
description: Weekly report of outdated dependencies in api.
trigger:
  schedule: { cron: "0 8 * * 1", tz: America/Santiago }
target:
  workspace: Acme
  repos: [api]
  task: per-run
agent: { id: claude, effort: medium }
steps:
  - id: collect
    command: [scripts/outdated.sh]
    timeout: 10m
  - id: summarize
    agent:
      readonly: true
      fresh_session: true
      result: verdict
budgets: { run_deadline: 30m }
report: { inbox: on_findings }
---

Read the log of the `collect` step. Return verdict `noop` when nothing is outdated. Otherwise return `pass` with one finding per outdated dependency that has a major update or a security advisory: name, current and latest version, and why it matters. Do not change files.
```

### Campaign: broken links

`scripts/broken-links.sh` follows the selector contract: one item per docs page with broken links, `data.files` the page, `version` a hash of its broken links. `scripts/check-links.sh` exits non-zero while the page still has broken links.

```markdown
---
schema: 1
name: docs-links
description: Fix broken links in api's docs, one draft PR per page.
trigger:
  manual: true
target:
  workspace: Acme
  repos: [api]
  task: per-item
agent: { id: claude, effort: high }
selector:
  run: [scripts/broken-links.sh]
  max_new_items: 2
  exclusive_on: [files]
pools: { active: 2, open_prs: 2 }
budgets: { active_time: 30m, repair_rounds: 2 }
authorization:
  allow: [commit, push, pr_create, task_delete]
steps:
  - id: fix
    agent: { prompt: "Fix only the links listed in the item data. Commit; do not push." }
  - id: verify
    command: [scripts/check-links.sh, "{item.id}"]
    bind: git_fingerprint
    on_fail: { goto: fix, max: repair_rounds }
  - id: approve
    timeout: 2d
    approval:
      title: "Publish link fixes for {item.title}?"
      default: reject
      on_reject: block
  - id: publish
    publish:
      - git.push
      - github.pr_create: { draft: true, title: "docs: fix links in {item.title}" }
  - id: merged
    label: awaiting_review
    wait: { event: { watcher: pr_integrated } }
  - id: cleanup
    publish: wtm.task_delete_clean
completion: { selector_empty: true }
---

Each item is one docs page. Replace or remove its broken links, keeping the surrounding text. Never touch other pages.
```
