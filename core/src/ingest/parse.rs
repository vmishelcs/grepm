use std::fs;
use std::io;
use std::mem;
use std::path::Path;

use serde::Deserialize;

pub fn parse_conversation_file(path: impl AsRef<Path>) -> io::Result<RawConversationFile> {
    let text = fs::read_to_string(path)?;
    let mut file: RawConversationFile = serde_json::from_str(&text)?;
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
    pub title: Option<String>,
    pub is_still_participant: Option<bool>,
    pub thread_path: Option<String>,
}

impl RawConversationFile {
    fn repair_mojibake(&mut self) {
        if let Some(title) = self.title.take() {
            self.title = Some(repair_mojibake(title));
        }

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

    fn write_file(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

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

        assert_eq!(parsed.title.as_deref(), Some("Alice"));
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

        assert_eq!(
            parsed.thread_path.as_deref(),
            Some("inbox/alice_1234567890")
        );
    }

    #[test]
    fn parse_conversation_file_treats_missing_optional_fields_as_none() {
        let export = tempdir().unwrap();
        let file = export.path().join("message_1.json");
        write_file(
            &file,
            r#"{
                "participants": [{"name": "Alice"}],
                "messages": [
                    {"sender_name": "Alice", "timestamp_ms": 1000, "content": "hi"}
                ]
            }"#,
        );

        let parsed = parse_conversation_file(&file).unwrap();

        assert_eq!(parsed.title, None);
        assert_eq!(parsed.is_still_participant, None);
        assert_eq!(parsed.thread_path, None);
    }

    #[test]
    fn parse_conversation_file_defaults_messages_to_empty_when_absent() {
        let export = tempdir().unwrap();
        let file = export.path().join("message_1.json");
        write_file(&file, r#"{"participants": [{"name": "Alice"}]}"#);

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
                ]
            }"#,
        );

        let parsed = parse_conversation_file(&file).unwrap();

        assert_eq!(parsed.messages[0].sender_name, None);
    }

    #[test]
    fn parse_conversation_file_errors_when_the_file_is_missing() {
        let export = tempdir().unwrap();
        let missing = export.path().join("message_1.json");

        let result = parse_conversation_file(&missing);

        assert!(result.is_err());
    }

    #[test]
    fn parse_conversation_file_errors_on_malformed_json() {
        let export = tempdir().unwrap();
        let file = export.path().join("message_1.json");
        write_file(&file, "{ this is not valid json");

        let result = parse_conversation_file(&file);

        assert!(result.is_err());
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
        assert_eq!(parsed.title.as_deref(), Some(repaired));
        assert_eq!(parsed.participants[0].name, repaired);
        assert_eq!(parsed.messages[0].sender_name.as_deref(), Some(repaired));
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
}
