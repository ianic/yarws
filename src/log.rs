use slog::Drain;
use slog::Logger;

pub fn config() -> Logger {
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();
    //let drain = slog::LevelFilter::new(drain, slog::Level::Info).fuse();
    // let log Logger::root(drain, o!()); // simple variant without file:line module
    Logger::root(
        drain,
        o!("file" =>
         slog::FnValue(move |info| {
             format!("{}:{} {}",
                     info.file(),
                     info.line(),
                     info.module(),
                     )
         })
        ),
    )
}

pub fn null() -> Logger {
    slog::Logger::root(slog_stdlog::StdLog.fuse(), o!())
}
