use structopt::StructOpt;
use yarws::{connect, Error};

#[derive(StructOpt, Debug)]
struct Args {
    #[structopt(default_value = "ws://127.0.0.1:9001")]
    url: String,
}

// send different sizes of text messages to the echo server
// and expect response to match the request

#[tokio::main]
async fn main() -> Result<(), Error> {
    let args = Args::from_args();
    let mut socket = connect(&args.url, None).await?;

    let data = "01234567890abcdefghijklmnopqrstuvwxyz"; //36 characters
    let sizes = vec![1, 36, 125, 126, 127, 65535, 65536, 65537, 1048576];
    for size in sizes {
        let rep = size / data.len() + 1;
        let req = &data.repeat(rep)[0..size];

        socket.send_text(req).await?;
        let rsp = socket.must_recv_text().await?;
        assert_eq!(req, rsp);
    }
    Ok(())
}
