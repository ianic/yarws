use tokio;
use tokio::spawn;
use tokio::sync::mpsc;
use tokio::sync::mpsc::{Receiver, Sender};

#[tokio::main]
async fn main() {
    let (mut tx, mut rx): (Sender<String>, Receiver<String>) = mpsc::channel(1);
    spawn(async move {
        tx.send("pero".to_owned()).await.unwrap();
        tx.send("zdero".to_owned()).await.unwrap();
    });
    while let Some(text) = rx.recv().await {
        println!("recived: {0}", text);
    }
}
