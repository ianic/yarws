// server2.rs
use std::io;
use std::io::prelude::*;
use std::net::{TcpListener, TcpStream};

fn handle_connection(stream: TcpStream) -> io::Result<()> {
    let mut rdr = io::BufReader::new(stream);
    let mut text = String::new();
    rdr.read_line(&mut text)?;
    println!("got '{}'", text.trim_end());
    Ok(())
}

fn handle_connection2(stream: TcpStream) -> io::Result<()> {
    let mut ostream = stream.try_clone()?;
    let mut rdr = io::BufReader::new(stream);
    loop {
        let mut line = String::new();
        match rdr.read_line(&mut line) {
            Ok(0) => break, // eof
            Ok(2) => break, // empty line \r\n = end of header line
            Ok(n) => print!("{} {}", n, line),
            _ => break,
        }
    }
    println!();
    let rsp = "HTTP/1.1 200 OK\r\n\r\n{}";
    ostream.write_all(rsp.as_bytes())?;
    ostream.flush()?;
    Ok(())
}

fn handle_connection_book(mut stream: TcpStream) -> io::Result<()> {
    let mut buffer = [0; 512];
    stream.read(&mut buffer)?;
    println!("Request: {}", String::from_utf8_lossy(&buffer[..]));
    Ok(())
}

fn main() {
    let listener = TcpListener::bind("127.0.0.1:8000").expect("could not start server");

    // accept connections and get a TcpStream
    for connection in listener.incoming() {
        match connection {
            Ok(stream) => {
                if let Err(e) = handle_connection2(stream) {
                    println!("error {:?}", e);
                }
            }
            Err(e) => {
                print!("connection failed {}\n", e);
            }
        }
    }
}
