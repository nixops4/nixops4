use anyhow::Result;
use nix_expr::eval_state::{gc_registering_current_thread, EvalState};
use nix_store::store::Store;
use std::process::exit;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::runtime::Builder;

pub mod eval;

fn main() {
    // Be friendly to the user if they try to run this.
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 || args[1] != "<subprocess>" {
        eprintln!("nixops4-eval is not for direct use");
        exit(1);
    }
    let r = gc_registering_current_thread(|| {
        let runtime = Builder::new_current_thread().build()?;
        runtime.block_on(async_main())?;
        Ok(())
    });
    let r = r.and_then(|x| x);
    handle_err(r)
}

fn handle_err(r: Result<()>) {
    match r {
        Ok(()) => (),
        Err(e) => {
            eprintln!("Error: {}", e);
            exit(1);
        }
    }
}

async fn async_main() -> Result<()> {
    // Session output handle
    struct Session {
        out: Box<dyn tokio::io::AsyncWrite + Unpin + Send>,
    }

    #[async_trait::async_trait]
    impl eval::Respond for Session {
        async fn call(&mut self, response: nixops4_core::eval_api::EvalResponse) -> Result<()> {
            let s = nixops4_core::eval_api::eval_response_to_json(&response)?;
            self.out.write_all(s.as_bytes()).await?;
            self.out.write_all(b"\n").await?;
            self.out.flush().await?;
            Ok(())
        }
    }

    let session = Session {
        out: Box::new(tokio::io::stdout()),
    };

    let store = Store::open("auto", [])?;
    let eval_state = EvalState::new(store, [])?;

    let mut driver = eval::EvaluationDriver::new(eval_state, Box::new(session));

    // Read lines from stdin and pass them to the driver
    let stdin = tokio::io::stdin();
    let reader = BufReader::new(stdin);
    let mut lines = reader.lines();

    let r = tokio::spawn(async move {
        while let Some(line) = lines.next_line().await? {
            let request = nixops4_core::eval_api::eval_request_from_json(&line)?;
            driver.perform_request(&request).await?;
        }
        Ok(())
    });
    r.await?
}
