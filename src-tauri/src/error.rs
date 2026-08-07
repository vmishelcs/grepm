use serde::Serialize;

/// The error type every Tauri command returns.
///
/// Tauri requires command errors to be `Serialize`, and `grepm_core::Error`
/// deliberately isn't — it wraps `rusqlite` and `io` types that have no
/// meaningful JSON form. The tempting fix is to render everything to a string
/// at the boundary, but then the front end has to parse prose to tell "that
/// name is taken" (recoverable, show it next to the field) from "the disk is
/// full" (not recoverable). So every case the UI branches on gets its own
/// variant with its data intact, and only the genuinely unexpected ones carry
/// a rendered message.
///
/// Serialized internally tagged, so the front end sees
/// `{ "kind": "name_in_use", "name": "Work chats" }` and can switch on
/// `kind`. Mirrored in `src/lib/ipc/types.ts` — change one, change the other.
#[derive(Debug, Serialize, thiserror::Error)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AppError {
    #[error("an import named \"{name}\" already exists")]
    NameInUse { name: String },

    #[error("an import needs a name")]
    EmptyName,

    #[error("{path} doesn't look like a Messenger export")]
    NotAnExport { path: String },

    /// The index still lists this import but its database file is gone —
    /// deleted or moved behind the app's back. The index is allowed to drift
    /// from the folder; this is what that drift looks like when it surfaces.
    #[error("could not find export \"{name}\"")]
    ImportFileMissing { name: String },

    #[error("no import with id {id}")]
    UnknownImport { id: String },

    #[error("this import was made by a newer grepm (schema {found} > {supported})")]
    UnsupportedSchemaVersion { found: i32, supported: i32 },

    #[error("this import's database is corrupt (schema version {found})")]
    InvalidSchemaVersion { found: i32 },

    #[error("the library index at {path} is unreadable: {message}")]
    CorruptIndex { path: String, message: String },

    #[error("{message}")]
    Io { message: String },

    #[error("{message}")]
    Parse { message: String },

    #[error("{message}")]
    Database { message: String },
}

impl From<grepm_core::Error> for AppError {
    fn from(err: grepm_core::Error) -> Self {
        use grepm_core::Error;

        // Rendered up front: the match below moves `err`, and the variants
        // that fall through to a message need its `Display` output.
        let message = err.to_string();
        match err {
            Error::UnsupportedSchemaVersion { found, supported } => {
                Self::UnsupportedSchemaVersion { found, supported }
            }
            Error::InvalidSchemaVersion { found } => Self::InvalidSchemaVersion { found },
            Error::Db(_) => Self::Database { message },
            Error::Parse { .. } => Self::Parse { message },
            // `ReadFile` already names the file in its `Display`, so the
            // context isn't lost by folding it in with plain io errors.
            Error::Io(_) | Error::ReadFile { .. } => Self::Io { message },
        }
    }
}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        Self::Io {
            message: err.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_schema_version_error_keeps_its_numbers_across_the_boundary() {
        let err = AppError::from(grepm_core::Error::UnsupportedSchemaVersion {
            found: 7,
            supported: 1,
        });

        let json = serde_json::to_value(&err).unwrap();

        assert_eq!(json["kind"], "unsupported_schema_version");
        assert_eq!(json["found"], 7);
        assert_eq!(json["supported"], 1);
    }

    #[test]
    fn an_unexpected_error_carries_a_rendered_message() {
        let err = AppError::from(grepm_core::Error::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "nope",
        )));

        let json = serde_json::to_value(&err).unwrap();

        assert_eq!(json["kind"], "io");
        assert_eq!(json["message"], "nope");
    }

    #[test]
    fn the_ui_facing_variants_serialize_their_data() {
        let json = serde_json::to_value(AppError::NameInUse {
            name: "Work chats".into(),
        })
        .unwrap();

        assert_eq!(json["kind"], "name_in_use");
        assert_eq!(json["name"], "Work chats");
    }
}
