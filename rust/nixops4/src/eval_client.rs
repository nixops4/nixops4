use std::{
    collections::HashMap,
    io::{BufRead, Write},
    process::ChildStdout,
};

use anyhow::{Context, Result};
use nixops4_core::eval_api::{
    self, DeploymentType, EvalRequest, EvalResponse, FlakeType, Id, IdNum, Ids, MessageType,
    QueryRequest,
};

#[derive(Clone)]
pub(crate) struct Options {
    pub(crate) verbose: bool,
    pub(crate) show_trace: bool,
    pub(crate) flake_input_overrides: Vec<(String, String)>,
}

pub struct EvalClient {
    options: Options,

    response_bufreader: std::io::BufReader<ChildStdout>,
    command_handle: std::process::ChildStdin,
    tracing_event_receiver: tracing_tunnel::TracingEventReceiver,

    ids: Ids,
    deployments: HashMap<Id<FlakeType>, Vec<String>>,
    resources: HashMap<Id<DeploymentType>, Vec<String>>,
    errors: HashMap<IdNum, String>,
}
impl EvalClient {
    pub fn with<T>(options: &Options, f: impl FnOnce(EvalClient) -> Result<T>) -> Result<T> {
        let exe = std::env::var("_NIXOPS4_EVAL").unwrap_or("nixops4-eval".to_string());
        let mut nix_config = std::env::var("NIX_CONFIG").unwrap_or("".to_string());
        if options.show_trace {
            nix_config.push_str("\nshow-trace = true\n");
        }
        let mut process = std::process::Command::new(exe)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .arg("<subprocess>")
            .env("NIX_CONFIG", nix_config)
            .spawn()
            .context("while starting the nixops4 evaluator process")?;

        if options.verbose {
            eprintln!("started nixops4-eval process: {}", process.id());
        }

        let r;
        {
            let c: EvalClient = EvalClient {
                options: options.clone(),
                response_bufreader: std::io::BufReader::new(process.stdout.take().unwrap()),
                command_handle: process.stdin.take().unwrap(),
                tracing_event_receiver: tracing_tunnel::TracingEventReceiver::default(),
                ids: Ids::new(),
                deployments: HashMap::new(),
                resources: HashMap::new(),
                errors: HashMap::new(),
            };

            r = f(c)
        }
        // Wait for the process to exit, giving it a chance to flush its output
        // TODO (tokio): add timeout
        process.wait()?;

        r
    }
    pub fn send(&mut self, request: &EvalRequest) -> Result<()> {
        let json = eval_api::eval_request_to_json(request)?;
        if self.options.verbose {
            eprintln!("\x1b[35msending: {}\x1b[0m", json);
        }
        self.command_handle.write_all(json.as_bytes())?;
        self.command_handle.write_all(b"\n")?;
        self.command_handle.flush()?;
        Ok(())
    }
    pub fn query<P, R>(
        &mut self,
        f: impl FnOnce(QueryRequest<P, R>) -> EvalRequest,
        payload: P,
    ) -> Result<Id<MessageType>> {
        let msg_id = self.next_id();
        self.send(&f(QueryRequest::new(msg_id, payload)))?;
        Ok(msg_id)
    }
    fn receive(&mut self) -> Result<eval_api::EvalResponse> {
        let mut line = String::new();
        let n = self.response_bufreader.read_line(&mut line);
        match n {
            Err(e) => {
                Err(e).context("error reading from nixops4-eval process stdout")?;
            }
            Ok(0) => {
                Err(anyhow::anyhow!("nixops4-eval process closed its stdout"))?;
            }
            Ok(_) => {}
        }
        if self.options.verbose {
            eprintln!("\x1b[32mreceived: {}\x1b[0m", line.trim_end());
        }
        let response = eval_api::eval_response_from_json(line.as_str())?;
        Ok(response)
    }
    pub fn receive_until<T>(
        &mut self,
        cond: impl Fn(&mut EvalClient, &EvalResponse) -> Result<Option<T>>,
    ) -> Result<T> {
        loop {
            let response = self.receive()?;
            self.handle_response(&response)?;
            let r = cond(self, &response)?;
            match r {
                Some(r) => return Ok(r),
                None => continue,
            }
        }
    }

    pub fn next_id<T>(&mut self) -> Id<T> {
        self.ids.next()
    }

    pub fn get_error<T>(&self, id: Id<T>) -> Option<&String> {
        self.errors.get(&id.num())
    }

    pub fn check_error<T>(&self, id: Id<T>) -> Result<()> {
        if let Some(e) = self.get_error(id) {
            Err(anyhow::anyhow!("evaluation: {}", e))
        } else {
            Ok(())
        }
    }

    pub fn get_deployments(&self, id: Id<FlakeType>) -> Option<&Vec<String>> {
        self.deployments.get(&id)
    }

    pub fn get_resources(&self, id: Id<DeploymentType>) -> Option<&Vec<String>> {
        self.resources.get(&id)
    }

    fn handle_response(&mut self, response: &eval_api::EvalResponse) -> Result<()> {
        match response {
            eval_api::EvalResponse::Error(id, error) => {
                self.errors.insert(id.num(), error.clone());
            }
            eval_api::EvalResponse::QueryResponse(_id, value) => match value {
                eval_api::QueryResponseValue::ListDeployments((flake_id, deployments)) => {
                    self.deployments.insert(*flake_id, deployments.clone());
                }
                eval_api::QueryResponseValue::ListResources((deployment_id, resources)) => {
                    self.resources.insert(*deployment_id, resources.clone());
                }
                _ => {}
            },
            eval_api::EvalResponse::TracingEvent(v) => {
                let event =
                    serde_json::from_value(v.clone()).context("while parsing tracing event")?;
                if let Err(e) = self.tracing_event_receiver.try_receive(event) {
                    eprintln!("error handling tracing event: {}", e);
                }
            }
        }
        Ok(())
    }
}
