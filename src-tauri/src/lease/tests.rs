use std::collections::HashSet;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::server::{serve, StreamClient};
use super::*;
use crate::cli_bridge::{read_message, VERSION};

const POOLS: &str = r#"
[heavy_checks]
size = 1
env = { PARALLEL_PROCESSOR_COUNT = "1" }

[test_slot]
values = [13, 14, 15, 16, 17]
env = { TEST_ENV_NUMBER = "{value}", SLOT_LABEL = "slot-{value}" }

[named]
values = ["a", "b"]
"#;

struct Scratch(PathBuf);

impl Scratch {
    fn new(pools: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("wtm-lease-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("pools.toml"), pools).unwrap();
        Self(dir)
    }

    fn leases(&self) -> Leases {
        Leases::new(Some(self.0.join("pools.toml")))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn request(pool: &str, holder: &str) -> LeaseRequest {
    LeaseRequest {
        pool: pool.into(),
        holder: holder.into(),
        pid: None,
    }
}

fn waiters(leases: &Leases, pool: &str) -> usize {
    leases
        .lock()
        .queue
        .iter()
        .filter(|ticket| ticket.request.pool == pool)
        .count()
}

fn eventually(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "condition not met in time");
        thread::sleep(Duration::from_millis(10));
    }
}

fn exited_child() -> i32 {
    let mut child = Command::new("true").spawn().unwrap();
    let pid = child.id() as i32;
    child.wait().unwrap();
    pid
}

#[test]
fn config_defaults_to_one_heavy_check_and_expands_sizes_values_and_env() {
    let defaults = config::load(None).unwrap();
    assert_eq!(defaults["heavy_checks"].values, ["1"]);
    assert!(defaults["heavy_checks"].env_for("1").is_empty());
    let missing = PathBuf::from("/nonexistent/wtm/pools.toml");
    assert_eq!(config::load(Some(&missing)).unwrap(), defaults);
    assert_eq!(
        config::default_path(Some("/home/me".into())),
        Some(PathBuf::from("/home/me/.worktreemanager/pools.toml"))
    );
    assert_eq!(config::default_path(None), None);

    let scratch = Scratch::new(POOLS);
    let pools = config::load(Some(&scratch.0.join("pools.toml"))).unwrap();
    assert_eq!(pools["test_slot"].values, ["13", "14", "15", "16", "17"]);
    assert_eq!(pools["named"].values, ["a", "b"]);
    assert_eq!(
        pools["test_slot"].env_for("14"),
        BTreeMap::from([
            ("SLOT_LABEL".to_string(), "slot-14".to_string()),
            ("TEST_ENV_NUMBER".to_string(), "14".to_string()),
        ])
    );
}

#[test]
fn invalid_config_is_reported_instead_of_falling_back() {
    for (pools, message) in [
        ("[a]\nsize = 0", "needs either"),
        ("[a]\nvalues = []", "needs either"),
        ("[a]\nvalues = [1, \"1\"]", "needs either"),
        ("[a]\nsize = 2\nvalues = [1]", "needs either"),
        ("[\"a b\"]\nsize = 1", "may only use"),
        (
            "[a]\nsize = 1\nenv = { \"BAD-NAME\" = \"x\" }",
            "invalid variable",
        ),
        ("[a]\nsize = 1\nextra = true", "Invalid"),
    ] {
        let scratch = Scratch::new(pools);
        let error = config::load(Some(&scratch.0.join("pools.toml"))).unwrap_err();
        assert!(error.contains(message), "{pools}: {error}");
        assert!(matches!(
            scratch.leases().status(),
            Err(LeaseError::Config(_))
        ));
    }
    let scratch = Scratch::new("");
    assert!(config::load(Some(&scratch.0))
        .unwrap_err()
        .starts_with("Cannot read"));
    assert_eq!(
        scratch
            .leases()
            .acquire(request("missing", "me"), |_| true)
            .err(),
        Some(LeaseError::Config("Unknown pool `missing`".into()))
    );
    assert_eq!(
        LeaseError::Config("bad".into()).response()["error"]["code"],
        "lease_config"
    );
    assert_eq!(
        LeaseError::Cancelled.response()["error"]["code"],
        "cancelled"
    );
}

#[test]
fn waiters_are_granted_in_fifo_order() {
    let scratch = Scratch::new(POOLS);
    let leases = scratch.leases();
    let first = leases
        .acquire(request("heavy_checks", "first"), |_| true)
        .unwrap();
    assert_eq!(first.lease().env["PARALLEL_PROCESSOR_COUNT"], "1");
    let (granted, order) = mpsc::channel();
    let mut threads = Vec::new();
    for (index, holder) in ["second", "third", "fourth"].into_iter().enumerate() {
        let waiting = leases.clone();
        let granted = granted.clone();
        threads.push(thread::spawn(move || {
            let mut positions = Vec::new();
            let guard = waiting
                .acquire(request("heavy_checks", holder), |ahead| {
                    positions.push(ahead);
                    true
                })
                .unwrap();
            granted.send(guard.lease().holder.clone()).unwrap();
            thread::sleep(Duration::from_millis(20));
            positions
        }));
        eventually(|| waiters(&leases, "heavy_checks") == index + 1);
    }
    drop(first);
    let order: Vec<String> = order.iter().take(3).collect();
    assert_eq!(order, ["second", "third", "fourth"]);
    let positions: Vec<Vec<usize>> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert_eq!(positions[2][0], 2);
    assert!(leases.lock().held.is_empty());
}

#[test]
fn concurrent_clients_never_share_a_slot() {
    let scratch = Scratch::new(POOLS);
    let leases = scratch.leases();
    let guards: Vec<LeaseGuard> = thread::scope(|scope| {
        let threads: Vec<_> = (0..5)
            .map(|index| {
                let leases = &leases;
                scope.spawn(move || {
                    leases
                        .acquire(request("test_slot", &format!("w{index}")), |_| true)
                        .unwrap()
                })
            })
            .collect();
        threads.into_iter().map(|t| t.join().unwrap()).collect()
    });
    let values: HashSet<&str> = guards
        .iter()
        .map(|guard| guard.lease().value.as_str())
        .collect();
    assert_eq!(values.len(), 5);
    assert_eq!(
        leases
            .acquire(request("test_slot", "sixth"), |_| false)
            .err(),
        Some(LeaseError::Cancelled)
    );
    assert_eq!(waiters(&leases, "test_slot"), 0);
    let other_pool = leases.acquire(request("named", "x"), |_| true).unwrap();
    assert_eq!(other_pool.lease().value, "a");
}

#[test]
fn leases_of_a_dead_process_are_released() {
    let scratch = Scratch::new(POOLS);
    let leases = scratch.leases();
    let mut owner = Command::new("sleep").arg("30").spawn().unwrap();
    let lease = leases
        .acquire_until_release(
            LeaseRequest {
                pid: Some(owner.id() as i32),
                ..request("heavy_checks", "dying")
            },
            |_| true,
        )
        .unwrap();
    assert_eq!(lease.held_until, HeldUntil::Release);
    assert_eq!(
        leases.status().unwrap()["pools"][0]["leases"][0]["holder"],
        "dying"
    );
    let waiter = {
        let leases = leases.clone();
        thread::spawn(move || {
            leases
                .acquire(request("heavy_checks", "next"), |_| true)
                .map(|guard| guard.lease().holder.clone())
        })
    };
    eventually(|| waiters(&leases, "heavy_checks") == 1);
    owner.kill().unwrap();
    owner.wait().unwrap();
    assert_eq!(waiter.join().unwrap().unwrap(), "next");

    leases
        .acquire_until_release(
            LeaseRequest {
                pid: Some(exited_child()),
                ..request("heavy_checks", "gone")
            },
            |_| true,
        )
        .unwrap();
    assert_eq!(leases.status().unwrap()["pools"][0]["leases"], json!([]));
    assert!(process_alive(1));
}

#[test]
fn long_lived_leases_are_retry_safe_and_released_by_id_or_holder() {
    let scratch = Scratch::new(POOLS);
    let leases = scratch.leases();
    let first = leases
        .acquire_until_release(request("test_slot", "/wt/a"), |_| true)
        .unwrap();
    let again = leases
        .acquire_until_release(request("test_slot", "/wt/a"), |_| true)
        .unwrap();
    assert_eq!(first, again);
    assert_eq!(first.env["TEST_ENV_NUMBER"], "13");
    let other = leases
        .acquire_until_release(request("test_slot", "/wt/b"), |_| true)
        .unwrap();
    assert_eq!(other.value, "14");
    assert_eq!(leases.release(|lease| lease.id == first.id), [first]);
    assert!(leases.release(|lease| lease.id == "unknown").is_empty());
    assert_eq!(leases.lock().held, [other]);
}

#[test]
fn status_lists_configured_pools_holders_and_waiters() {
    let scratch = Scratch::new(POOLS);
    let leases = scratch.leases();
    let held = leases.acquire(request("named", "me"), |_| true).unwrap();
    let _other = leases.acquire(request("named", "other"), |_| true).unwrap();
    let mut retired = held.lease().clone();
    retired.pool = "retired".into();
    leases.lock().held.push(retired);
    let waiter = {
        let leases = leases.clone();
        thread::spawn(move || leases.acquire(request("named", "you"), |_| true).is_ok())
    };
    eventually(|| waiters(&leases, "named") == 1);
    let status = leases.status().unwrap();
    let names: Vec<&str> = status["pools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pool| pool["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["heavy_checks", "named", "retired", "test_slot"]);
    assert_eq!(status["pools"][1]["leases"][0]["holder"], "me");
    assert_eq!(status["pools"][1]["waiting"][0]["holder"], "you");
    assert_eq!(status["pools"][2]["values"], Value::Null);
    drop(held);
    assert!(waiter.join().unwrap());
}

fn serve_pair(
    leases: &Leases,
    method: &str,
    params: Value,
) -> (UnixStream, thread::JoinHandle<()>) {
    let (cli, app) = UnixStream::pair().unwrap();
    let leases = leases.clone();
    let method = method.to_string();
    let handle = thread::spawn(move || {
        serve(
            &leases,
            &method,
            &params,
            &mut StreamClient::new(app, "r1".into()),
        );
    });
    (cli, handle)
}

fn next_message(reader: &mut impl BufRead) -> Value {
    let message = read_message(reader).unwrap();
    assert_eq!(message["id"], "r1");
    assert_eq!(message["version"], VERSION);
    message
}

#[test]
fn run_holds_the_lease_until_the_connection_closes() {
    let scratch = Scratch::new(POOLS);
    let leases = scratch.leases();
    let params = json!({ "pool": "test_slot", "holder": "/wt/a", "pid": std::process::id() });
    let (cli, server) = serve_pair(&leases, "lease.run", params);
    let mut reader = BufReader::new(cli.try_clone().unwrap());
    let granted = next_message(&mut reader);
    assert_eq!(granted["data"]["env"]["TEST_ENV_NUMBER"], "13");
    assert_eq!(granted["data"]["heldUntil"], "exit");
    (&cli).write_all(b"ignored\n").unwrap();
    thread::sleep(Duration::from_millis(50));
    assert_eq!(leases.lock().held.len(), 1);
    drop((cli, reader));
    server.join().unwrap();
    assert!(leases.lock().held.is_empty());
}

#[test]
fn a_waiting_client_that_disconnects_leaves_the_queue() {
    let scratch = Scratch::new(POOLS);
    let leases = scratch.leases();
    let holder = leases
        .acquire(request("heavy_checks", "busy"), |_| true)
        .unwrap();
    let (cli, server) = serve_pair(
        &leases,
        "lease.acquire",
        json!({ "pool": "heavy_checks", "holder": "/wt/b" }),
    );
    let mut reader = BufReader::new(cli.try_clone().unwrap());
    assert_eq!(
        next_message(&mut reader)["progress"],
        "Waiting for a lease: 0 ahead"
    );
    thread::sleep(Duration::from_millis(500));
    assert_eq!(waiters(&leases, "heavy_checks"), 1);
    drop((cli, reader));
    server.join().unwrap();
    assert_eq!(waiters(&leases, "heavy_checks"), 0);
    drop(holder);
}

#[test]
fn bridge_methods_validate_params_and_report_results() {
    let scratch = Scratch::new(POOLS);
    let leases = scratch.leases();
    let call = |method: &str, params: Value| {
        let (cli, server) = serve_pair(&leases, method, params);
        let message = next_message(&mut BufReader::new(cli));
        server.join().unwrap();
        message
    };
    let acquired = call(
        "lease.acquire",
        json!({ "pool": "named", "holder": "/wt/a", "pid": std::process::id() }),
    );
    assert_eq!(acquired["data"]["value"], "a");
    assert_eq!(acquired["data"]["heldUntil"], "release");
    assert_eq!(
        call("lease.status", json!({}))["data"]["pools"][1]["name"],
        "named"
    );
    for (method, params, code) in [
        ("lease.acquire", json!({ "pool": "named" }), "invalid_args"),
        ("lease.run", json!({ "holder": "x" }), "invalid_args"),
        (
            "lease.run",
            json!({ "pool": "x", "holder": "x" }),
            "lease_config",
        ),
        (
            "lease.acquire",
            json!({ "pool": "named", "holder": "x", "pid": -1 }),
            "invalid_args",
        ),
        (
            "lease.acquire",
            json!({ "pool": "nope", "holder": "x" }),
            "lease_config",
        ),
        ("lease.release", json!({}), "invalid_args"),
        (
            "lease.release",
            json!({ "id": "a", "pool": "named" }),
            "invalid_args",
        ),
        ("lease.release", json!({ "id": "unknown" }), "not_found"),
        ("lease.unknown", json!({}), "method_not_found"),
    ] {
        assert_eq!(
            call(method, params.clone())["error"]["code"],
            code,
            "{method} {params}"
        );
    }
    let released = call(
        "lease.release",
        json!({ "pool": "named", "holder": "/wt/a" }),
    );
    assert_eq!(released["data"][0]["id"], acquired["data"]["id"]);
    let again = call(
        "lease.acquire",
        json!({ "pool": "named", "holder": "/wt/a" }),
    );
    let released = call("lease.release", json!({ "id": again["data"]["id"] }));
    assert_eq!(released["data"][0]["value"], "a");

    let broken = Scratch::new("not toml [");
    let (cli, server) = serve_pair(&broken.leases(), "lease.status", json!({}));
    assert_eq!(
        next_message(&mut BufReader::new(cli))["error"]["code"],
        "lease_config"
    );
    server.join().unwrap();
    assert!(server::is_lease_method("lease.status"));
    assert!(!server::is_lease_method("task.list"));
}

mod cli {
    use super::*;
    use crate::lease::client::run_if_requested;

    fn args(input: &str) -> Vec<String> {
        input.split_whitespace().map(str::to_owned).collect()
    }

    fn connect_to(leases: &Leases) -> impl FnOnce(bool) -> std::io::Result<UnixStream> {
        let leases = leases.clone();
        move |_| {
            let (cli, app) = UnixStream::pair()?;
            thread::spawn(move || {
                let mut reader = BufReader::new(app.try_clone().unwrap());
                let request: crate::cli_bridge::Request =
                    serde_json::from_value(read_message(&mut reader).unwrap()).unwrap();
                serve(
                    &leases,
                    &request.method,
                    &request.params,
                    &mut StreamClient::new(app, request.id),
                );
            });
            Ok(cli)
        }
    }

    fn unreachable(_: bool) -> std::io::Result<UnixStream> {
        Err(std::io::ErrorKind::NotFound.into())
    }

    #[test]
    fn ignores_other_commands_and_help() {
        for input in [
            "task list",
            "--local",
            "lease --help",
            "lease run --pool heavy_checks -h",
        ] {
            assert_eq!(run_if_requested(&args(input), unreachable), None, "{input}");
        }
    }

    #[test]
    fn rejects_invalid_lease_arguments() {
        for input in [
            "lease",
            "lease run --pool heavy_checks",
            "lease run --pool heavy_checks --",
            "lease run -- true",
            "lease acquire",
            "lease acquire --pool a --pool b",
            "lease acquire --pool a --pid 0",
            "lease acquire --pool a --pid",
            "lease acquire --pool a --bogus",
            "--local --local lease status",
            "lease status --pool a",
            "lease release abc --pool a",
            "lease release abc def",
            "lease status -- true",
        ] {
            assert_eq!(
                run_if_requested(&args(input), unreachable),
                Some(2),
                "{input}"
            );
        }
        assert_eq!(
            run_if_requested(&args("lease status"), unreachable),
            Some(1)
        );
    }

    #[test]
    fn run_exports_the_lease_environment_and_returns_the_command_status() {
        let scratch = Scratch::new(POOLS);
        let leases = scratch.leases();
        let output = scratch.0.join("env.txt");
        let script = format!(
            "printf %s \"$TEST_ENV_NUMBER $SLOT_LABEL\" > '{}'",
            output.display()
        );
        let mut command = args("--local lease run --pool test_slot --holder /wt/a --");
        command.extend(["sh".into(), "-c".into(), script]);
        let mut local = None;
        let connect = connect_to(&leases);
        let code = run_if_requested(&command, |is_local| {
            local = Some(is_local);
            connect(is_local)
        });
        assert_eq!(code, Some(0));
        assert_eq!(local, Some(true));
        assert_eq!(fs::read_to_string(&output).unwrap(), "13 slot-13");
        eventually(|| leases.lock().held.is_empty());

        let mut failing = args("lease run --pool heavy_checks --");
        failing.extend(["sh".into(), "-c".into(), "exit 3".into()]);
        assert_eq!(run_if_requested(&failing, connect_to(&leases)), Some(3));
        let mut signalled = args("lease run --pool heavy_checks --");
        signalled.extend(["sh".into(), "-c".into(), "kill -TERM $$".into()]);
        assert_eq!(
            run_if_requested(&signalled, connect_to(&leases)),
            Some(128 + libc::SIGTERM)
        );
        assert_eq!(
            run_if_requested(
                &args("lease run --pool heavy_checks -- /nonexistent/wtm-command"),
                connect_to(&leases)
            ),
            Some(127)
        );
        assert_eq!(
            run_if_requested(
                &args("lease run --pool missing -- true"),
                connect_to(&leases)
            ),
            Some(1)
        );
        eventually(|| leases.lock().held.is_empty());
    }

    #[test]
    fn acquire_release_and_status_round_trip() {
        let scratch = Scratch::new(POOLS);
        let leases = scratch.leases();
        assert_eq!(
            run_if_requested(
                &args("lease acquire --pool named --pid 1"),
                connect_to(&leases)
            ),
            Some(0)
        );
        let holder = std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(leases.lock().held[0].holder, holder);
        assert_eq!(leases.lock().held[0].pid, Some(1));
        assert_eq!(
            run_if_requested(&args("lease status"), connect_to(&leases)),
            Some(0)
        );
        assert_eq!(
            run_if_requested(&args("lease release --pool named"), connect_to(&leases)),
            Some(0)
        );
        assert_eq!(
            run_if_requested(&args("lease release nope"), connect_to(&leases)),
            Some(1)
        );
        assert!(leases.lock().held.is_empty());
    }

    #[test]
    fn rejects_responses_for_another_request() {
        let connect = |_| {
            let (cli, app) = UnixStream::pair()?;
            thread::spawn(move || {
                let mut reader = BufReader::new(app.try_clone().unwrap());
                read_message(&mut reader).unwrap();
                writeln!(
                    &app,
                    "{}",
                    json!({ "id": "other", "version": VERSION, "ok": true })
                )
                .unwrap();
            });
            Ok(cli)
        };
        assert_eq!(run_if_requested(&args("lease status"), connect), Some(1));
    }
}
