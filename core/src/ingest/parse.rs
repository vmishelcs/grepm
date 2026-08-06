use std::fs;
use std::mem;
use std::path::Path;

use serde::Deserialize;

use crate::{Error, Result};

pub fn parse_conversation_file(path: impl AsRef<Path>) -> Result<RawConversationFile> {
    let path = path.as_ref();
    let text = fs::read_to_string(path).map_err(|source| Error::ReadFile {
        path: path.to_path_buf(),
        source,
    })?;
    let mut file: RawConversationFile =
        serde_json::from_str(&text).map_err(|source| Error::Parse {
            path: path.to_path_buf(),
            source,
        })?;
    file.repair_mojibake();
    Ok(file)
}

/// Repairs Messenger's export bug where non-ASCII text is mangled by
/// encoding it as UTF-8 and then mis-decoding those bytes as Latin-1 (so
/// e.g. an em dash comes through as the three characters `â`, U+0080,
/// U+0094 instead of one `—`). Reversing that requires the opposite: treat
/// each `char` as a single Latin-1 byte, then decode that byte sequence as
/// UTF-8.
///
/// Real exports never contain "already correct" non-ASCII text (the bug is
/// systemic), so this is applied unconditionally rather than trying to
/// detect whether a given string needs it. It's still defensive: any
/// `char` outside the Latin-1 range (which no genuinely mojibake-shaped
/// string can contain) or a byte sequence that isn't valid UTF-8 once
/// reinterpreted leaves the original text untouched.
fn repair_mojibake(text: String) -> String {
    if text.is_ascii() {
        return text;
    }

    let mut bytes = Vec::with_capacity(text.len());
    for c in text.chars() {
        match u8::try_from(c as u32) {
            Ok(byte) => bytes.push(byte),
            Err(_) => return text,
        }
    }

    String::from_utf8(bytes).unwrap_or(text)
}

#[derive(Debug, Deserialize)]
pub struct RawConversationFile {
    pub participants: Vec<RawParticipant>,
    #[serde(default)]
    pub messages: Vec<RawMessage>,
    /// Required, unlike most fields here, because `(title, thread_path)` is
    /// the key that stitches a conversation's `message_N.json` files into
    /// one thread. SQLite treats NULLs as distinct in a unique index, so a
    /// file missing either one would silently become its own conversation
    /// instead of merging. Real exports always populate both; rejecting the
    /// file at the door turns that assumption into a checked, well-located
    /// error rather than a split thread nobody notices.
    pub title: String,
    pub is_still_participant: Option<bool>,
    /// Required — see [`RawConversationFile::title`].
    pub thread_path: String,
}

impl RawConversationFile {
    fn repair_mojibake(&mut self) {
        self.title = repair_mojibake(mem::take(&mut self.title));

        for participant in &mut self.participants {
            participant.name = repair_mojibake(mem::take(&mut participant.name));
        }

        for message in &mut self.messages {
            if let Some(sender_name) = message.sender_name.take() {
                message.sender_name = Some(repair_mojibake(sender_name));
            }
            if let Some(content) = message.content.take() {
                message.content = Some(repair_mojibake(content));
            }
            for reaction in &mut message.reactions {
                reaction.reaction = repair_mojibake(mem::take(&mut reaction.reaction));
                reaction.actor = repair_mojibake(mem::take(&mut reaction.actor));
            }
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct RawParticipant {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct RawMessage {
    pub sender_name: Option<String>,
    pub timestamp_ms: i64,
    pub content: Option<String>,
    #[serde(default)]
    pub reactions: Vec<RawReaction>,
    #[serde(default)]
    pub photos: Vec<RawAttachment>,
    #[serde(default)]
    pub videos: Vec<RawAttachment>,
    #[serde(default)]
    pub audio_files: Vec<RawAttachment>,
    #[serde(default)]
    pub gifs: Vec<RawAttachment>,
}

impl RawMessage {
    /// Classifies this message by which attachment list it carries; a
    /// message with no attachments is plain text. An attachment list wins
    /// over `content`, since an attachment message may also carry a text
    /// caption.
    pub fn message_type(&self) -> MessageType {
        if !self.photos.is_empty() {
            MessageType::Photos
        } else if !self.videos.is_empty() {
            MessageType::Videos
        } else if !self.audio_files.is_empty() {
            MessageType::AudioFiles
        } else if !self.gifs.is_empty() {
            MessageType::Gifs
        } else {
            MessageType::Text
        }
    }

    /// The total number of attachments across all four lists, part of the
    /// messages dedup key: without it, two attachment-only messages from
    /// the same sender in the same millisecond (both `content = ''`, same
    /// type) would be collapsed into one whenever their attachment
    /// counts differ. `0` for a plain text message, matching the column's
    /// `DEFAULT 0`.
    pub fn attachment_count(&self) -> i64 {
        (self.photos.len() + self.videos.len() + self.audio_files.len() + self.gifs.len()) as i64
    }
}

/// The message shapes ingestion currently distinguishes, derived from which
/// attachment list a raw message carries (see [`RawMessage::message_type`]).
/// Stored in the `messages.type` column as the string form from
/// [`Self::as_str`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessageType {
    Text,
    Photos,
    Videos,
    AudioFiles,
    Gifs,
}

impl MessageType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Photos => "photos",
            Self::Videos => "videos",
            Self::AudioFiles => "audio_files",
            Self::Gifs => "gifs",
        }
    }
}

/// One entry in a message's attachment list. Facebook's export carries a
/// `uri` and, for most kinds, a `creation_timestamp`; only the list's
/// *presence* matters for classification, so both fields are optional
/// rather than risking a whole-file parse failure over them.
#[derive(Debug, Deserialize)]
pub struct RawAttachment {
    pub uri: Option<String>,
    /// Unix timestamp in *seconds* — unlike `RawMessage::timestamp_ms`,
    /// which is in milliseconds.
    pub creation_timestamp: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct RawReaction {
    pub reaction: String,
    pub actor: String,
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;
    use crate::test_util::write_file;

    #[test]
    fn parse_conversation_file_parses_a_file() {
        let export = tempdir().unwrap();
        let file = export.path().join("message_1.json");
        write_file(
            &file,
            r#"{
                "participants": [{"name": "Alice"}, {"name": "Bob"}],
                "messages": [
                    {
                        "sender_name": "Alice",
                        "timestamp_ms": 1000,
                        "content": "hi there"
                    }
                ],
                "title": "Alice",
                "is_still_participant": true,
                "thread_path": "inbox/conv_a"
            }"#,
        );

        let parsed = parse_conversation_file(&file).unwrap();

        assert_eq!(parsed.title, "Alice");
        assert_eq!(parsed.is_still_participant, Some(true));
        assert_eq!(
            parsed
                .participants
                .iter()
                .map(|p| &p.name)
                .collect::<Vec<_>>(),
            vec!["Alice", "Bob"]
        );
        assert_eq!(parsed.messages.len(), 1);
        assert_eq!(parsed.messages[0].sender_name.as_deref(), Some("Alice"));
        assert_eq!(parsed.messages[0].timestamp_ms, 1000);
        assert_eq!(parsed.messages[0].content.as_deref(), Some("hi there"));
    }

    #[test]
    fn parse_conversation_file_defaults_reactions_to_empty_when_absent() {
        let export = tempdir().unwrap();
        let file = export.path().join("message_1.json");
        write_file(
            &file,
            r#"{
                "participants": [{"name": "Alice"}],
                "messages": [
                    {"sender_name": "Alice", "timestamp_ms": 1000, "content": "hi"}
                ],
                "title": "Alice",
                "is_still_participant": true,
                "thread_path": "inbox/conv_a"
            }"#,
        );

        let parsed = parse_conversation_file(&file).unwrap();

        assert!(parsed.messages[0].reactions.is_empty());
    }

    #[test]
    fn parse_conversation_file_parses_reactions_when_present() {
        let export = tempdir().unwrap();
        let file = export.path().join("message_1.json");
        write_file(
            &file,
            r#"{
                "participants": [{"name": "Alice"}, {"name": "Bob"}],
                "messages": [
                    {
                        "sender_name": "Alice",
                        "timestamp_ms": 1000,
                        "content": "hi",
                        "reactions": [
                            {"reaction": "â¤", "actor": "Bob"}
                        ]
                    }
                ],
                "title": "Alice",
                "is_still_participant": true,
                "thread_path": "inbox/conv_a"
            }"#,
        );

        let parsed = parse_conversation_file(&file).unwrap();

        let reactions = &parsed.messages[0].reactions;
        assert_eq!(reactions.len(), 1);
        assert_eq!(reactions[0].actor, "Bob");
        // Repaired from the raw "â¤" mojibake into the real
        // heavy black heart character.
        assert_eq!(reactions[0].reaction, "\u{2764}");
    }

    #[test]
    fn parse_conversation_file_repairs_mojibake_content() {
        let export = tempdir().unwrap();
        let file = export.path().join("message_1.json");
        write_file(
            &file,
            r#"{
                "participants": [{"name": "Alice"}],
                "messages": [
                    {
                        "sender_name": "Alice",
                        "timestamp_ms": 1000,
                        "content": "hi â there"
                    }
                ],
                "title": "Alice",
                "is_still_participant": true,
                "thread_path": "inbox/conv_a"
            }"#,
        );

        let parsed = parse_conversation_file(&file).unwrap();

        // Repaired from the raw "hi â there" mojibake into
        // the real em dash character.
        assert_eq!(
            parsed.messages[0].content.as_deref(),
            Some("hi \u{2014} there")
        );
    }

    #[test]
    fn parse_conversation_file_treats_null_content_as_none() {
        let export = tempdir().unwrap();
        let file = export.path().join("message_1.json");
        write_file(
            &file,
            r#"{
                "participants": [{"name": "Alice"}],
                "messages": [
                    {"sender_name": "Alice", "timestamp_ms": 1000, "content": null}
                ],
                "title": "Alice",
                "is_still_participant": true,
                "thread_path": "inbox/conv_a"
            }"#,
        );

        let parsed = parse_conversation_file(&file).unwrap();

        assert_eq!(parsed.messages[0].content, None);
    }

    #[test]
    fn parse_conversation_file_captures_thread_path() {
        let export = tempdir().unwrap();
        let file = export.path().join("message_1.json");
        write_file(
            &file,
            r#"{
                "participants": [{"name": "Alice"}],
                "messages": [
                    {"sender_name": "Alice", "timestamp_ms": 1000, "content": "hi"}
                ],
                "title": "Alice",
                "is_still_participant": true,
                "thread_path": "inbox/alice_1234567890"
            }"#,
        );

        let parsed = parse_conversation_file(&file).unwrap();

        assert_eq!(parsed.thread_path, "inbox/alice_1234567890");
    }

    #[test]
    fn parse_conversation_file_treats_missing_is_still_participant_as_none() {
        let export = tempdir().unwrap();
        let file = export.path().join("message_1.json");
        write_file(
            &file,
            r#"{
                "participants": [{"name": "Alice"}],
                "messages": [
                    {"sender_name": "Alice", "timestamp_ms": 1000, "content": "hi"}
                ],
                "title": "Alice",
                "thread_path": "inbox/conv_a"
            }"#,
        );

        let parsed = parse_conversation_file(&file).unwrap();

        assert_eq!(parsed.is_still_participant, None);
    }

    #[test]
    fn parse_conversation_file_rejects_a_file_without_the_conversation_key() {
        // `(title, thread_path)` is what merges a conversation's files into
        // one thread; a file missing either would silently become its own
        // conversation, so it's refused here instead.
        let export = tempdir().unwrap();
        let no_title = export.path().join("message_1.json");
        write_file(
            &no_title,
            r#"{"participants": [], "thread_path": "inbox/conv_a"}"#,
        );
        let no_thread_path = export.path().join("message_2.json");
        write_file(&no_thread_path, r#"{"participants": [], "title": "Alice"}"#);

        for file in [&no_title, &no_thread_path] {
            let err = parse_conversation_file(file).unwrap_err();
            assert!(
                matches!(&err, Error::Parse { path, .. } if path == file),
                "expected a parse error naming {}, got: {err:?}",
                file.display()
            );
        }
    }

    #[test]
    fn parse_conversation_file_defaults_messages_to_empty_when_absent() {
        let export = tempdir().unwrap();
        let file = export.path().join("message_1.json");
        write_file(
            &file,
            r#"{
                "participants": [{"name": "Alice"}],
                "title": "Alice",
                "thread_path": "inbox/conv_a"
            }"#,
        );

        let parsed = parse_conversation_file(&file).unwrap();

        assert!(parsed.messages.is_empty());
    }

    #[test]
    fn parse_conversation_file_treats_missing_sender_name_as_none() {
        let export = tempdir().unwrap();
        let file = export.path().join("message_1.json");
        write_file(
            &file,
            r#"{
                "participants": [{"name": "Alice"}],
                "messages": [
                    {"timestamp_ms": 1000, "content": "hi"}
                ],
                "title": "Alice",
                "thread_path": "inbox/conv_a"
            }"#,
        );

        let parsed = parse_conversation_file(&file).unwrap();

        assert_eq!(parsed.messages[0].sender_name, None);
    }

    #[test]
    fn parse_conversation_file_errors_when_the_file_is_missing() {
        let export = tempdir().unwrap();
        let missing = export.path().join("message_1.json");

        let err = parse_conversation_file(&missing).unwrap_err();

        assert!(
            matches!(&err, Error::ReadFile { path, .. } if *path == missing),
            "a missing file should yield Error::ReadFile with its path, got: {err:?}"
        );
    }

    #[test]
    fn parse_conversation_file_errors_on_malformed_json() {
        let export = tempdir().unwrap();
        let file = export.path().join("message_1.json");
        write_file(&file, "{ this is not valid json");

        let err = parse_conversation_file(&file).unwrap_err();

        assert!(
            matches!(&err, Error::Parse { path, .. } if *path == file),
            "malformed JSON should yield Error::Parse with the file's path, got: {err:?}"
        );
        assert!(
            err.to_string().contains("message_1.json"),
            "the error message should name the offending file: {err}"
        );
    }

    #[test]
    fn parse_conversation_file_repairs_title_and_names_too() {
        let export = tempdir().unwrap();
        let file = export.path().join("message_1.json");
        write_file(
            &file,
            r#"{
                "participants": [{"name": "TomÃ¡s"}],
                "messages": [
                    {"sender_name": "TomÃ¡s", "timestamp_ms": 1000, "content": "hi"}
                ],
                "title": "TomÃ¡s",
                "is_still_participant": true,
                "thread_path": "inbox/conv_a"
            }"#,
        );

        let parsed = parse_conversation_file(&file).unwrap();

        // "TomÃ¡s" mojibake repairs to "Tomás" (Tomás).
        let repaired = "Tom\u{e1}s";
        assert_eq!(parsed.title, repaired);
        assert_eq!(parsed.participants[0].name, repaired);
        assert_eq!(parsed.messages[0].sender_name.as_deref(), Some(repaired));
    }

    fn raw_message(json: &str) -> RawMessage {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn message_type_is_text_when_no_attachments_are_present() {
        let message = raw_message(r#"{"timestamp_ms": 1000, "content": "hi"}"#);
        assert_eq!(message.message_type(), MessageType::Text);
    }

    #[test]
    fn message_type_is_photos_for_a_message_with_a_photos_list() {
        let message = raw_message(r#"{"timestamp_ms": 1000, "photos": [{"uri": "photos/1.jpg"}]}"#);
        assert_eq!(message.message_type(), MessageType::Photos);
    }

    #[test]
    fn message_type_is_videos_for_a_message_with_a_videos_list() {
        let message = raw_message(r#"{"timestamp_ms": 1000, "videos": [{"uri": "videos/1.mp4"}]}"#);
        assert_eq!(message.message_type(), MessageType::Videos);
    }

    #[test]
    fn message_type_is_audio_files_for_a_message_with_an_audio_files_list() {
        let message =
            raw_message(r#"{"timestamp_ms": 1000, "audio_files": [{"uri": "audio/1.aac"}]}"#);
        assert_eq!(message.message_type(), MessageType::AudioFiles);
    }

    #[test]
    fn message_type_is_gifs_for_a_message_with_a_gifs_list() {
        let message = raw_message(r#"{"timestamp_ms": 1000, "gifs": [{"uri": "gifs/1.gif"}]}"#);
        assert_eq!(message.message_type(), MessageType::Gifs);
    }

    #[test]
    fn message_type_prefers_the_attachment_list_over_caption_content() {
        // A photo sent with a text caption is a photos message; the caption
        // rides along in `content`.
        let message = raw_message(
            r#"{
                "timestamp_ms": 1000,
                "content": "look at this",
                "photos": [{"uri": "photos/1.jpg"}]
            }"#,
        );
        assert_eq!(message.message_type(), MessageType::Photos);
    }

    #[test]
    fn attachment_count_is_zero_for_a_text_message() {
        let message = raw_message(r#"{"timestamp_ms": 1000, "content": "hi"}"#);
        assert_eq!(message.attachment_count(), 0);
    }

    #[test]
    fn attachment_count_sums_across_all_attachment_lists() {
        let message = raw_message(
            r#"{
                "timestamp_ms": 1000,
                "photos": [{"uri": "photos/1.jpg"}, {"uri": "photos/2.jpg"}],
                "gifs": [{"uri": "gifs/1.gif"}]
            }"#,
        );
        assert_eq!(message.attachment_count(), 3);
    }

    #[test]
    fn attachments_capture_the_creation_timestamp_when_present() {
        let message = raw_message(
            r#"{
                "timestamp_ms": 1000,
                "photos": [{"uri": "photos/1.jpg", "creation_timestamp": 1712345678}]
            }"#,
        );
        assert_eq!(message.photos[0].creation_timestamp, Some(1712345678));
    }

    #[test]
    fn attachments_treat_a_missing_creation_timestamp_as_none() {
        let message = raw_message(r#"{"timestamp_ms": 1000, "gifs": [{"uri": "gifs/1.gif"}]}"#);
        assert_eq!(message.gifs[0].creation_timestamp, None);
    }

    #[test]
    fn message_type_tolerates_attachment_entries_without_a_uri() {
        // Only the list's presence matters for classification; a stripped
        // or unusual attachment entry shouldn't fail the parse.
        let message = raw_message(r#"{"timestamp_ms": 1000, "photos": [{}]}"#);
        assert_eq!(message.message_type(), MessageType::Photos);
    }

    #[test]
    fn repair_mojibake_leaves_plain_ascii_unchanged() {
        assert_eq!(repair_mojibake("hello world".to_string()), "hello world");
    }

    #[test]
    fn repair_mojibake_fixes_em_dash() {
        // U+2014 EM DASH encoded as UTF-8 (E2 80 94), mis-decoded as Latin-1.
        let mojibake = "hi \u{e2}\u{80}\u{94} there".to_string();
        assert_eq!(repair_mojibake(mojibake), "hi \u{2014} there");
    }

    #[test]
    fn repair_mojibake_fixes_emoji() {
        // U+2764 HEAVY BLACK HEART encoded as UTF-8 (E2 9D A4), mis-decoded
        // as Latin-1.
        let mojibake = "\u{e2}\u{9d}\u{a4}".to_string();
        assert_eq!(repair_mojibake(mojibake), "\u{2764}");
    }

    #[test]
    fn repair_mojibake_leaves_text_outside_latin1_range_unchanged() {
        // A genuinely mojibake-shaped string can never contain a char
        // outside 0..=0xFF, so this is the defensive fallback path.
        let text = "already fine \u{1f600}".to_string();
        assert_eq!(repair_mojibake(text.clone()), text);
    }

    #[test]
    fn repair_mojibake_peels_exactly_one_layer_of_corruption() {
        // Deliberately *not* idempotent: it undoes one application of the
        // export bug, so a doubly-corrupted string needs two passes and a
        // once-repaired string would be "repaired" again if fed back in.
        //
        // That's the same accepted risk as KNOWN_ISSUES #10 — "Ã©" is
        // indistinguishable from corrupted "é" by inspection, and this
        // function's contract is that its input is always corrupted exactly
        // once. Pinned here so a future refactor toward idempotence has to
        // be a deliberate choice rather than an accident.
        let once = corrupt("é");
        let twice = corrupt(&once);

        assert_eq!(repair_mojibake(twice), once);
        assert_eq!(repair_mojibake(once), "é");
    }

    /// Applies the corruption Messenger's exporter does: encode as UTF-8,
    /// then read each of those bytes back as a Latin-1 character.
    fn corrupt(text: &str) -> String {
        text.bytes().map(char::from).collect()
    }

    proptest::proptest! {
        /// The property [`repair_mojibake`] exists to satisfy: it is a left
        /// inverse of the corruption, for *any* text a message could hold —
        /// not just the em dash and emoji the examples above cover.
        #[test]
        fn repair_mojibake_undoes_the_corruption_for_any_text(text: String) {
            proptest::prop_assert_eq!(repair_mojibake(corrupt(&text)), text);
        }

        /// The first escape hatch: a char above U+00FF can't have come from
        /// reading a byte as Latin-1, so the text isn't corrupted and is
        /// left alone.
        #[test]
        fn repair_mojibake_leaves_any_text_with_a_non_latin1_char_alone(
            prefix: String,
            non_latin1 in proptest::char::range('\u{100}', char::MAX),
            suffix: String,
        ) {
            let text = format!("{prefix}{non_latin1}{suffix}");
            proptest::prop_assert_eq!(repair_mojibake(text.clone()), text);
        }

        /// The fast path: ASCII survives the corruption unchanged, so
        /// repairing it has nothing to do.
        #[test]
        fn repair_mojibake_leaves_any_ascii_text_alone(text in "[ -~]*") {
            proptest::prop_assert_eq!(repair_mojibake(text.clone()), text);
        }
    }
}
