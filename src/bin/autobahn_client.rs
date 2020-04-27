use slog::Logger;
use std::str;
use structopt::StructOpt;
use yarws::{connect, log, Error, Socket};

#[macro_use]
extern crate slog;

#[derive(StructOpt, Debug)]
struct Args {
    #[structopt(default_value = "ws://127.0.0.1:9001")]
    url: String,
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let args = Args::from_args();
    let log = log::config();

    let cc = get_case_count(&args.url, log.clone()).await?;
    info!(log, "case count {}", cc);

    for i in 1..(cc + 1) {
        let l = log.new(o!("conn" => i));
        let url = format!("{}/runCase?case={}&agent=yarws", &args.url, i);
        let socket = match connect(&url, l).await {
            Ok(s) => s,
            Err(e) => {
                error!(log, "{}", e);
                return Err(e);
            }
        };

        if let Err(e) = echo(socket, log.clone()).await {
            error!(log, "{}", e);
        }
    }

    generate_report(&args.url, log.clone()).await?;

    Ok(())
}

async fn echo(mut socket: Socket, _log: Logger) -> Result<(), Error> {
    while let Some(msg) = socket.recv().await {
        socket.send(msg).await?;
    }
    Ok(())
}

async fn generate_report(root: &str, log: Logger) -> Result<(), Error> {
    let url = root.to_owned() + "/updateReports?agent=yarws";
    let mut socket = connect(&url, log).await?;
    while let Some(_msg) = socket.recv().await {}
    Ok(())
}

async fn get_case_count(root: &str, log: Logger) -> Result<usize, Error> {
    let url = root.to_owned() + "/getCaseCount";
    let mut socket = connect(&url, log).await?.into_text();
    let msg = socket.try_recv().await?;
    if let Ok(i) = msg.as_str().parse::<usize>() {
        return Ok(i);
    }
    Ok(0)
}
