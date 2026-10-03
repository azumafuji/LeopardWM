use leopardwm_ipc::DaemonLogStatus;
use std::io::{self, Write};
use std::path::Path;
use std::sync::{Arc, OnceLock};
use tracing::Level;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::time::{FormatTime, SystemTime};
use tracing_subscriber::fmt::MakeWriter;

#[derive(Clone)]
pub(crate) struct LogHealth {
    initial: DaemonLogStatus,
    first_write_error: Arc<OnceLock<String>>,
}

impl LogHealth {
    fn new(initial: DaemonLogStatus) -> Self {
        Self {
            initial,
            first_write_error: Arc::new(OnceLock::new()),
        }
    }

    pub(crate) fn status(&self) -> DaemonLogStatus {
        match (&self.initial, self.first_write_error.get()) {
            (DaemonLogStatus::Writing { path }, Some(error)) => DaemonLogStatus::WriteFailed {
                path: path.clone(),
                error: error.clone(),
            },
            _ => self.initial.clone(),
        }
    }

    fn record<T>(&self, result: io::Result<T>) -> io::Result<T> {
        if let Err(error) = &result {
            let _ = self.first_write_error.set(error.to_string());
        }
        result
    }
}

pub(crate) struct LogWriter<W> {
    inner: W,
    health: LogHealth,
}

impl<'a, W: MakeWriter<'a>> MakeWriter<'a> for LogWriter<W> {
    type Writer = LogWriter<W::Writer>;

    fn make_writer(&'a self) -> Self::Writer {
        LogWriter {
            inner: self.inner.make_writer(),
            health: self.health.clone(),
        }
    }
}

impl<W: Write> Write for LogWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.health.record(self.inner.write(buf))
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.health.record(self.inner.write_all(buf))
    }

    fn flush(&mut self) -> io::Result<()> {
        self.health.record(self.inner.flush())
    }
}

pub(crate) fn open(
    log_dir: &Path,
    log_level: Level,
) -> (Option<LogWriter<RollingFileAppender>>, LogHealth) {
    let path = log_dir.join("leopardwm-daemon.log").display().to_string();
    match RollingFileAppender::builder()
        .rotation(Rotation::NEVER)
        .filename_prefix("leopardwm-daemon.log")
        .build(log_dir)
    {
        Ok(inner) => {
            let health = LogHealth::new(DaemonLogStatus::Writing { path });
            let mut writer = LogWriter {
                inner,
                health: health.clone(),
            };
            let mut timestamp = String::new();
            SystemTime
                .format_time(&mut Writer::new(&mut timestamp))
                .expect("formatting a timestamp into a String cannot fail");
            let _ = writeln!(
                writer,
                "{timestamp} LeopardWM daemon {} (pid {}) opened this log at log level {}",
                env!("CARGO_PKG_VERSION"),
                std::process::id(),
                log_level.as_str().to_ascii_lowercase()
            );
            (Some(writer), health)
        }
        Err(error) => {
            eprintln!("[leopardwm] Cannot open daemon log {path}: {error}");
            (
                None,
                LogHealth::new(DaemonLogStatus::OpenFailed {
                    path,
                    error: error.to_string(),
                }),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_failure_is_reported_without_panicking() {
        let path =
            std::env::temp_dir().join(format!("leopardwm-log-blocked-{}", std::process::id()));
        std::fs::write(&path, b"not a directory").unwrap();
        let (writer, health) = open(&path, Level::INFO);
        std::fs::remove_file(&path).unwrap();
        assert!(writer.is_none());
        assert!(
            matches!(health.status(), DaemonLogStatus::OpenFailed { path: reported, error } if reported == path.join("leopardwm-daemon.log").display().to_string() && !error.is_empty())
        );
    }

    #[test]
    fn never_rotation_writes_the_exact_daemon_filename() {
        let dir =
            std::env::temp_dir().join(format!("leopardwm-log-writing-{}", std::process::id()));
        let (writer, health) = open(&dir, Level::WARN);
        let writer = writer.unwrap();
        let startup = std::fs::read_to_string(dir.join("leopardwm-daemon.log")).unwrap();
        let (timestamp, message) = startup.split_once(' ').unwrap();
        assert!(timestamp.ends_with('Z'));
        assert_eq!(
            message,
            format!(
                "LeopardWM daemon {} (pid {}) opened this log at log level warn\n",
                env!("CARGO_PKG_VERSION"),
                std::process::id()
            )
        );
        writer.make_writer().write_all(b"log entry\n").unwrap();
        drop(writer);
        assert_eq!(
            std::fs::read_to_string(dir.join("leopardwm-daemon.log")).unwrap(),
            format!("{startup}log entry\n")
        );
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        assert!(matches!(health.status(), DaemonLogStatus::Writing { .. }));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn writer_preserves_first_error_and_keeps_attempting_writes() {
        struct FailingWriter(usize);
        impl Write for FailingWriter {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                self.0 += 1;
                Err(io::Error::other(format!("failure {}", self.0)))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let health = LogHealth::new(DaemonLogStatus::Writing {
            path: "daemon.log".into(),
        });
        let mut writer = LogWriter {
            inner: FailingWriter(0),
            health: health.clone(),
        };
        assert!(writer.write_all(b"first").is_err());
        assert!(writer.write_all(b"second").is_err());
        assert_eq!(writer.inner.0, 2);
        assert_eq!(
            health.status(),
            DaemonLogStatus::WriteFailed {
                path: "daemon.log".into(),
                error: "failure 1".into()
            }
        );
    }
}
