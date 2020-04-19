use structopt::StructOpt;
use yarws::{connect, log, Error};

#[derive(StructOpt, Debug)]
struct Args {
    #[structopt(default_value = "ws://127.0.0.1:9001")]
    url: String,
}

// Sends different sizes of text messages to the echo server.
// Expect response to match the request.
#[tokio::main]
async fn main() -> Result<(), Error> {
    let args = Args::from_args();
    // connect to the server and switch to text only mode
    let mut socket = connect(&args.url, log::config()).await?.into_text();

    let data = "01234567890abcdefghijklmnopqrstuvwxyz"; //36 characters
    let sizes = vec![1, 36, 125, 126, 127, 65535, 65536, 65537, 1048576];
    for size in sizes {
        // create request with `size` number of characters
        let rep = size / data.len() + 1;
        let req = &data.repeat(rep)[0..size];

        socket.send(req).await?; // send request
        let rsp = socket.recv_one().await?; // receive response
        assert_eq!(req, rsp);
    }
    Ok(())
}
