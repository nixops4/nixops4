use std::sync::Arc;

use anyhow::{bail, Context, Result};
use nixops4_core::eval_api::{self, EvalRequest, Id, Ids, MessageType, QueryRequest};
use tokio::sync::mpsc::{channel, Sender};
use tokio::sync::Mutex;
use tokio::{
    io::{AsyncBufReadExt as _, AsyncWriteExt as _},
    process::ChildStdout,
};
use tracing::debug;

#[derive(Clone)]
pub(crate) struct Options {
    pub(crate) verbose: bool,
    pub(crate) show_trace: bool,
    pub(crate) flake_input_overrides: Vec<(String, String)>,
}

#[derive(Clone)]
pub struct EvalSender {
    sender: Arc<Mutex<Option<Sender<EvalRequest>>>>,
    ids: Ids,
}

type EvalReceiver = tokio::sync::mpsc::Receiver<eval_api::EvalResponse>;

impl EvalSender {
    pub async fn with<T, F, Fut>(options: &Options, f: F) -> Result<T>
    where
        F: FnOnce(EvalSender, EvalReceiver) -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let exe = std::env::var("_NIXOPS4_EVAL").unwrap_or("nixops4-eval".to_string());
        let mut nix_config = std::env::var("NIX_CONFIG").unwrap_or("".to_string());
        if options.show_trace {
            nix_config.push_str("\nshow-trace = true\n");
        }
        let mut process = tokio::process::Command::new(exe)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .arg("<subprocess>")
            .env("NIX_CONFIG", nix_config)
            .spawn()
            .context("while starting the nixops4 evaluator process")?;

        let r;
        {
            let response_bufreader = tokio::io::BufReader::new(process.stdout.take().unwrap());
            let command_handle = process.stdin.take().unwrap();
            let tracing_event_receiver = tracing_tunnel::TracingEventReceiver::default();
            let ids = Ids::new();
            let verbose = options.verbose;

            let (command_sender, command_receiver) = channel::<EvalRequest>(128);

            let writer = tokio::spawn(forward_eval_commands(
                command_handle,
                verbose,
                command_receiver,
            ));

            let (response_sender, response_receiver) = channel::<eval_api::EvalResponse>(128);

            let reader = tokio::spawn(forward_eval_responses(
                verbose,
                response_sender,
                response_bufreader,
                tracing_event_receiver,
            ));

            let eval_sender = EvalSender {
                sender: Arc::new(Mutex::new(Some(command_sender))),
                ids: ids.clone(),
            };

            r = f(eval_sender, response_receiver).await;

            writer.await??;
            reader.await??;
        }
        // Wait for the process to exit, giving it a chance to flush its output
        // TODO (tokio): add timeout
        process.wait().await?;

        r
    }

    pub fn next_id<T>(&self) -> eval_api::Id<T> {
        self.ids.next()
    }

    pub async fn send(&self, request: &EvalRequest) -> Result<()> {
        let mut sender = self.sender.lock().await;
        match sender.as_mut() {
            Some(sender) => {
                if let Err(e) = sender.send(request.clone()).await {
                    bail!("error sending eval request: {}", e);
                }
                Ok(())
            }
            None => {
                bail!("refusing to send eval request as eval process is exiting");
            }
        }
    }

    pub async fn close(&self) {
        let mut sender = self.sender.lock().await;
        *sender = None;
    }

    pub async fn query<P, R>(
        &self,
        id: Id<MessageType>,
        f: impl FnOnce(QueryRequest<P, R>) -> EvalRequest,
        payload: P,
    ) -> Result<()> {
        self.send(&f(QueryRequest::new(id, payload))).await
    }
}

async fn forward_eval_responses(
    verbose: bool,
    response_sender: Sender<eval_api::EvalResponse>,
    response_bufreader: tokio::io::BufReader<ChildStdout>,
    mut tracing_event_receiver: tracing_tunnel::TracingEventReceiver,
) -> Result<()> {
    let mut lines = response_bufreader.lines();
    loop {
        let line_result = lines.next_line().await;
        match line_result {
            Ok(Some(line)) => {
                if let Ok(response) = eval_api::eval_response_from_json(line.as_str()) {
                    if verbose {
                        eprintln!("\x1b[32mreceived: {}\x1b[0m", line.trim_end());
                    }

                    if let eval_api::EvalResponse::TracingEvent(v) = response {
                        let event =
                            serde_json::from_value(v).context("while parsing tracing event")?;
                        if let Err(e) = tracing_event_receiver.try_receive(event) {
                            eprintln!("error handling tracing event: {}", e);
                        }
                    } else if let Err(e) = response_sender.send(response).await {
                        // Presumably the main program has produced an error, or has terminated correctly, and we can just ignore any extra messages?
                        debug!("error sending response to channel: {}", e);
                        // continue processing tracing events
                    }
                } else {
                    bail!("error parsing response: {}", line);
                }
            }
            Ok(None) => break,
            Err(e) => {
                bail!("error reading from nixops4-eval process stdout: {}", e);
            }
        }
    }
    Ok(())
}

async fn forward_eval_commands(
    mut command_handle: tokio::process::ChildStdin,
    verbose: bool,
    mut command_receiver: tokio::sync::mpsc::Receiver<EvalRequest>,
) -> Result<()> {
    loop {
        let request = command_receiver.recv().await;
        match request {
            Some(request) => {
                let r = write_eval_request(verbose, &mut command_handle, &request).await;
                match r {
                    Ok(()) => {}
                    Err(e) => {
                        eprintln!("error writing to nixops4-eval process stdin: {}", e);
                        break;
                    }
                }
            }
            None => break,
        }
    }
    Ok(())
}

async fn write_eval_request(
    verbose: bool,
    command_handle: &mut tokio::process::ChildStdin,
    request: &EvalRequest,
) -> Result<()> {
    let json = eval_api::eval_request_to_json(&request)?;
    if verbose {
        eprintln!("\x1b[35msending: {}\x1b[0m", json);
    }
    command_handle.write_all(json.as_bytes()).await?;
    command_handle.write_all(b"\n").await?;
    command_handle.flush().await?;
    Ok(())
}
