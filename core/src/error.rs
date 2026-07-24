use std::io;
use std::path::PathBuf;

/// Convenience alias used by every fallible public function in this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// The crate-level error type: everything fallible in `grepm_core` returns
/// this, so callers (e.g. the Tauri layer) can match on what actually went
/// wrong instead of unwrapping a `Box<dyn Error>` or a backend-specific
/// error type.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] io::Error),

    #[error(transparent)]
    Db(#[from] rusqlite::Error),

    #[error("failed to read {}: {source}", path.display())]
    ReadFile { path: PathBuf, source: io::Error },

    #[error("failed to parse {}: {source}", path.display())]
    Parse {
        path: PathBuf,
        source: serde_json::Error,
    },

    #[error(
        "database schema version {found} is newer than this build supports \
         ({supported}); it was probably created by a newer version of the app"
    )]
    UnsupportedSchemaVersion { found: i32, supported: i32 },
}

#[cfg(test)]
mod tests {
    use super::*;

    // The Tauri layer will move these errors across thread and async
    // boundaries, which requires Send + Sync.
    #[test]
    fn error_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Error>();
    }

    #[test]
    fn parse_error_message_names_the_offending_file() {
        let source = serde_json::from_str::<i32>("not json").unwrap_err();
        let err = Error::Parse {
            path: PathBuf::from("/export/messages/inbox/conv/message_1.json"),
            source,
        };

        assert!(
            err.to_string()
                .contains("/export/messages/inbox/conv/message_1.json"),
            "error message should include the file path: {err}"
        );
    }
}
