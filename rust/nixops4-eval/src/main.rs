use anyhow::Result;
use nix_expr::eval_state::{gc_registering_current_thread, EvalState};
use nix_store::store::Store;
use std::process::exit;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::runtime::Builder;
use tokio::sync::mpsc::{channel, Sender};
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;

pub mod eval;
mod json_tracing;

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
        // out: Box<dyn tokio::io::AsyncWrite + Unpin + Send>,
        sender: Sender<nixops4_core::eval_api::EvalResponse>,
    }

    #[async_trait::async_trait]
    impl eval::Respond for Session {
        async fn call(&mut self, response: nixops4_core::eval_api::EvalResponse) -> Result<()> {
            self.sender.send(response).await?;
            Ok(())
        }
    }

    // An effectively unbounded channel. We don't want to drop logs.
    let (eval_tx, mut eval_rx) = channel(Semaphore::MAX_PERMITS);

    let session = Session {
        sender: eval_tx.clone(),
    };

    {
        // Downgrade eval_tx so that we can drop it when all the real work is done, closing the log channel.
        let tx = eval_tx.downgrade();
        let log_subscriber = json_tracing::QueueSubscriber::new(Box::new(move |json| {
            if let Some(tx) = tx.upgrade() {
                let _ = tx.try_send(nixops4_core::eval_api::EvalResponse::TracingMessage(json));
            } else {
                eprintln!("warning: can't log after log channel is closed; some structured logs may be lost");
            }
        }));
        tracing::subscriber::set_global_default(log_subscriber)?;
    }

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
        let span = tracing::info_span!("nixops4-eval-stdin-reader");
        while let Some(line) = lines.next_line().await? {
            let request = nixops4_core::eval_api::eval_request_from_json(&line)?;
            if has_prio(&request) {
                high_prio_tx.send(request).await?;
            } else {
                low_prio_tx.send(request).await?;
            }
        }
        drop(span);
        Ok(())
    });

    let writer_done: JoinHandle<Result<()>> = tokio::spawn(async move {
        while let Some(response) = eval_rx.recv().await {
            let mut s = nixops4_core::eval_api::eval_response_to_json(&response)?;
            s.push('\n');
            tokio::io::stdout().write_all(s.as_bytes()).await?;
        }
        Ok(())
    });

    // let queue_done : JoinHandle<Result<()>> = tokio::task::Builder::new().name("nixops4-eval-queue-worker").spawn(async move {
    let queue_done: JoinHandle<Result<()>> = tokio::spawn(async move {
        let span = tracing::info_span!("nixops4-eval-high-prio-queue-worker");
        loop {
            while let Ok(request) = high_prio_rx.try_recv() {
                let ed = span.enter();
                driver.perform_request(&request).await?;
                drop(ed)
            }
            // Await both queues simultaneously
            let request = tokio::select! {
                Some(request) = high_prio_rx.recv() => request,
                Some(request) = low_prio_rx.recv() => request,
                else => break,
            };
            let ed = span.enter();
            driver.perform_request(&request).await?;
            drop(ed)
        }
        drop(span);
        Ok(())
    });

    reader_done.await??;
    queue_done.await??;
    drop(eval_tx);
    writer_done.await??;
    Ok(())
}

fn has_prio(request: &nixops4_core::eval_api::EvalRequest) -> bool {
    match request {
        nixops4_core::eval_api::EvalRequest::PutResourceOutput(_, _) => true,
        _ => false,
    }
}
