// server2.rs
use std::io;
use std::io::prelude::*;
use std::net::{TcpListener, TcpStream};

#[derive(Debug)]
struct Header {
    connection: String,
    upgrade: String,
    version: String,
    key: String,
    extensions: String,
}

impl Header {
    fn new() -> Header {
        Header {
            connection: String::new(),
            upgrade: String::new(),
            version: String::new(),
            key: String::new(),
            extensions: String::new(),
        }
    }

    fn parse(&mut self, line: &str) {
        if let Some((key, value)) = parse_http_header(&line) {
            match key {
                "Connection" => self.connection = value.to_string(),
                "Upgrade" => self.upgrade = value.to_string(),
                "Sec-WebSocket-Version" => self.version = value.to_string(),
                "Sec-WebSocket-Key" => self.key = value.to_string(),
                "Sec-WebSocket-Extensions" => self.extensions = value.to_string(),
                //"Host"
                //"Origin"
                _ => println!("other: '{}' => '{}'", key, value),
            }
        }
    }

    fn is_ws_upgrade(&self) -> bool {
        self.connection == "Upgrade"
            && self.upgrade == "websocket"
            && self.version == "13"
            && self.key.len() > 0
    }
}

fn parse_http_header(line: &str) -> Option<(&str, &str)> {
    let mut splitter = line.splitn(2, ':');
    let first = splitter.next()?;
    let second = splitter.next()?;
    //println!("splitted to: {} :: {}", first, second);
    Some((first, second.trim()))
    // for s in line.split(":") {
    //     println!("split line part {0}", s);
    // }
}

fn handle_connection(stream: TcpStream) -> io::Result<()> {
    let mut ostream = stream.try_clone()?;
    let mut rdr = io::BufReader::new(stream);

    let mut header = Header::new();
    loop {
        let mut line = String::new();
        match rdr.read_line(&mut line) {
            Ok(0) => break, // eof
            Ok(2) => break, // empty line \r\n = end of header line
            Ok(_n) => {
                header.parse(&line);
                // if let Some((key, value)) = inspect(&line) {
                //     match key {
                //         "Connection" => println!("connection header: {}", value),
                //         "Upgrade" =>
                //         "Sec-WebSocket-Version"
                //         "Sec-WebSocket-Key"
                //         "Sec-WebSocket-Extensions"
                //         "Host"
                //         "Origin"
                //         _ => println!("other: '{}' => '{}'", key, value),
                //     }
                // }
                // match inspect(&line) {
                //     Some((key, value)) => println!("detected header: {} => {}", key, value),
                //     None => (),
                // }
                //print!("{} {}", n, line);
            }
            _ => break,
        }
    }
    println!("header: {:?}", header);
    println!("is websocket upgrade: {}", header.is_ws_upgrade());
    let rsp = "HTTP/1.1 200 OK\r\n\r\n{}";
    ostream.write_all(rsp.as_bytes())?;
    ostream.flush()?;
    Ok(())
}

fn main() {
    let listener = TcpListener::bind("127.0.0.1:8000").expect("could not start server");

    // accept connections and get a TcpStream
    for connection in listener.incoming() {
        match connection {
            Ok(stream) => {
                if let Err(e) = handle_connection(stream) {
                    println!("error {:?}", e);
                }
            }
            Err(e) => {
                print!("connection failed {}\n", e);
            }
        }
    }
}

/*
16 GET / HTTP/1.1
22 Host: localhost:8000
21 Connection: Upgrade
18 Pragma: no-cache
25 Cache-Control: no-cache
135 User-Agent: Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_4) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/80.0.3987.149 Safari/537.36
20 Upgrade: websocket
31 Origin: http://localhost:8000
27 Sec-WebSocket-Version: 13
36 Accept-Encoding: gzip, deflate, br
114 Accept-Language: en-US,en;q=0.9,de;q=0.8,et;q=0.7,hr;q=0.6,it;q=0.5,sk;q=0.4,sl;q=0.3,sr;q=0.2,bs;q=0.1,mt;q=0.1
45 Sec-WebSocket-Key: HNDy4+PhhRtPmNt1Xet/Ew==
70 Sec-WebSocket-Extensions: permessage-deflate; client_max_window_bits
*/
