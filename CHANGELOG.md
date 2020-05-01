# Changelog
All notable changes to this project will be documented in this file.


## [0.3.1] - 2020-05-01
### Added: 
- HTTP headers are now [visible](https://github.com/ianic/yarws/blob/ac3a1662d361b904ff6b2810349470ca92acc0c1/src/lib.rs#L333) after successful client or server connection.

## [0.3.0] - 2020-05-01
### Added: 
- Setting custom http headers and cookies on Client.
- [Builder](https://doc.rust-lang.org/1.0.0/style/ownership/builders.html) pattern for setting options while creating both Server and Client.

### Fixed:
- All documentation examples are now runnable.
- Project has [roadmap](https://github.com/ianic/yarws/projects/1) now.

## [0.2.1] - 2020-04-28
### Added:
- Buffered read. From [BufReader](https://docs.rs/tokio/0.2.19/tokio/io/struct.BufReader.html) docs: "It can be excessively inefficient to work directly with a AsyncRead instance. A BufReader performs large, infrequent reads on the underlying AsyncRead and maintains an in-memory buffer of the results." 
In the previous implementation we were reading http header byte by byte, ws headers also few bytes at the time. Now we are using BufReader with buffer size of 1024, so number of system calls should be much lower.


## [0.2.0] - 2020-04-23
### Added:
- Tls in connect. It is now possible to connect to the wss:// endpoints. Thanks to [mcseemk](https://www.reddit.com/r/rust/comments/g4zoip/ann_yet_another_rust_websocket_library_yarws/) for pointing  that.

### Fixed:
- Possible [leak](https://github.com/ianic/yarws/commit/abecb1cecda5c80a75a26968379b4d4e95aafafd) of tasks. When stream goes out of scope there was no guarantee that reader and writer tasks are closed.
- Removed unnecessary [copy](https://github.com/ianic/yarws/commit/e0c710a8d2e45d5f721bc167471109a4915b0016) of each incoming message.

## [0.1.2] - 2020-04-19
Initial crate release.

## [Unreleased]

## Notes
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).