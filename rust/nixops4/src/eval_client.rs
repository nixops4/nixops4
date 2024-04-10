use std::{
    borrow::BorrowMut,
    collections::HashMap,
    io::{BufRead, Write},
    process::ChildStdout,
};

use anyhow::{Context, Result};
use nixops4_core::eval_api::{self, EvalRequest, FlakeType, Id, IdNum, Ids};

// struct ProcessWithHandles<'a> {
//     process: Arc<std::process::Child>,
//     command_handle: &'a ChildStdin,
//     // response_handle: &'a ChildStdout,
//     response_bufreader: std::io::BufReader<&'a mut ChildStdout>,
// }

pub struct EvalClient<'a> {
    // process: &'a mut std::process::Child,
    response_bufreader: &'a mut std::io::BufReader<&'a mut ChildStdout>,
    // Reference with the liftime of the process
    command_handle: &'a mut std::process::ChildStdin,
    ids: Ids,
    deployments: HashMap<Id<FlakeType>, Vec<String>>,
    errors: HashMap<IdNum, String>,
}
impl<'a> EvalClient<'a> {
    pub fn with<T>(f: impl FnOnce(EvalClient) -> Result<T>) -> Result<T> {
        let exe = std::env::var("_NIXOPS4_EVAL").unwrap_or("nixops4-eval".to_string());
        let mut process = std::process::Command::new(exe)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .arg("<subprocess>")
            .spawn()
            .context("while starting the nixops4 evaluator process")?;
        // let response_handle = process.borrow_mut().stdout.as_mut().unwrap();
        // let mut command_handle = *process.borrow_mut().stdin.as_mut().unwrap();
        let process2 = process.borrow_mut();
        let mut response_bufreader = std::io::BufReader::new(process2.stdout.as_mut().unwrap());
        let c: EvalClient<'_> = EvalClient {
            response_bufreader: &mut response_bufreader,
            // process: process2,
            command_handle: process2.stdin.as_mut().unwrap(),
            ids: Ids::new(),
            deployments: HashMap::new(),
            errors: HashMap::new(),
        };
        f(c)
    }
    pub fn send(&mut self, request: &EvalRequest) -> Result<()> {
        self.command_handle
            .write_all(eval_api::eval_request_to_json(request)?.as_bytes())?;
        self.command_handle.write_all(b"\n")?;
        self.command_handle.flush()?;
        Ok(())
    }
    pub fn receive_until<T>(
        &mut self,
        cond: impl Fn(&EvalClient<'a>) -> Result<Option<T>>,
    ) -> Result<T> {
        loop {
            // read line from response_handle
            // parse line as json
            // match json as eval_response
            // match eval_response as T
            // return T
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

            {
                let response = eval_api::eval_response_from_json(line.as_str())?;
                self.handle_response(response)?;
            }
            let r = cond(self)?;
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

    fn handle_response(&mut self, response: eval_api::EvalResponse) -> Result<()> {
        match response {
            eval_api::EvalResponse::Error(id, error) => {
                self.errors.insert(id.num(), error);
            }
            eval_api::EvalResponse::ListDeployments(id, deployments) => {
                self.deployments.insert(id, deployments);
            }
        }
        Ok(())
    }
}
