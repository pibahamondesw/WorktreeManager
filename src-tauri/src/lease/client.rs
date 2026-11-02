use std::collections::BTreeMap;
use std::io::{self, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::ExitStatusExt;
use std::process::Command;
use std::time::Duration;

use serde_json::{json, Map, Value};

use crate::cli_bridge::{failure, read_message, Request, VERSION};

#[derive(Debug, PartialEq)]
enum Operation {
    Run(Vec<String>),
    Acquire,
    Release,
    Status,
}

#[derive(Debug, PartialEq)]
struct Invocation {
    local: bool,
    operation: Operation,
    params: Map<String, Value>,
}

/// Handles `wtm lease …`; `None` for any other command or when help is requested.
pub fn run_if_requested(
    args: &[String],
    connect: impl FnOnce(bool) -> io::Result<UnixStream>,
) -> Option<i32> {
    let (options, command) = match args.iter().position(|arg| arg == "--") {
        Some(separator) => (&args[..separator], Some(&args[separator + 1..])),
        None => (args, None),
    };
    if options.iter().find(|arg| *arg != "--local")? != "lease"
        || options.iter().any(|arg| arg == "--help" || arg == "-h")
    {
        return None;
    }
    let invocation = match parse(options, command) {
        Ok(invocation) => invocation,
        Err(error) => {
            println!("{}", failure("invalid_args", &error));
            return Some(2);
        }
    };
    let (code, output) = match invoke(invocation, connect) {
        Ok(outcome) => outcome,
        Err(error) => (1, Some(failure("client_error", &error))),
    };
    if let Some(output) = output {
        println!("{output}");
    }
    Some(code)
}

fn parse(options: &[String], command: Option<&[String]>) -> Result<Invocation, String> {
    let mut words = Vec::new();
    let mut params = Map::new();
    let mut local = false;
    let mut options = options.iter();
    while let Some(option) = options.next() {
        let key = match option.as_str() {
            "--local" if !local => {
                local = true;
                continue;
            }
            "--pool" => "pool",
            "--holder" => "holder",
            "--pid" => "pid",
            flag if flag.starts_with('-') => {
                return Err("Unknown or duplicated option; see --help".into())
            }
            word => {
                words.push(word);
                continue;
            }
        };
        let value = options
            .next()
            .ok_or_else(|| format!("{option} requires a value"))?;
        let value = if key == "pid" {
            json!(value
                .parse::<i32>()
                .ok()
                .filter(|pid| *pid > 0)
                .ok_or("--pid requires a positive process ID")?)
        } else {
            json!(value)
        };
        if params.insert(key.into(), value).is_some() {
            return Err("Unknown or duplicated option; see --help".into());
        }
    }
    let operation = match (words.as_slice(), command) {
        (["lease", "run"], Some(command)) if !command.is_empty() => {
            Operation::Run(command.to_vec())
        }
        (["lease", "acquire"], None) => Operation::Acquire,
        (["lease", "release", id], None) => {
            params.insert("id".into(), json!(id));
            Operation::Release
        }
        (["lease", "release"], None) => Operation::Release,
        (["lease", "status"], None) => Operation::Status,
        _ => return Err("Invalid lease operation; see --help".into()),
    };
    let allowed: &[&str] = match operation {
        Operation::Run(_) => &["pool", "holder"],
        Operation::Acquire => &["pool", "holder", "pid"],
        Operation::Release if params.contains_key("id") => &["id"],
        Operation::Release => &["pool", "holder"],
        Operation::Status => &[],
    };
    if params.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err("Invalid options for this lease operation; see --help".into());
    }
    if allowed.contains(&"pool") && !params.contains_key("pool") {
        return Err("Provide --pool".into());
    }
    if allowed.contains(&"holder") && !params.contains_key("holder") {
        params.insert("holder".into(), json!(default_holder()));
    }
    Ok(Invocation {
        local,
        operation,
        params,
    })
}

fn default_holder() -> String {
    std::env::current_dir()
        .map(|dir| dir.to_string_lossy().into_owned())
        .unwrap_or_else(|_| format!("pid {}", std::process::id()))
}

fn invoke(
    mut invocation: Invocation,
    connect: impl FnOnce(bool) -> io::Result<UnixStream>,
) -> Result<(i32, Option<Value>), String> {
    let method = match invocation.operation {
        Operation::Run(_) => {
            invocation
                .params
                .insert("pid".into(), json!(std::process::id()));
            "lease.run"
        }
        Operation::Acquire => "lease.acquire",
        Operation::Release => "lease.release",
        Operation::Status => "lease.status",
    };
    let stream = connect(invocation.local)
        .map_err(|_| "WorktreeManager is not running for this profile. Open the app first.")?;
    let (response, connection) = request(stream, method, invocation.params)?;
    let Operation::Run(command) = invocation.operation else {
        return Ok((if response["ok"] == true { 0 } else { 1 }, Some(response)));
    };
    if response["ok"] != true {
        return Ok((1, Some(response)));
    }
    let code = run_holding(&command, &response["data"]["env"]);
    drop(connection);
    Ok(code)
}

fn request(
    mut stream: UnixStream,
    method: &str,
    params: Map<String, Value>,
) -> Result<(Value, BufReader<UnixStream>), String> {
    let configure = |stream: &UnixStream| {
        stream.set_read_timeout(Some(Duration::from_secs(30)))?;
        stream.set_write_timeout(Some(Duration::from_secs(10)))
    };
    configure(&stream).map_err(|_| "Cannot configure connection")?;
    let id = format!("lease-{}", std::process::id());
    let request = Request {
        id: id.clone(),
        version: VERSION,
        method: method.into(),
        params: Value::Object(params),
    };
    let mut bytes = serde_json::to_vec(&request).map_err(|_| "Cannot encode request")?;
    bytes.push(b'\n');
    stream
        .write_all(&bytes)
        .map_err(|_| "Cannot send request")?;
    let mut reader = BufReader::new(stream);
    loop {
        let response = read_message(&mut reader)?;
        if response["id"] != id || response["version"] != VERSION {
            return Err("Unexpected response ID or protocol version".into());
        }
        match response["progress"].as_str() {
            Some(message) => eprintln!("{message}"),
            None => return Ok((response, reader)),
        }
    }
}

/// Runs the command with the lease environment while the connection keeps the lease held.
fn run_holding(command: &[String], env: &Value) -> (i32, Option<Value>) {
    let env: BTreeMap<String, String> = serde_json::from_value(env.clone()).unwrap_or_default();
    match Command::new(&command[0])
        .args(&command[1..])
        .envs(env)
        .status()
    {
        Ok(status) => (
            status
                .code()
                .unwrap_or_else(|| 128 + status.signal().unwrap_or(0)),
            None,
        ),
        Err(error) => (
            127,
            Some(failure(
                "spawn_failed",
                &format!("Cannot run {}: {error}", command[0]),
            )),
        ),
    }
}
