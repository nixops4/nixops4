use anyhow::Result;
use nix_expr::eval_state::{gc_registering_current_thread, EvalState};
use nix_store::store::Store;
use std::process::exit;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::runtime::Builder;
use tokio::sync::mpsc::channel;
use tokio::task::JoinHandle;

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

    // Easy requests that provide info and don't require significant computation
    // Processing these early means that we have access to more info, reducing
    // the need to re-evaluate when that info is supposedly not known yet.
    let (high_prio_tx, mut high_prio_rx) = channel(100);
    let (low_prio_tx, mut low_prio_rx) = channel(100);

    let reader_done: JoinHandle<Result<()>> = tokio::spawn(async move {
        while let Some(line) = lines.next_line().await? {
            let request = nixops4_core::eval_api::eval_request_from_json(&line)?;
            if has_prio(&request) {
                high_prio_tx.send(request).await?;
            } else {
                low_prio_tx.send(request).await?;
            }
        }
        Ok(())
    });

    // let queue_done : JoinHandle<Result<()>> = tokio::task::Builder::new().name("nixops4-eval-queue-worker").spawn(async move {
    let queue_done: JoinHandle<Result<()>> = tokio::spawn(async move {
        loop {
            while let Ok(request) = high_prio_rx.try_recv() {
                driver.perform_request(&request).await?;
            }
            // Await both queues simultaneously
            let request = tokio::select! {
                Some(request) = high_prio_rx.recv() => request,
                Some(request) = low_prio_rx.recv() => request,
                else => break,
            };
            driver.perform_request(&request).await?;
        }
        Ok(())
    });

    reader_done.await??;
    queue_done.await??;
    Ok(())
}

fn has_prio(request: &nixops4_core::eval_api::EvalRequest) -> bool {
    match request {
        nixops4_core::eval_api::EvalRequest::PutResourceOutput(_, _) => true,
        _ => false,
    }
}
