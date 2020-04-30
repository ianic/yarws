use structopt::StructOpt;
use tokio::time::{delay_for, Duration};
use yarws::Error;

#[derive(StructOpt, Debug)]
struct Args {
    #[structopt(default_value = "ws://127.0.0.1:9001")]
    url: String,

    #[structopt(short = "r", long = "repeat", default_value = "0")] // <0 repeates forever
    repeat: isize,

    #[structopt(short = "n", long = "no-wait")]
    no_wait: bool,
}

// send different sizes of text messages to the echo server
// and expect response to match the request

#[tokio::main]
async fn main() -> Result<(), Error> {
    let args = Args::from_args();
    let mut repeat = args.repeat;
    loop {
        {
            let mut socket = yarws::Client::new(&args.url)
                .default_logger()
                .connect()
                .await?
                .into_text();

            let data = "01234567890abcdefghijklmnopqrstuvwxyz"; //36 characters
            let sizes = vec![1, 36, 125, 126, 127, 65535, 65536, 65537, 1048576];
            for size in sizes {
                let rep = size / data.len() + 1;
                let req = &data.repeat(rep)[0..size];

                socket.send(req).await?;
                let rsp = socket.try_recv().await?;
                assert_eq!(req, rsp);
            }
        }
        if repeat == 0 {
            break;
        }
        repeat -= 1;
        if !args.no_wait {
            delay_for(Duration::from_secs(1)).await;
        }
    }
    Ok(())
}
