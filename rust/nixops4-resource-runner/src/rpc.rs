use jsonrpsee::async_client::{Client, ClientBuilder};
use nixops4_resource::rpc::{ContentLengthReceiver, ContentLengthSender};
use tokio::process;

pub(crate) fn build_rpc_client_from_child(process: &mut process::Child) -> Client {
    let sender = ContentLengthSender::new(process.stdin.take().unwrap());
    let receiver = ContentLengthReceiver::new(process.stdout.take().unwrap());

    ClientBuilder::new().build_with_tokio(sender, receiver)
}
