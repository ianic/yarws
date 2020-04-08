// server.rs
use slog::Drain;
use slog::Logger;
use std::error::Error;
use structopt::StructOpt;
use tokio;
use tokio::sync::mpsc::{Receiver, Sender};

#[macro_use]
extern crate failure;
#[macro_use]
extern crate slog;
extern crate slog_async;
extern crate slog_term;

#[derive(StructOpt, Debug)]
struct Args {
    #[structopt(short = "p", long = "port", default_value = "9001")]
    port: usize,

    #[structopt(short = "i", long = "bind-ip", default_value = "127.0.0.1")]
    ip: String,
}

impl Args {
    fn addr(&self) -> String {
        format!("{}:{}", self.ip, self.port)
    }
}

fn log_config() -> Logger {
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();
    //let log Logger::root(drain, o!());
    Logger::root(
        drain,
        o!("file" =>
         slog::FnValue(move |info| {
             format!("{}:{} {}",
                     info.file(),
                     info.line(),
                     info.module(),
                     )
         })
        ),
    )
}

#[tokio::main]
async fn main() {
    let args = Args::from_args();
    let log = log_config();

    match yarws::Server::bind(args.addr(), log.clone()).await {
        Ok(srv) => {
            let mut sessions = srv.sessions().await;
            while let Some(session) = sessions.recv().await {
                echo(session.rx, session.tx, log.clone()).await.unwrap();
            }
        }
        Err(e) => {
            error!(log, "failed to start server error: {}", e; "addr" => args.addr());
        }
    }
}

async fn echo(
    mut rx: Receiver<yarws::ws::Msg>,
    mut tx: Sender<yarws::ws::Msg>,
    log: slog::Logger,
) -> Result<(), Box<dyn Error>> {
    while let Some(m) = rx.recv().await {
        tx.send(m).await?;
    }
    trace!(log, "session closed");
    Ok(())
}
