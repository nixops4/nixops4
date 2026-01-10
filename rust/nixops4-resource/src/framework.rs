use std::{
    io::{BufRead, BufReader},
    os::fd::{AsRawFd, FromRawFd},
};

use anyhow::{Context, Result};
use nix::unistd::{dup, dup2};

use crate::schema::v0;

// TODO: we'll probably want Send bounds on these futures later.
#[allow(async_fn_in_trait)]
pub trait ResourceProvider {
    async fn create(
        &self,
        request: v0::CreateResourceRequest,
    ) -> Result<v0::CreateResourceResponse>;
    async fn update(
        &self,
        request: v0::UpdateResourceRequest,
    ) -> Result<v0::UpdateResourceResponse>;
    async fn state_read(
        &self,
        request: v0::StateResourceReadRequest,
    ) -> Result<v0::StateResourceReadResponse> {
        let _ = request;
        anyhow::bail!("State read operation not implemented by resource provider")
    }
    async fn state_event(
        &self,
        request: v0::StateResourceEvent,
    ) -> Result<v0::StateResourceEventResponse> {
        let _ = request;
        anyhow::bail!("State event not implemented by resource provider")
    }
}

fn write_response<W: std::io::Write>(mut out: W, resp: &v0::Response) -> Result<()> {
    out.write_all(
        serde_json::to_string(resp)
            .with_context(|| "Could not serialize response")
            .unwrap()
            .as_bytes(),
    )?;
    out.write_all(b"\n").context("writing newline")?;
    out.flush().context("flushing response")
}

pub async fn run_main(provider: impl ResourceProvider) {
    let pipe = {
        let pipe = init_stdio();
        pipe_fds_to_files(pipe)
    };

    // Read the request from the input

    let mut in_ = BufReader::new(pipe.in_);

    let mut out = pipe.out;

    // Loop to handle multiple requests
    loop {
        let request: v0::Request = {
            let mut line = String::new();
            match in_.read_line(&mut line) {
                Ok(0) => {
                    // EOF - client closed stdin, exit gracefully
                    break;
                }
                Ok(_) => serde_json::from_str(&line)
                    .with_context(|| "Could not parse request message")
                    .unwrap_or_exit(),
                Err(e) => {
                    eprintln!("Error reading request: {}", e);
                    break;
                }
            }
        };

        match request {
            v0::Request::CreateResourceRequest(r) => {
                let resp = provider
                    .create(r)
                    .await
                    .with_context(|| "Could not create resource")
                    .unwrap_or_exit();
                write_response(&mut out, &v0::Response::CreateResourceResponse(resp))
                    .context("writing response")
                    .unwrap_or_exit();
            }
            v0::Request::UpdateResourceRequest(r) => {
                let resp = provider
                    .update(r)
                    .await
                    .with_context(|| "Could not update resource")
                    .unwrap_or_exit();
                write_response(&mut out, &v0::Response::UpdateResourceResponse(resp))
                    .context("writing response")
                    .unwrap_or_exit();
            }
            v0::Request::StateResourceEvent(r) => {
                let resp = provider
                    .state_event(r)
                    .await
                    .with_context(|| "Could not handle state event")
                    .unwrap_or_exit();
                write_response(&mut out, &v0::Response::StateResourceEventResponse(resp))
                    .context("writing response")
                    .unwrap_or_exit();
            }
            v0::Request::StateResourceReadRequest(r) => {
                let resp = provider
                    .state_read(r)
                    .await
                    .with_context(|| "Could not read state")
                    .unwrap_or_exit();
                write_response(&mut out, &v0::Response::StateResourceReadResponse(resp))
                    .context("writing response")
                    .unwrap_or_exit();
            }
        }
    }
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
