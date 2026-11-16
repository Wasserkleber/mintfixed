// Logging initialization shared by mintfixed and its hook.
// Mechanically moved from mint_lib/src/lib.rs; behavior is unchanged.
use crate::built_info;
use anyhow::Result;
use fs_err as fs;
use std::{io::BufWriter, path::Path};
use tracing::*;
use tracing_subscriber::fmt::format::FmtSpan;

pub fn setup_logging<P: AsRef<Path>>(
    log_path: P,
    target: &str,
) -> Result<tracing_appender::non_blocking::WorkerGuard> {
    use tracing::metadata::LevelFilter;
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::{
        EnvFilter,
        field::RecordFields,
        filter,
        fmt::{
            self, FormatFields,
            format::{Pretty, Writer},
        },
    };

    /// Workaround for <https://github.com/tokio-rs/tracing/issues/1817>.
    struct NewType(Pretty);

    impl<'writer> FormatFields<'writer> for NewType {
        fn format_fields<R: RecordFields>(
            &self,
            writer: Writer<'writer>,
            fields: R,
        ) -> core::fmt::Result {
            self.0.format_fields(writer, fields)
        }
    }

    // Append instead of truncating so cleanup history survives restarts.
    let f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path.as_ref())?;
    let writer = BufWriter::new(f);
    let (log_file_appender, guard) = tracing_appender::non_blocking(writer);
    let debug_file_log = fmt::layer()
        .with_writer(log_file_appender)
        .fmt_fields(NewType(Pretty::default()))
        .with_ansi(false)
        .with_filter(filter::Targets::new().with_target(target, Level::DEBUG));
    let stderr_log = fmt::layer()
        .with_writer(std::io::stderr)
        .event_format(tracing_subscriber::fmt::format().without_time())
        .with_span_events(FmtSpan::CLOSE)
        .with_filter(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env_lossy(),
        );
    let subscriber = tracing_subscriber::registry()
        .with(stderr_log)
        .with(debug_file_log);

    tracing::subscriber::set_global_default(subscriber)?;

    debug!("tracing subscriber setup");
    info!("writing logs to {:?}", log_path.as_ref().display());
    info!("version: {}", built_info::version());

    Ok(guard)
}
