//! Development-only agent that replays a recorded Claude Code stream-json transcript. Its chat
//! relaunches this executable with `--replay-agent`, which answers the app like the `claude` CLI
//! would: each user message plays the next turn of the transcript (one turn per `result`, looping
//! back to the first), text and thinking stream word by word, and a recorded `control_request`
//! waits for the app's answer before the turn goes on. The app reads it with `ClaudeProvider`, so
//! the chat runs through the same adapter and UI as a real session. Answers never change what is
//! replayed. `WTM_REPLAY_TRANSCRIPT` points at a transcript of your own; the built-in one is used
//! otherwise.

use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};

use super::permission::PermissionMode;
use super::{
    AgentAdapter, AgentCapabilities, ChatRequest, ConversationFork, ForkMethod, LaunchContext,
    LaunchSpec,
};
use crate::commands::agent_alerts::AlertHooks;
use crate::commands::agent_usage::{CachedUsage, UsageFile, UsageSource};
use crate::commands::chat::claude::ClaudeProvider;
use crate::commands::chat::{ChatItem, Launch, SessionList};

pub(crate) const REPLAY_FLAG: &str = "--replay-agent";
const TRANSCRIPT_ENV: &str = "WTM_REPLAY_TRANSCRIPT";
const BUILT_IN_TRANSCRIPT: &str = include_str!("replay/transcript.jsonl");
const PACE: Pace = Pace {
    line: Duration::from_millis(300),
    word: Duration::from_millis(25),
};

pub(crate) struct Replay {
    program: PathBuf,
    transcript: Option<PathBuf>,
}

impl Replay {
    pub(crate) fn new(program: PathBuf, transcript: Option<PathBuf>) -> Self {
        Self {
            program,
            transcript,
        }
    }

    pub(crate) fn from_env() -> Self {
        Self::new(
            std::env::current_exe().unwrap_or_default(),
            std::env::var_os(TRANSCRIPT_ENV)
                .filter(|path| !path.is_empty())
                .map(PathBuf::from),
        )
    }
}

impl AgentAdapter for Replay {
    fn id(&self) -> &'static str {
        "replay"
    }

    fn label(&self) -> &'static str {
        "Replay"
    }

    fn installed(&self) -> bool {
        true
    }

    fn capabilities(&self) -> AgentCapabilities {
        AgentCapabilities {
            fork: None,
            review: false,
            rename: false,
            archive: false,
            models: false,
            effort: false,
            permission_modes: &[PermissionMode::Interactive],
            interrupt: false,
            steer: false,
            compact: false,
            images: false,
            headless: true,
            tool_rules: false,
            classifier_approval: false,
            read_only: false,
            structured_output: false,
        }
    }

    fn chat_launch(&self, request: ChatRequest) -> Result<Launch, String> {
        let mut command = Command::new(&self.program);
        crate::platform::hide_console(&mut command)
            .arg(REPLAY_FLAG)
            .args(&self.transcript)
            .current_dir(&request.cwd);
        let session_id = request
            .resume
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        Ok(Launch {
            command,
            provider: Box::new(ClaudeProvider::new(session_id).continuing(request.handoff)),
            history: Vec::new(),
        })
    }

    fn fork_method(&self) -> ForkMethod {
        ForkMethod::Relaunch
    }

    fn terminal_launch(&self, _ctx: &LaunchContext) -> Result<LaunchSpec, String> {
        Err("The replay agent only runs in chat".into())
    }

    fn sessions(&self, _cwd: &str, _current: Option<&str>) -> Result<SessionList, String> {
        Ok(SessionList {
            sessions: Vec::new(),
            can_rename: false,
            can_archive: false,
        })
    }

    fn history(
        &self,
        _cwd: &str,
        _id: &str,
        _fork: Option<&ConversationFork>,
    ) -> Result<Vec<ChatItem>, String> {
        Ok(Vec::new())
    }

    fn rename_session(&self, _cwd: &str, _id: &str, _title: &str) -> Result<(), String> {
        Err("Replay conversations cannot be renamed".into())
    }

    fn live_session_renames_itself(&self) -> bool {
        false
    }

    fn alert_hooks(&self) -> AlertHooks {
        AlertHooks {
            agent: "replay",
            config: std::env::temp_dir().join("wtm-replay-agent-hooks.json"),
            specs: &[],
            async_handlers: false,
        }
    }

    fn usage_source(&self) -> Box<dyn UsageSource> {
        Box::new(NoUsage)
    }
}

struct NoUsage;

impl UsageSource for NoUsage {
    fn latest(&self) -> Result<Option<UsageFile>, String> {
        Ok(None)
    }

    fn read(&self, _file: &UsageFile, _now: i64) -> Result<CachedUsage, String> {
        Err("The replay agent records no usage".into())
    }
}

/// Runs as the replay agent when this executable was launched with `--replay-agent`.
pub(crate) fn run_if_invoked<R: Read, W: Write>(
    args: &[String],
    input: impl FnOnce() -> R,
    output: impl FnOnce() -> W,
) -> Option<i32> {
    (args.first()? == REPLAY_FLAG).then(|| {
        let replayed = Transcript::load(args.get(1).map(Path::new)).and_then(|transcript| {
            transcript
                .replay(BufReader::new(input()), output(), PACE)
                .map_err(|error| error.to_string())
        });
        match replayed {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("replay: {error}");
                1
            }
        }
    })
}

#[derive(Clone, Copy)]
struct Pace {
    line: Duration,
    word: Duration,
}

struct Transcript {
    turns: Vec<Vec<Value>>,
}

impl Transcript {
    fn load(path: Option<&Path>) -> Result<Self, String> {
        let text = match path {
            Some(path) => fs::read_to_string(path).map_err(|error| {
                format!("Could not read the transcript {}: {error}", path.display())
            })?,
            None => BUILT_IN_TRANSCRIPT.to_string(),
        };
        Self::parse(&text)
    }

    fn parse(text: &str) -> Result<Self, String> {
        let mut turns = vec![Vec::new()];
        for (index, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let message: Value = serde_json::from_str(line)
                .map_err(|_| format!("Line {} of the transcript is not JSON", index + 1))?;
            let ends_turn = message["type"] == "result";
            turns.last_mut().unwrap().push(message);
            if ends_turn {
                turns.push(Vec::new());
            }
        }
        turns.retain(|turn| !turn.is_empty());
        if turns.is_empty() {
            return Err("The transcript has no messages".into());
        }
        Ok(Self { turns })
    }

    fn replay(&self, input: impl BufRead, output: impl Write, pace: Pace) -> io::Result<()> {
        let mut player = Player {
            input: input.lines(),
            output,
            pace,
            streamed: None,
        };
        let mut lap = 0;
        loop {
            for turn in &self.turns {
                if !player.wait_for("user")? {
                    return Ok(());
                }
                for message in turn {
                    if !player.play(&relabeled(message, lap))? {
                        return Ok(());
                    }
                }
            }
            lap += 1;
        }
    }
}

/// Later laps suffix every recorded id, so the chat shows new items instead of rewriting the
/// first lap's.
fn relabeled(message: &Value, lap: usize) -> Value {
    let mut message = message.clone();
    if lap == 0 {
        return message;
    }
    let suffix = format!("#{}", lap + 1);
    let relabel = |id: Option<&mut Value>| {
        if let Some(id) = id.filter(|id| id.is_string()) {
            *id = format!("{}{suffix}", id.as_str().unwrap_or_default()).into();
        }
    };
    for pointer in ["/message/id", "/request_id", "/request/tool_use_id"] {
        relabel(message.pointer_mut(pointer));
    }
    let content = message.pointer_mut("/message/content");
    for block in content.and_then(Value::as_array_mut).into_iter().flatten() {
        relabel(block.get_mut("id"));
        relabel(block.get_mut("tool_use_id"));
    }
    message
}

struct Player<L, W> {
    input: L,
    output: W,
    pace: Pace,
    /// The assistant message being streamed and how many of its blocks were streamed so far.
    streamed: Option<(String, usize)>,
}

impl<L: Iterator<Item = io::Result<String>>, W: Write> Player<L, W> {
    /// Reads the app's lines until one of type `kind`, answering its control requests on the way.
    /// `false` when the app closed the input.
    fn wait_for(&mut self, kind: &str) -> io::Result<bool> {
        while let Some(line) = self.input.next() {
            let message: Value = serde_json::from_str(&line?).unwrap_or_default();
            if message["type"] == kind {
                return Ok(true);
            }
            if message["type"] == "control_request" {
                let answer = json!({
                    "type": "control_response",
                    "response": {
                        "subtype": "success",
                        "request_id": message["request_id"],
                        "response": {},
                    },
                });
                self.write(&answer)?;
            }
        }
        Ok(false)
    }

    /// Recorded stream events are dropped: the assistant messages are streamed again instead.
    fn play(&mut self, message: &Value) -> io::Result<bool> {
        match message["type"].as_str() {
            Some("stream_event") => return Ok(true),
            Some("assistant") => {
                thread::sleep(self.pace.line);
                self.stream(&message["message"])?;
            }
            _ => thread::sleep(self.pace.line),
        }
        self.write(message)?;
        if message["type"] == "control_request" {
            return self.wait_for("control_response");
        }
        Ok(true)
    }

    fn stream(&mut self, message: &Value) -> io::Result<()> {
        let id = message["id"].as_str().unwrap_or_default().to_string();
        let mut index = match &self.streamed {
            Some((streamed, blocks)) if *streamed == id => *blocks,
            _ => {
                self.event(json!({ "type": "message_start", "message": { "id": id } }))?;
                0
            }
        };
        for block in message["content"].as_array().into_iter().flatten() {
            if let Some(field) = streamed_field(block) {
                self.stream_block(index, block, field)?;
            }
            index += 1;
        }
        self.streamed = Some((id, index));
        Ok(())
    }

    fn stream_block(&mut self, index: usize, block: &Value, field: &str) -> io::Result<()> {
        let start = json!({
            "type": "content_block_start",
            "index": index,
            "content_block": { "type": block["type"] },
        });
        self.event(start)?;
        for word in block[field]
            .as_str()
            .unwrap_or_default()
            .split_inclusive(' ')
        {
            thread::sleep(self.pace.word);
            let delta = json!({
                "type": "content_block_delta",
                "index": index,
                "delta": { "type": format!("{field}_delta"), field: word },
            });
            self.event(delta)?;
        }
        self.event(json!({ "type": "content_block_stop", "index": index }))
    }

    fn event(&mut self, event: Value) -> io::Result<()> {
        self.write(&json!({ "type": "stream_event", "event": event }))
    }

    fn write(&mut self, message: &Value) -> io::Result<()> {
        writeln!(self.output, "{message}")?;
        self.output.flush()
    }
}

fn streamed_field(block: &Value) -> Option<&'static str> {
    match block["type"].as_str()? {
        "text" => Some("text"),
        "thinking" => Some("thinking"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agents::permission::PermissionPolicy;
    use crate::commands::agents::Agents;
    use crate::commands::chat::test_support::{ChatProvider, ChatSnapshot, ProviderOutput};
    use crate::commands::chat::ItemKind;

    const INSTANT: Pace = Pace {
        line: Duration::ZERO,
        word: Duration::ZERO,
    };

    fn replay_lines(transcript: &Transcript, input: &[Value]) -> Vec<Value> {
        let input: String = input.iter().map(|line| format!("{line}\n")).collect();
        let mut output = Vec::new();
        transcript
            .replay(input.as_bytes(), &mut output, INSTANT)
            .unwrap();
        String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn user(text: &str) -> Value {
        json!({ "type": "user", "message": { "role": "user", "content": text } })
    }

    fn approve(id: &str) -> Value {
        json!({ "type": "control_response", "response": { "subtype": "success", "request_id": id } })
    }

    fn parsed(lines: &[Value]) -> Transcript {
        let text: String = lines.iter().map(|line| format!("{line}\n\n")).collect();
        Transcript::parse(&text).unwrap()
    }

    fn assistant_text(id: &str, text: &str) -> Value {
        json!({ "type": "assistant", "message": { "id": id, "content": [{ "type": "text", "text": text }] } })
    }

    fn done() -> Value {
        json!({ "type": "result", "subtype": "success", "is_error": false })
    }

    fn request(kind: &str) -> ChatRequest {
        ChatRequest {
            cwd: "/tmp".into(),
            extra_dirs: Vec::new(),
            resume: None,
            pinned: false,
            fork: None,
            handoff: None,
            permission: PermissionPolicy::Interactive,
            output_schema: None,
            name: Some(kind.into()),
        }
    }

    #[test]
    fn waits_for_a_user_message_and_answers_control_requests_meanwhile() {
        let transcript = parsed(&[done()]);
        let lines = replay_lines(
            &transcript,
            &[
                json!({ "type": "control_request", "request_id": "wm-1", "request": { "subtype": "initialize" } }),
                json!({ "type": "other" }),
            ],
        );
        assert_eq!(
            lines,
            [json!({ "type": "control_response", "response": {
                "subtype": "success", "request_id": "wm-1", "response": {},
            }})]
        );
    }

    #[test]
    fn ignores_input_that_is_not_json() {
        let mut output = Vec::new();
        parsed(&[done()])
            .replay("not json\n".as_bytes(), &mut output, INSTANT)
            .unwrap();
        assert!(output.is_empty());
    }

    #[test]
    fn streams_text_word_by_word_before_the_assistant_message() {
        let message = assistant_text("m1", "Hello there");
        let transcript = parsed(&[message.clone(), done()]);
        let lines = replay_lines(&transcript, &[user("hi")]);
        let events: Vec<_> = lines.iter().map(|line| line["event"].clone()).collect();
        assert_eq!(
            events[0],
            json!({ "type": "message_start", "message": { "id": "m1" } })
        );
        assert_eq!(events[1]["type"], "content_block_start");
        assert_eq!(
            events[2]["delta"],
            json!({ "type": "text_delta", "text": "Hello " })
        );
        assert_eq!(
            events[3]["delta"],
            json!({ "type": "text_delta", "text": "there" })
        );
        assert_eq!(
            events[4],
            json!({ "type": "content_block_stop", "index": 0 })
        );
        assert_eq!(lines[5], message);
        assert_eq!(lines[6], done());
    }

    #[test]
    fn numbers_blocks_across_messages_that_share_an_id() {
        let thinking = json!({ "type": "assistant", "message": { "id": "m1", "content": [
            { "type": "thinking", "thinking": "Hmm" },
            { "type": "tool_use", "id": "t1", "name": "Read", "input": {} },
        ]}});
        let transcript = parsed(&[thinking, assistant_text("m1", "Done"), done()]);
        let lines = replay_lines(&transcript, &[user("hi")]);
        let starts: Vec<_> = lines
            .iter()
            .filter(|line| line["event"]["type"] == "message_start")
            .collect();
        assert_eq!(starts.len(), 1);
        let deltas: Vec<_> = lines
            .iter()
            .filter(|line| line["event"]["type"] == "content_block_delta")
            .map(|line| {
                (
                    line["event"]["index"].clone(),
                    line["event"]["delta"].clone(),
                )
            })
            .collect();
        assert_eq!(
            deltas,
            [
                (
                    json!(0),
                    json!({ "type": "thinking_delta", "thinking": "Hmm" })
                ),
                (json!(2), json!({ "type": "text_delta", "text": "Done" })),
            ]
        );
    }

    #[test]
    fn drops_recorded_stream_events() {
        let recorded = json!({ "type": "stream_event", "event": { "type": "message_start" } });
        let lines = replay_lines(&parsed(&[recorded, done()]), &[user("hi")]);
        assert_eq!(lines, [done()]);
    }

    #[test]
    fn a_recorded_control_request_waits_for_the_app_answer() {
        let ask = json!({ "type": "control_request", "request_id": "r1", "request": { "subtype": "can_use_tool" } });
        let after = assistant_text("m2", "ok");
        let transcript = parsed(&[ask.clone(), after.clone(), done()]);

        let unanswered = replay_lines(&transcript, &[user("hi")]);
        assert_eq!(unanswered, std::slice::from_ref(&ask));

        let answered = replay_lines(&transcript, &[user("hi"), approve("r1")]);
        assert_eq!(answered.first(), Some(&ask));
        assert!(answered.contains(&after));
        assert_eq!(answered.last(), Some(&done()));
    }

    #[test]
    fn each_message_plays_the_next_turn_and_loops_back() {
        let first = assistant_text("m1", "one");
        let second = assistant_text("m2", "two");
        let transcript = parsed(&[first.clone(), done(), second.clone()]);
        assert_eq!(transcript.turns.len(), 2);
        let lines = replay_lines(&transcript, &[user("a"), user("b"), user("c")]);
        let messages: Vec<_> = lines
            .iter()
            .filter(|line| line["type"] == "assistant")
            .collect();
        assert_eq!(messages, [&first, &second, &assistant_text("m1#2", "one")]);
    }

    #[test]
    fn later_laps_suffix_every_recorded_id() {
        let tool = json!({ "type": "assistant", "message": { "id": "m1", "content": [
            { "type": "tool_use", "id": "t1", "name": "Read" },
        ]}});
        let output = json!({ "type": "user", "message": { "content": [
            { "type": "tool_result", "tool_use_id": "t1", "content": "ok" },
        ]}});
        let ask = json!({ "type": "control_request", "request_id": "r1", "request": {
            "subtype": "can_use_tool", "tool_use_id": "t1",
        }});
        assert_eq!(relabeled(&tool, 0), tool);
        assert_eq!(relabeled(&tool, 1)["message"]["id"], "m1#2");
        assert_eq!(relabeled(&tool, 2)["message"]["content"][0]["id"], "t1#3");
        assert_eq!(
            relabeled(&output, 1)["message"]["content"][0]["tool_use_id"],
            "t1#2"
        );
        let relabeled_ask = relabeled(&ask, 1);
        assert_eq!(relabeled_ask["request_id"], "r1#2");
        assert_eq!(relabeled_ask["request"]["tool_use_id"], "t1#2");
        assert_eq!(relabeled(&done(), 1), done());
    }

    #[test]
    fn a_transcript_must_hold_json_messages() {
        assert_eq!(
            Transcript::parse("{}\nnope").err(),
            Some("Line 2 of the transcript is not JSON".into())
        );
        assert_eq!(
            Transcript::parse("\n  \n").err(),
            Some("The transcript has no messages".into())
        );
    }

    #[test]
    fn loads_a_transcript_file_or_the_built_in_one() {
        let path = std::env::temp_dir().join(format!("wtm-replay-{}.jsonl", uuid::Uuid::new_v4()));
        fs::write(&path, format!("{}\n", done())).unwrap();
        assert_eq!(Transcript::load(Some(&path)).unwrap().turns, [vec![done()]]);
        fs::remove_file(&path).unwrap();
        let missing = Transcript::load(Some(&path)).err().unwrap();
        assert!(
            missing.starts_with("Could not read the transcript"),
            "{missing}"
        );
        assert_eq!(Transcript::load(None).unwrap().turns.len(), 4);
    }

    #[test]
    fn runs_only_when_launched_as_the_replay_agent() {
        let args = |args: &[&str]| args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>();
        let unused = || -> &[u8] { panic!("input read") };
        assert_eq!(run_if_invoked(&args(&[]), unused, Vec::new), None);
        assert_eq!(run_if_invoked(&args(&["--cli"]), unused, Vec::new), None);
        assert_eq!(
            run_if_invoked(&args(&[REPLAY_FLAG, "/missing.jsonl"]), unused, Vec::new),
            Some(1)
        );
        assert_eq!(
            run_if_invoked(&args(&[REPLAY_FLAG]), || &b""[..], Vec::new),
            Some(0)
        );
    }

    #[test]
    fn a_closed_output_fails_the_replay() {
        let input = format!("{}\n", user("hi"));
        let closed: &mut [u8] = &mut [];
        let transcript = parsed(&[assistant_text("m1", "Hello there"), done()]);
        assert!(transcript
            .replay(input.as_bytes(), closed, INSTANT)
            .is_err());
    }

    /// The built-in transcript, answered like the app would, builds the chat it is meant to show.
    #[test]
    fn the_built_in_transcript_plays_through_the_claude_provider() {
        let input = [
            user("hi"),
            user("edit"),
            approve("replay-approval"),
            user("ask"),
            approve("replay-question"),
            user("sum up"),
            user("again"),
        ];
        let lines = replay_lines(&Transcript::load(None).unwrap(), &input);
        let mut provider = ClaudeProvider::new("s1".into());
        let mut snapshot = ChatSnapshot::new(1);
        for line in lines {
            let mut out = ProviderOutput::default();
            provider.handle_line(&line.to_string(), &snapshot, &mut out);
            for event in &out.events {
                snapshot.apply(event);
            }
        }
        let kinds: Vec<_> = snapshot.items.iter().map(|item| item.kind).collect();
        for kind in [
            ItemKind::Reasoning,
            ItemKind::Assistant,
            ItemKind::Tool,
            ItemKind::Command,
            ItemKind::FileChange,
            ItemKind::Question,
        ] {
            assert!(kinds.contains(&kind), "{kind:?} missing from {kinds:?}");
        }
        let unfinished: Vec<_> = snapshot
            .items
            .iter()
            .filter(|item| {
                item.status
                    .as_deref()
                    .is_some_and(|status| status != "completed")
            })
            .collect();
        assert!(unfinished.is_empty(), "{unfinished:?}");
        let requests: Vec<_> = snapshot
            .pending
            .iter()
            .map(|request| request.id.as_str())
            .collect();
        assert_eq!(requests, ["replay-approval", "replay-question"]);
        let text = |id: &str| snapshot.item(id).map(|item| item.text.as_str());
        let first = Some("I'll look at the project layout first.");
        assert_eq!(text("msg_replay_1:1"), first);
        assert_eq!(text("msg_replay_1#2:1"), first);
        assert!(snapshot.item("toolu_replay_1#2").is_some());
        assert!(snapshot.items.iter().any(
            |item| item.kind == ItemKind::Assistant && item.text.contains("| File | Change |")
        ));
    }

    #[test]
    fn launches_this_executable_as_the_replay_agent() {
        let replay = Replay::new("/app/wtm".into(), Some("/fixtures/chat.jsonl".into()));
        let launch = replay.chat_launch(request("fresh")).unwrap();
        assert_eq!(launch.command.get_program(), "/app/wtm");
        let args: Vec<_> = launch.command.get_args().collect();
        assert_eq!(args, [REPLAY_FLAG, "/fixtures/chat.jsonl"]);
        assert_eq!(launch.command.get_current_dir(), Some(Path::new("/tmp")));
        assert!(launch.history.is_empty());

        let resumed = Replay::new("/app/wtm".into(), None)
            .chat_launch(ChatRequest {
                resume: Some("c1".into()),
                ..request("resumed")
            })
            .unwrap();
        assert_eq!(
            resumed.command.get_args().collect::<Vec<_>>(),
            [REPLAY_FLAG]
        );
        let mut provider = resumed.provider;
        let mut out = ProviderOutput::default();
        provider.start(&mut out);
        assert_eq!(out.conversation.as_deref(), Some("c1"));
    }

    #[test]
    fn reads_its_transcript_from_the_environment() {
        let replay = Replay::from_env();
        assert_eq!(replay.program, std::env::current_exe().unwrap());
        assert_eq!(
            replay.transcript,
            std::env::var_os(TRANSCRIPT_ENV)
                .filter(|path| !path.is_empty())
                .map(PathBuf::from)
        );
    }

    #[test]
    fn development_builds_list_the_replay_agent_as_a_chat_only_agent() {
        let agents = Agents::default();
        let replay = agents.get("replay").unwrap();
        let descriptor = replay.descriptor();
        assert_eq!((descriptor.label, descriptor.installed), ("Replay", true));
        assert!(descriptor.capabilities.headless);
        assert_eq!(descriptor.capabilities.fork, None);
        assert_eq!(replay.fork_method(), ForkMethod::Relaunch);
        let ctx = LaunchContext {
            canonical_dir: "/tmp",
            extra_dirs: &[],
            branch_name: None,
            session_store: Path::new("/tmp"),
        };
        assert!(replay.terminal_launch(&ctx).is_err());
        assert!(replay.sessions("/tmp", None).unwrap().sessions.is_empty());
        assert!(replay.history("/tmp", "c1", None).unwrap().is_empty());
        assert!(replay.rename_session("/tmp", "c1", "x").is_err());
        assert!(!replay.live_session_renames_itself());
        let hooks = replay.alert_hooks();
        assert_eq!((hooks.agent, hooks.specs.len()), ("replay", 0));
        assert!(!hooks.async_handlers);
    }

    #[test]
    fn records_no_usage() {
        let usage = Replay::from_env().usage_source();
        assert_eq!(usage.latest(), Ok(None));
        let file = UsageFile {
            path: PathBuf::from("/tmp/usage"),
            modified: std::time::UNIX_EPOCH,
            len: 0,
        };
        assert!(usage.read(&file, 0).is_err());
    }
}
