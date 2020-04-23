# Changelog
All notable changes to this project will be documented in this file.

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