use clap::Parser;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    // The command is parsed before the runtime is built, because it decides
    // which runtime to build. See `Cli::wants_worker_threads`: a hook has
    // nothing to schedule in parallel, and the multi-threaded scheduler spawns
    // a worker per core before any of this runs.
    let cli = leteo::cli::Cli::parse();
    let mut runtime = if cli.wants_worker_threads() {
        tokio::runtime::Builder::new_multi_thread()
    } else {
        tokio::runtime::Builder::new_current_thread()
    };
    let answer = runtime.enable_all().build()?.block_on(leteo::cli::run(cli));
    // A store somebody else is writing to is not a broken one.
    //
    // Every other failure here is worth its whole chain: a person debugging a
    // real fault wants the cause. This one is not a fault. It printed
    // `Error: database error: database is locked` and then `Error code 5:
    // database is locked` twice more, which reads like a damaged file and is a
    // store that was in use — the tools have said so since `store_busy`, the
    // hooks say it now, and this was the last surface handing SQLite's words
    // to a person.
    match answer {
        Err(error)
            if error
                .downcast_ref::<leteo::StoreError>()
                .is_some_and(leteo::StoreError::is_busy) =>
        {
            eprintln!("leteo: {}", leteo::StoreError::BUSY_ADVICE);
            std::process::exit(1);
        }
        other => other,
    }
}
