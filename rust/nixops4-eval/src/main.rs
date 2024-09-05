use std::{
    io::{BufRead, Write},
    process::exit,
};

use anyhow::Result;
use nix_expr::eval_state::{gc_registering_current_thread, EvalState};
use nix_store::store::Store;

pub mod eval;

fn main() {
    // Be friendly to the user if they try to run this.
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 || args[1] != "<subprocess>" {
        eprintln!("nixops4-eval is not for direct use");
        std::process::exit(1);
    }

    // Session output handle

    struct Session {
        out: Box<dyn Write>,
    }
    impl eval::Respond for Session {
        // TODO: ignore these errors in main?
        fn call(&mut self, response: nixops4_core::eval_api::EvalResponse) -> Result<()> {
            let s = nixops4_core::eval_api::eval_response_to_json(&response)?;
            self.out.write_all(s.as_bytes())?;
            self.out.write_all(b"\n")?;
            self.out.flush()?;
            Ok(())
        }
    }
    let session = Session {
        out: Box::new(std::io::stdout()),
    };

    let status = gc_registering_current_thread(|| -> Result<()> {
        let store = Store::open("auto", [])?;
        let eval_state = EvalState::new(store, [])?;

        let mut driver = eval::EvaluationDriver::new(eval_state, Box::new(session));

        // Read lines from stdin and pass them to the driver
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let line = line?;
            let request = nixops4_core::eval_api::eval_request_from_json(&line)?;
            driver.perform_request(&request)?;
        }

        Ok(())
    })
    .and_then(|r| r);

    match status {
        Ok(()) => {}
        Err(e) => {
            eprintln!("nixops4-eval fatal error: {}", e);
            exit(1);
        }
    }
}
