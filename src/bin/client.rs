use structopt::StructOpt;

#[derive(StructOpt, Debug)]
struct Args {
    #[structopt(default_value = "ws://127.0.0.1:9001")]
    url: String,
}

#[tokio::main]
async fn main() -> Result<(), yarws::Error> {
    let args = Args::from_args();
    let socket = yarws::connect(&args.url, None).await?;
    let mut caller = Caller { socket: socket };

    let data = "01234567890abcdefghijklmnopqrstuvwxyz"; //36 characters
    let sizes = vec![1, 36, 125, 126, 127, 65535, 65536, 65537];
    for size in sizes {
        let rep = size / data.len() + 1;
        let req = &data.repeat(rep)[0..size];

        let rsp = caller.call(req).await?;
        assert_eq!(req, rsp);
    }
    Ok(())
}

struct Caller {
    socket: yarws::Socket,
}

impl Caller {
    async fn call(&mut self, req: &str) -> Result<String, yarws::Error> {
        self.socket.send(req).await?;
        if let Some(text) = self.socket.receive().await? {
            return Ok(text);
        }
        Ok(String::new())
    }
}
