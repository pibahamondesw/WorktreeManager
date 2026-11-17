use super::*;

fn agent_source(selector: &str) -> (AgentSource, u64) {
    let entry = load_campaign(&campaign(selector));
    assert_eq!(entry.errors, Vec::new(), "{selector}");
    let selector = entry.definition.unwrap().selector.unwrap();
    let ItemSource::Agent(source) = selector.source else {
        panic!("expected an agent source")
    };
    (source, selector.timeout_secs)
}

#[test]
fn an_agent_selector_takes_the_top_level_agent_and_reads_only_by_default() {
    let (source, timeout) = agent_source("selector:\n  agent: { prompt: Pick the next issues. }\n");

    assert_eq!(source.agent.id, "claude");
    assert_eq!(source.agent.model, "default");
    assert_eq!(source.prompt.as_deref(), Some("Pick the next issues."));
    assert_eq!(source.skill, None);
    assert!(source.readonly);
    assert_eq!(timeout, 15 * 60);

    let (source, timeout) = agent_source(
        "selector:\n  agent: { model: opus, effort: high, permission: strict, skill: triage, readonly: false }\n  timeout: 30m\n",
    );
    assert_eq!(
        (source.agent.model.as_str(), source.agent.effort.as_deref()),
        ("opus", Some("high"))
    );
    assert_eq!(
        source.agent.permission.mode,
        crate::commands::agents::permission::UnattendedMode::Strict
    );
    assert_eq!(source.skill.as_deref(), Some("triage"));
    assert!(!source.readonly);
    assert_eq!(timeout, 30 * 60);
}

#[test]
fn an_agent_selector_needs_instructions_and_the_capabilities_it_uses() {
    let has = |selector: &str, field: &str, kind: IssueKind| {
        let issues = source_issues(&campaign(selector));
        assert!(
            issues.contains(&(field.into(), kind)),
            "{selector}: {issues:?}"
        );
    };
    has(
        "selector:\n  agent: {}\n",
        "selector.agent.prompt",
        IssueKind::Missing,
    );
    has(
        "selector:\n  agent: { id: plain, prompt: x }\n",
        "selector.agent",
        IssueKind::Capability,
    );
    has(
        "selector:\n  agent: { id: nobody, prompt: x }\n",
        "selector.agent.id",
        IssueKind::UnknownAgent,
    );
    has(
        "selector:\n  agent: { prompt: x, fresh_session: true }\n",
        "selector.agent.fresh_session",
        IssueKind::UnknownField,
    );
    has(
        "selector:\n  run: [scripts/queue.sh]\n  agent: { prompt: x }\n",
        "selector.run",
        IssueKind::Invalid,
    );
    let plain = source_issues(&campaign(
        "selector:\n  agent: { id: plain, prompt: x, readonly: false }\n",
    ));
    let capability = load_campaign(&campaign(
        "selector:\n  agent: { id: plain, prompt: x, readonly: false }\n",
    ))
    .errors
    .into_iter()
    .find(|issue| issue.kind == IssueKind::Capability)
    .unwrap();
    assert_eq!(plain.len(), 1, "{plain:?}");
    assert_eq!(
        capability.message,
        "plain lacks the structured_output capability the selector needs"
    );
}
