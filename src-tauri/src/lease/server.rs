use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;

use serde_json::{json, Value};

use super::{HeldUntil, LeaseRequest, Leases};
use crate::cli_bridge::{failure, VERSION};

const PROGRESS_EVERY_TICKS: u32 = 25;

/// The CLI side of a bridge connection: `lease.run` holds its lease until the client hangs up.
pub trait Client {
    fn send(&mut self, message: Value) -> bool;
    fn closed(&self) -> bool;
    fn wait_closed(&mut self);
}

pub struct StreamClient {
    stream: UnixStream,
    request_id: String,
}

impl StreamClient {
    pub fn new(stream: UnixStream, request_id: String) -> Self {
        Self { stream, request_id }
    }
}

impl Client for StreamClient {
    fn send(&mut self, mut message: Value) -> bool {
        message["id"] = json!(self.request_id);
        message["version"] = json!(VERSION);
        serde_json::to_vec(&message)
            .map(|mut bytes| {
                bytes.push(b'\n');
                std::io::Write::write_all(&mut self.stream, &bytes).is_ok()
            })
            .unwrap_or(false)
    }

    fn closed(&self) -> bool {
        let mut byte = 0u8;
        let read = unsafe {
            libc::recv(
                self.stream.as_raw_fd(),
                (&raw mut byte).cast(),
                1,
                libc::MSG_PEEK | libc::MSG_DONTWAIT,
            )
        };
        read == 0
            || (read < 0
                && std::io::Error::last_os_error().kind() != std::io::ErrorKind::WouldBlock)
    }

    fn wait_closed(&mut self) {
        let _ = self.stream.set_read_timeout(None);
        let mut buffer = [0u8; 64];
        while matches!(self.stream.read(&mut buffer), Ok(read) if read > 0) {}
    }
}

pub fn is_lease_method(method: &str) -> bool {
    method.starts_with("lease.")
}

pub fn serve(leases: &Leases, method: &str, params: &Value, client: &mut impl Client) {
    let response = match method {
        "lease.run" => match lease_request(params) {
            Ok(request) => match leases.acquire(request, waiting(client)) {
                Ok(guard) => {
                    if client.send(success(guard.lease())) {
                        client.wait_closed();
                    }
                    return;
                }
                Err(error) => error.response(),
            },
            Err(response) => response,
        },
        "lease.acquire" => match lease_request(params) {
            Ok(request) => leases
                .acquire_until_release(request, waiting(client))
                .map_or_else(|error| error.response(), |lease| success(&lease)),
            Err(response) => response,
        },
        "lease.release" => release(leases, params),
        "lease.status" => leases
            .status()
            .map_or_else(|error| error.response(), |status| success(&status)),
        _ => failure("method_not_found", "Unknown operation. See wtm --help."),
    };
    client.send(response);
}

fn success(data: &impl serde::Serialize) -> Value {
    json!({ "ok": true, "data": data })
}

fn waiting(client: &mut impl Client) -> impl FnMut(usize) -> bool + '_ {
    let mut ticks = 0;
    move |ahead| {
        ticks += 1;
        if ticks % PROGRESS_EVERY_TICKS == 1 {
            client.send(json!({ "progress": format!("Waiting for a lease: {ahead} ahead") }))
        } else {
            !client.closed()
        }
    }
}

fn text(params: &Value, key: &str) -> Option<String> {
    params[key]
        .as_str()
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn lease_request(params: &Value) -> Result<LeaseRequest, Value> {
    let pid = match &params["pid"] {
        Value::Null => None,
        value => Some(
            value
                .as_i64()
                .and_then(|pid| i32::try_from(pid).ok())
                .filter(|pid| *pid > 0)
                .ok_or_else(|| failure("invalid_args", "pid must be a positive process ID."))?,
        ),
    };
    match (text(params, "pool"), text(params, "holder")) {
        (Some(pool), Some(holder)) => Ok(LeaseRequest { pool, holder, pid }),
        _ => Err(failure("invalid_args", "A pool and a holder are required.")),
    }
}

fn release(leases: &Leases, params: &Value) -> Value {
    let released = match (
        text(params, "id"),
        text(params, "pool"),
        text(params, "holder"),
    ) {
        (Some(id), None, None) => leases.release(|lease| lease.id == id),
        (None, Some(pool), Some(holder)) => leases.release(|lease| {
            lease.pool == pool && lease.holder == holder && lease.held_until == HeldUntil::Release
        }),
        _ => {
            return failure(
                "invalid_args",
                "Release by lease ID, or by pool and holder.",
            )
        }
    };
    if released.is_empty() {
        failure("not_found", "No matching lease is held.")
    } else {
        success(&released)
    }
}
