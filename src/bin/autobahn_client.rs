use slog::Logger;
use std::str;
use structopt::StructOpt;

#[macro_use]
extern crate slog;

#[derive(StructOpt, Debug)]
struct Args {
    #[structopt(default_value = "ws://127.0.0.1:9001")]
    url: String,
}

#[tokio::main]
async fn main() -> Result<(), yarws::Error> {
    let args = Args::from_args();
    let log = yarws::log::config();

    let cc = get_case_count(&args.url, log.clone()).await?;
    info!(log, "case count {}", cc);

    for i in 1..(cc + 1) {
        let l = log.new(o!("conn" => i));
        let url = format!("{}/runCase?case={}&agent=yarws", &args.url, i);
        let session = match yarws::connect(&url, l).await {
            Ok(s) => s,
            Err(e) => {
                error!(log, "{}", e);
                return Err(e);
            }
        };

        if let Err(e) = echo(session, log.clone()).await {
            error!(log, "{}", e);
        }
    }

    generate_report(&args.url, log.clone()).await?;

    Ok(())
}

async fn echo(mut s: yarws::Session, _log: Logger) -> Result<(), yarws::Error> {
    while let Some(m) = s.rx.recv().await {
        s.tx.send(m).await?;
    }
    Ok(())
}

async fn generate_report(root: &str, log: Logger) -> Result<(), yarws::Error> {
    let url = root.to_owned() + "/updateReports?agent=yarws";
    let mut session = yarws::connect(&url, log).await?;
    while let Some(_m) = session.rx.recv().await {}
    Ok(())
}

async fn get_case_count(root: &str, log: Logger) -> Result<usize, yarws::Error> {
    let url = root.to_owned() + "/getCaseCount";
    let mut session = yarws::connect(&url, log).await?;
    if let Some(s) = session.receive().await? {
        if let Ok(i) = s.as_str().parse::<usize>() {
            return Ok(i);
        }
    }
    Ok(0)
}
