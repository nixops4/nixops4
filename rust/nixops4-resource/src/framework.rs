use std::{
    io::{BufRead, BufReader},
    os::fd::{AsRawFd, FromRawFd},
};

use anyhow::{bail, Context, Result};
use nix::unistd::{dup, dup2};
use serde_json::json;
use tracing;

use crate::schema::v0::{self};

pub trait ResourceProvider {
    fn create(&self, request: v0::CreateResourceRequest) -> Result<v0::CreateResourceResponse>;
    fn read(&self, request: v0::ReadResourceRequest) -> Result<v0::ReadResourceResponse>;
    fn destroy(&self, request: v0::DestroyResourceRequest) -> Result<v0::DestroyResourceResponse>;
    fn update(&self, request: v0::UpdateResourceRequest) -> Result<v0::UpdateResourceResponse>;
    fn state_read(
        &self,
        request: v0::StateResourceReadRequest,
    ) -> Result<v0::StateResourceReadResponse> {
        let _ = request;
        bail!("State read operation not implemented by resource provider")
    }
    fn state_event(
        &self,
        request: v0::StateResourceEvent,
    ) -> Result<v0::StateResourceEventResponse> {
        let _ = request;
        bail!("State event not implemented by resource provider")
    }
}

fn write_response<W: std::io::Write>(out: W, resp: &v0::Response) -> Result<()> {
    serde_json::to_writer(out, &resp).context("writing response")
}

pub fn run_main(provider: impl ResourceProvider) {
    let pipe = {
        let pipe = init_stdio();
        pipe_fds_to_files(pipe)
    };

    // Read the request from the input

    let mut in_ = BufReader::new(pipe.in_);

    let request: v0::Request = {
        let mut line = String::new();
        in_.read_line(&mut line)
            .with_context(|| "Could not read line for request message")
            .unwrap_or_exit();
        serde_json::from_str(&line)
            .with_context(|| "Could not parse request message")
            .unwrap_or_exit()
    };

    match request {
        v0::RequestOutputProperties::CreateResourceRequestEnvelope(r) => {
            let span =
                tracing::info_span!("create", type = r.create_resource_request.type_.as_str());
            let resp = provider
                .create(r.create_resource_request)
                .with_context(|| "Could not create resource")
                .unwrap_or_exit();
            write_response(
                pipe.out,
                &v0::Response::CreateResourceResponseEnvelope(v0::CreateResourceResponseEnvelope {
                    create_resource_response: resp,
                }),
            )
            .context("writing response")
            .unwrap_or_exit();
            drop(span);
        }
        v0::RequestOutputProperties::ReadResourceRequestEnvelope(r) => {
            let span =
                tracing::info_span!("read", type = r.read_resource_request.resource.type_.as_str());
            let resp = provider
                .read(r.read_resource_request)
                .with_context(|| "Could not read resource")
                .unwrap_or_exit();
            serde_json::to_writer(
                pipe.out,
                &v0::Response::ReadResourceResponseEnvelope(v0::ReadResourceResponseEnvelope {
                    read_resource_response: resp,
                }),
            )
            .context("writing response")
            .unwrap_or_exit();
            drop(span);
        }
        v0::RequestOutputProperties::UpdateResourceRequestEnvelope(r) => {
            let span = tracing::info_span!("update", type = r.update_resource_request.resource.type_.as_str());
            let resp = provider
                .update(r.update_resource_request)
                .with_context(|| "Could not update resource")
                .unwrap_or_exit();
            serde_json::to_writer(
                pipe.out,
                &v0::Response::UpdateResourceResponseEnvelope(v0::UpdateResourceResponseEnvelope {
                    update_resource_response: resp,
                }),
            )
            .context("writing response")
            .unwrap_or_exit();
            drop(span);
        }
        v0::RequestOutputProperties::DestroyResourceRequestEnvelope(r) => {
            let span = tracing::info_span!("destroy", type = r.destroy_resource_request.resource.type_.as_str());
            let resp = provider
                .destroy(r.destroy_resource_request)
                .with_context(|| "Could not destroy resource (or not completely, correctly)")
                .unwrap_or_exit();
            serde_json::to_writer(
                pipe.out,
                &v0::DestroyResourceResponseEnvelope {
                    destroy_resource_response: resp,
                },
            )
            .context("writing response")
            .unwrap_or_exit();
            drop(span);
        }
        v0::RequestOutputProperties::StateResourceEventEnvelope(r) => {
            let span = tracing::info_span!("state_event", type = r.state_resource_event.resource.type_.as_str());
            let resp = provider
                .state_event(r.state_resource_event)
                .with_context(|| "Could not handle state event")
                .unwrap_or_exit();
            serde_json::to_writer(
                pipe.out,
                &v0::Response::StateResourceEventResponseEnvelope(
                    v0::StateResourceEventResponseEnvelope {
                        state_resource_event_response: resp,
                    },
                ),
            )
            .context("writing response")
            .unwrap_or_exit();
            drop(span);
        }
        v0::RequestOutputProperties::StateResourceReadRequestEnvelope(r) => {
            let span = tracing::info_span!("state_read", type = r.state_resource_read_request.resource.type_.as_str());
            let resp = provider
                .state_read(r.state_resource_read_request)
                .with_context(|| "Could not read state")
                .unwrap_or_exit();
            serde_json::to_writer(
                pipe.out,
                &v0::Response::StateResourceReadResponseEnvelope(
                    v0::StateResourceReadResponseEnvelope {
                        state_resource_read_response: resp,
                    },
                ),
            )
            .context("writing response")
            .unwrap_or_exit();
            drop(span);
        }
    }

    // Write the response to the output
}

/// A pair of `T` values: one for input and one for output.
struct InOut<T> {
    in_: T,
    out: T,
}

/// A file descriptor
type Fd = i32;

/// Configure the standard input/output streams for the process.
/// This returns the communication channels with nixops4, and reconfigures the
/// stdio file descriptor as follows:
///
/// ```text
/// 0: /dev/null
/// 1: stderr
/// 2: stderr
/// ```
fn init_stdio() -> InOut<Fd> {
    let r = InOut {
        in_: dup(0).with_context(|| "dup(0)").unwrap(),
        out: dup(1).with_context(|| "dup(1)").unwrap(),
    };

    // 0: dev/null
    {
        // Open an empty stream for stdin
        let dev_null = std::fs::File::open("/dev/null")
            .with_context(|| "Could not open /dev/null")
            .unwrap();
        // dup2 the file descriptor of dev_null to 0
        dup2(dev_null.as_raw_fd(), 0)
            .with_context(|| "Could not dup2(/dev/null, 0)")
            .unwrap();
    }

    // 1: stderr
    dup2(2, 1).with_context(|| "Could not dup2(2, 1)").unwrap();

    // 2: stderr is left as is

    // All good, return the communication channels
    r
}

fn pipe_fds_to_files(pipe: InOut<i32>) -> InOut<std::fs::File> {
    InOut {
        in_: unsafe { std::fs::File::from_raw_fd(pipe.in_) },
        out: unsafe { std::fs::File::from_raw_fd(pipe.out) },
    }
}

trait NixOps4MainError<T> {
    type V;
    fn unwrap_or_exit(self) -> Self::V;
}
impl<T> NixOps4MainError<Result<T>> for Result<T> {
    type V = T;
    fn unwrap_or_exit(self) -> T {
        match self {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Error: {:?}", e);
                std::process::exit(1);
            }
        }
    }
}
