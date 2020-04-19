# yarws

WebSocket server and client implementation.
Based on [Tokio] runtime.

Per message deflate is implemented for incoming messages. Lib can receive compressed messages.
Currently all outgoing messages are sent uncompressed.

Lib is passing all [autobahn] tests. Including those for compressed messages.


## Examples

### Server:
```rust
let mut srv = yarws::Server::bind("127.0.0.1:9001", None).await?;
while let Some(socket) = srv.accept().await {
    spawn(async move {
        while let Some(msg) = socket.recv().await {
            socket.send(msg).await.unwrap();
        }
    }
}
```
This is an example of echo server. We are replying with the same message on each incoming message.
First line starts listening for WebSocket connections on an ip:port.
Each client is represented by [`Socket`] returned from [`accept`].
For each client we are looping while messages arrive and replying with the same message.
For the complete echo server example refer to [src/bin/echo_server.rs].

### Client:
```rust
let mut socket = yarws::connect("ws://127.0.0.1:9001", None).await?;
while let Some(msg) = socket.recv().await {
    socket.send(msg).await?;
}
```
This is example of an echo client.
[`connect`] method returns [`Socket`] which is used to send and receive messages.
Looping on recv returns each incoming message until socket is closed.
Here in loop we reply with the same message.
For the complete echo client example refer to [src/bin/echo_client.rs].


## Testing
Run client with external echo server.
```shell
cargo run --bin client -- ws://echo.websocket.org
```
Client will send few messages of different sizes and expect to get the same in return.
If everything went fine will finish without error.

To run same client on our server. First start server:
```shell
cargo run --bin echo_server
```
Then in other terminal run client:
```shell
cargo run --bin client
```
If it is in trace log mode server will log type and size of every message it receives.

### websocat test tool
You can use [websocat] to connect to the server and test communication.
First start server:
```shell
cargo run --bin echo_server
```
Then in other terminal run websocat:
```shell
websocat -E --linemode-strip-newlines ws://127.0.0.1:9001
```
Type you message press enter to send it and server will reply with the same message.
For more exciting server run it in with reverse flag:
```shell
cargo run --bin echo_server -- --reverse
```
and than use websocat to send text messages.

## Autobahn tests
Ensure that you have [wstest] autobahn-testsuite test tool installed:
```shell
pip install autobahntestsuite
```
Start echo_server:
```shell
cargo run --bin echo_server
```
In another terminal run server tests and view results:
```shell
cd autobahn
wstest -m fuzzingclient
open reports/server/index.html
```

For testing client implementation first start autobahn server suite:
```shell
wstest -m fuzzingserver
```
Then in another terminal run client tests and view results:
```shell
cargo run --bin autobahn_client
open autobahn/reports/client/index.html
```
For development purpose there is automation for running autobahn test suite and showing results:
```shell
cargo run --bin autobahn_server_test
```
you can use run that in development on every file change with cargo-watch:
```shell
cargo watch -x 'run --bin autobahn_server_test'
```

## Chat server example
Simple example of server accepting text messages and distributing them to the all connected clients.
First start chat server:
```shell
cargo run --bin chat_server
```
Then in browser development console connect to the server and send chat messages:
```javascript
var socket = new WebSocket('ws://127.0.0.1:9001');
var msgNo = 0;
var interval;
socket.addEventListener('open', function (event) {
    console.log('open');
    socket.send("new client");
    interval = setInterval(function() {
        msgNo++;
        socket.send("message: " + msgNo);
    }, 1000);
});
socket.addEventListener('message', function (event) {
    console.log('chat', event.data);
});
socket.addEventListener('close', function (event) {
    console.log('closed');
    clearInterval(interval);
});
```
Start multiple browser tabs with the same code running.
You can disconnect from the server with: `socket.close();`.

## References
[WebSocket Protocol] IETF RFC 6455
[MDN writing WebSocket servers]

[WebSocket Protocol]: https://tools.ietf.org/html/rfc6455
[MDN writing WebSocket servers]: https://developer.mozilla.org/en-US/docs/Web/API/WebSockets_API/Writing_WebSocket_servers

[`Socket`]: struct.Socket.html
[`accept`]: struct.Server.html#method.accept
[src/bin/echo_client.rs]: https://github.com/ianic/yarws/blob/master/src/bin/echo_client.rs
[src/bin/echo_server.rs]: https://github.com/ianic/yarws/blob/master/src/bin/echo_server.rs
[websocat]: https://github.com/vi/websocat
[wstest]: https://github.com/crossbario/autobahn-testsuite
[autobahn]: https://github.com/crossbario/autobahn-testsuite
[cargo-watch]: https://github.com/passcod/cargo-watch
[Tokio]: https://tokio.rs

