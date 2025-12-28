use std::os::fd::{AsRawFd, FromRawFd};

use anyhow::{Context, Error, Result};
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use nix::unistd::{dup, dup2};
use tokio_util::{
    bytes::{BufMut, BytesMut},
    codec::{Decoder, Encoder, FramedRead, FramedWrite},
};

use crate::{rpc::ResourceProviderRpcServer, schema::v0};

#[async_trait]
pub trait ResourceProvider: Send + Sync + 'static {
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

#[derive(Default)]
pub struct ContentLengthCodec {
    content_length: Option<usize>,
}

impl ContentLengthCodec {
    fn decode_headers(&mut self, src: &mut BytesMut) -> Result<(), Error> {
        if let Some(pos) = src.windows(4).position(|w| w == b"\r\n\r\n") {
            // Extract headers
            let headers = src.split_to(pos + 4);
            let text = String::from_utf8(headers.to_vec()).context("Decoding UTF-8 payload")?;

            // Find Content-Length
            for line in text.lines() {
                if let Some(v) = line.strip_prefix("Content-Length:") {
                    let len = v
                        .trim()
                        .parse::<usize>()
                        .context("Parsing Content-Length value")?;
                    self.content_length = Some(len);
                    break;
                }
            }

            let Some(content_length) = self.content_length else {
                return Err(anyhow::anyhow!("Missing Content-Length header"));
            };

            // Allocate space for the payload
            src.reserve(content_length.saturating_sub(src.len()));

            Ok(())
        } else {
            Ok(()) // Need more data
        }
    }
}

impl Decoder for ContentLengthCodec {
    type Item = String;

    type Error = anyhow::Error;

    fn decode(
        &mut self,
        src: &mut BytesMut,
    ) -> std::result::Result<Option<Self::Item>, Self::Error> {
        // Step 1: parse the headers
        self.decode_headers(src)?;

        let Some(content_length) = self.content_length else {
            // Headers not complete yet
            return Ok(None);
        };

        // Step 2: check if we have enough data for the payload
        if src.len() < content_length {
            // Not enough data yet
            return Ok(None);
        }

        // Step 3: extract the payload
        let body = src.split_to(content_length);
        self.content_length = None;

        let text = String::from_utf8(body.to_vec()).context("Decoding UTF-8 payload")?;

        Ok(Some(text))
    }
}

impl Encoder<&str> for ContentLengthCodec {
    type Error = anyhow::Error;

    fn encode(&mut self, item: &str, dst: &mut BytesMut) -> std::result::Result<(), Self::Error> {
        let header = format!("Content-Length: {}\r\n\r\n", item.len());
        // Pre-allocate so we ensure only one allocation
        dst.reserve(header.len() + item.len() + 1);
        dst.put(header.as_bytes());
        dst.put(item.as_bytes());
        dst.put("\n".as_bytes());

        Ok(())
    }
}

pub async fn run_main(provider: impl ResourceProvider) {
    let pipe = {
        let pipe = init_stdio();
        pipe_fds_to_files(pipe)
    };

    let in_ = tokio::fs::File::from_std(pipe.in_);

    let mut out = FramedWrite::new(
        tokio::fs::File::from_std(pipe.out),
        ContentLengthCodec::default(),
    );

    let rpc_module = provider.into_rpc();

    // Loop to handle multiple requests
    let mut framed = FramedRead::new(in_, ContentLengthCodec::default());
    while let Some(Ok(request)) = framed.next().await {
        let (resp, _) = rpc_module
            .raw_json_request(&request, 1)
            .await
            .with_context(|| "Could not parse request message")
            .unwrap_or_exit();

        out.send(resp.get())
            .await
            .context("writing response")
            .unwrap_or_exit();
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
