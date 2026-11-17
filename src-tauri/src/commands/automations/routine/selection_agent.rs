//! `selector.agent`: one headless agent turn outside any task, in the run's scratch directory,
//! whose structured result is the selection pass's items. Its session, reply and the requests its
//! policy settled are recorded in the run log as the selection pass's, under no item.

use std::fs;
use std::time::Duration;

use super::super::definition::AgentSource;
use super::super::pipeline::RunLog;
use super::super::scheduler::RunRequest;
use super::super::selector::agent::{read_result, result_schema};
use super::super::selector::{Selector, SelectorInput, SelectorOutput};
use super::super::store::events::{RunEvent, SELECTION_ITEM, SELECTOR_STEP};
use super::commands::scratch_dir;
use super::prompt::{selector_prompt, SelectorPrompt};
use super::{
    settled_event, ResolvedTask, RoutineExecutor, SessionLaunch, SettledAsk, WriterLog,
    SESSION_NAME_CHARS,
};
use crate::commands::chat::{truncate_to, TurnEnd};

const ATTEMPT: u32 = 0;

pub(super) struct AgentSelector<'a> {
    pub executor: &'a RoutineExecutor,
    pub source: &'a AgentSource,
    pub request: &'a RunRequest,
    pub timeout: Duration,
}

impl Selector for AgentSelector<'_> {
    fn select(&self, input: &SelectorInput) -> Result<SelectorOutput, String> {
        let workplace = self.workplace()?;
        let launch = SessionLaunch {
            agent: self.source.agent.clone(),
            grants: Vec::new(),
            read_only: self.source.readonly,
            output_schema: Some(result_schema()),
            resume: None,
            env: Vec::new(),
            name: truncate_to(
                &format!("{} · selector", self.request.automation),
                SESSION_NAME_CHARS,
            ),
        };
        let sessions = &self.executor.sessions;
        let session_id = sessions
            .start(&workplace, &launch)
            .map_err(|error| format!("selector agent did not start: {error}"))?;
        let selected = self.turn(&session_id, &workplace.folders[0], input);
        sessions.close(&session_id);
        selected
    }
}

impl AgentSelector<'_> {
    /// The scratch directory of the run for the pass, as a step without a task has one.
    fn workplace(&self) -> Result<ResolvedTask, String> {
        let request = self.request;
        let run_dir = self
            .executor
            .store
            .run_dir(&request.automation, &request.run_id)?;
        let directory = scratch_dir(&run_dir, SELECTION_ITEM);
        fs::create_dir_all(&directory)
            .map_err(|error| format!("{}: {error}", directory.display()))?;
        Ok(ResolvedTask::scratch(&directory))
    }

    fn turn(
        &self,
        session_id: &str,
        workplace: &str,
        input: &SelectorInput,
    ) -> Result<SelectorOutput, String> {
        let executor = self.executor;
        let automation = &self.request.automation;
        let mut log = WriterLog::of(&executor.store, self.request);
        let conversation = self.record_session(&mut log, session_id)?;
        let decisions = executor.store.state(automation)?.decisions;
        let prompt = selector_prompt(&SelectorPrompt {
            source: self.source,
            request: self.request,
            workplace,
            input,
            decisions: &decisions,
        });
        let live = executor
            .live_turns
            .running(session_id, automation, &self.source.agent.id);
        let end = executor
            .sessions
            .run_turn(session_id, &prompt, self.timeout);
        drop(live);
        let late_conversation = conversation.is_none();
        late_conversation
            .then(|| self.record_session(&mut log, session_id))
            .transpose()?;
        self.record_settled_requests(&mut log, session_id)?;
        let reply = match end {
            TurnEnd::Completed {
                reply,
                error: false,
            } => reply,
            TurnEnd::Completed { reply, error: true } => {
                record_reply(&mut log, reply.clone(), None)?;
                let reason = reply.unwrap_or_default();
                return Err(format!("selector agent failed: {}", reason.trim()));
            }
            TurnEnd::Ended { message, .. } => {
                let reason = message.unwrap_or_else(|| "without a reply".into());
                return Err(format!("selector agent exited: {reason}"));
            }
            TurnEnd::TimedOut => {
                let secs = self.timeout.as_secs();
                return Err(format!("selector agent timed out after {secs}s"));
            }
        };
        match read_result(reply.as_deref()) {
            Ok(selection) => {
                record_reply(&mut log, reply, Some(selection.result))?;
                Ok(selection.output)
            }
            Err(error) => {
                record_reply(&mut log, reply, None)?;
                Err(format!("selector agent result: {error}"))
            }
        }
    }

    /// Records the session and the conversation it holds so far.
    fn record_session(
        &self,
        log: &mut WriterLog,
        session_id: &str,
    ) -> Result<Option<String>, String> {
        let conversation = self.executor.sessions.conversation(session_id);
        log.append(RunEvent::SessionStarted {
            item: SELECTION_ITEM.into(),
            step: SELECTOR_STEP.into(),
            attempt: ATTEMPT,
            session_id: session_id.to_string(),
            conversation_id: conversation.clone(),
        })?;
        Ok(conversation)
    }

    /// Logs what the agent asked for and its policy settled, for the Inbox and the owner.
    fn record_settled_requests(&self, log: &mut WriterLog, session_id: &str) -> Result<(), String> {
        let asking = SettledAsk {
            item: SELECTION_ITEM,
            step: SELECTOR_STEP,
            attempt: ATTEMPT,
            session_id,
            mode: self.source.agent.permission.mode,
        };
        let settled = self.executor.sessions.settled_requests(session_id);
        for request in &settled {
            log.append(settled_event(&asking, request))?;
        }
        Ok(())
    }
}

fn record_reply(
    log: &mut WriterLog,
    reply: Option<String>,
    result: Option<serde_json::Value>,
) -> Result<(), String> {
    log.append(RunEvent::StepOutput {
        item: SELECTION_ITEM.into(),
        step: SELECTOR_STEP.into(),
        attempt: ATTEMPT,
        reply: reply.as_deref().map(super::truncated),
        result,
    })?;
    Ok(())
}
