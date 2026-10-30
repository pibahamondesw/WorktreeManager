//! What a session may do without asking. Interactive sessions ask the user, as the chat always
//! has; unattended ones run with explicit tool rules and resolve any request still pending after
//! a timeout with a default, so no one has to watch them. Each adapter translates the policy into
//! its agent's launch flags; the chat session only applies the timeout.

use std::time::Duration;

use serde::{Deserialize, Serialize, Serializer};

const DEFAULT_PENDING_TIMEOUT: Duration = Duration::from_secs(10 * 60);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    Interactive,
    Unattended,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PendingDefault {
    Allow,
    #[default]
    Deny,
}

/// How long a pending request waits for an answer before resolving with `default`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PendingResolution {
    pub timeout: Duration,
    pub default: PendingDefault,
}

/// Agent-native tool rules: `allow` runs without asking, `deny` never runs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnattendedPolicy {
    pub allow: Vec<String>,
    pub deny: Vec<String>,
    #[serde(rename = "pendingTimeoutSecs", serialize_with = "whole_seconds")]
    pub pending_timeout: Duration,
    pub pending_default: PendingDefault,
}

fn whole_seconds<S: Serializer>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_u64(duration.as_secs())
}

impl Default for UnattendedPolicy {
    fn default() -> Self {
        Self {
            allow: Vec::new(),
            deny: Vec::new(),
            pending_timeout: DEFAULT_PENDING_TIMEOUT,
            pending_default: PendingDefault::default(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum PermissionPolicy {
    #[default]
    Interactive,
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "the automation engine (WOR-249) wraps definition policies"
        )
    )]
    Unattended(UnattendedPolicy),
}

impl PermissionPolicy {
    pub fn mode(&self) -> PermissionMode {
        match self {
            Self::Interactive => PermissionMode::Interactive,
            Self::Unattended(_) => PermissionMode::Unattended,
        }
    }

    /// Interactive requests wait for the user however long it takes.
    pub fn pending_resolution(&self) -> Option<PendingResolution> {
        match self {
            Self::Interactive => None,
            Self::Unattended(policy) => Some(PendingResolution {
                timeout: policy.pending_timeout,
                default: policy.pending_default,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unattended_requests_default_to_deny_after_ten_minutes() {
        let policy = PermissionPolicy::Unattended(UnattendedPolicy::default());
        assert_eq!(policy.mode(), PermissionMode::Unattended);
        assert_eq!(
            policy.pending_resolution(),
            Some(PendingResolution {
                timeout: Duration::from_secs(600),
                default: PendingDefault::Deny,
            })
        );
    }

    #[test]
    fn interactive_requests_never_time_out() {
        let policy = PermissionPolicy::default();
        assert_eq!(policy.mode(), PermissionMode::Interactive);
        assert_eq!(policy.pending_resolution(), None);
    }

    #[test]
    fn pending_defaults_read_as_in_definitions() {
        let parsed: Vec<PendingDefault> = serde_json::from_str(r#"["allow", "deny"]"#).unwrap();
        assert_eq!(parsed, [PendingDefault::Allow, PendingDefault::Deny]);
        assert_eq!(
            serde_json::to_value(PermissionMode::Unattended).unwrap(),
            "unattended"
        );
    }

    #[test]
    fn unattended_policies_serialize_their_timeout_in_seconds() {
        assert_eq!(
            serde_json::to_value(UnattendedPolicy::default()).unwrap(),
            serde_json::json!({
                "allow": [],
                "deny": [],
                "pendingTimeoutSecs": 600,
                "pendingDefault": "deny"
            })
        );
    }
}
