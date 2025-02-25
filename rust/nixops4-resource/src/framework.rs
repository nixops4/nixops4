use std::{
    io::{BufRead, BufReader},
    os::fd::{AsRawFd, FromRawFd},
};

use anyhow::{Context, Result};
use nix::unistd::{dup, dup2};
use serde_json::json;

use crate::schema::v0::{self, CreateResourceRequest, CreateResourceResponse};

pub trait ResourceProvider {
    fn create(&self, request: CreateResourceRequest) -> Result<CreateResourceResponse>;
    // TODO:
    // fn check(&self) -> Result<()>;
    // fn destroy(&self) -> Result<()>;
    // fn update(&self) -> Result<()>;
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
        v0::Request::CreateResourceRequest(r) => {
            let resp = provider
                .create(r)
                .with_context(|| "Could not create resource")
                .unwrap_or_exit();
            serde_json::to_writer(pipe.out, &v0::Response::CreateResourceResponse(resp)).unwrap();
        }
        v0::Request::ReadResourceRequest(r) => todo!(),
        v0::Request::UpdateResourceRequest(r) => todo!(),
        v0::Request::DestroyResourceRequest(r) => todo!(),
        v0::Request::StateResourceEvent(r) => todo!(),
        v0::Request::StateResourceReadRequest(r) => todo!(),
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
