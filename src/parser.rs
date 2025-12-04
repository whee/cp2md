// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2025 Brian Hetro <whee@smaertness.net>

//! JSON parsing for GitHub Copilot chat exports.
//!
//! This module handles deserialization of the JSON format produced by
//! GitHub Copilot's chat export feature. The format contains conversation
//! history including user messages, assistant responses, and tool invocations.
//!
//! # Format Overview
//!
//! A Copilot chat export contains:
//! - Metadata about the responder (username)
//! - A list of request/response pairs
//! - Each response can contain text, file references, code edits, and tool calls
//!
//! # Example
//!
//! ```
//! use cp2md::parser::parse_chat;
//!
//! let json = r#"{
//!     "responderUsername": "GitHub Copilot",
//!     "requests": [{
//!         "timestamp": 1733356800000,
//!         "message": { "text": "Hello" },
//!         "response": [{ "value": "Hi there!" }]
//!     }]
//! }"#;
//!
//! let chat = parse_chat(json).unwrap();
//! assert_eq!(chat.requests.len(), 1);
//! ```

use serde::Deserialize;
use snafu::prelude::*;

/// Error type for JSON parsing failures.
#[derive(Debug, Snafu)]
pub enum ParseError {
    /// Failed to parse JSON content.
    #[snafu(display("failed to parse JSON: {source}"))]
    Json {
        /// The underlying JSON parsing error.
        source: serde_json::Error,
    },
}

/// The root structure of a GitHub Copilot chat export.
///
/// This represents the entire conversation history exported from
/// a Copilot chat session.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatExport {
    /// The display name of the assistant (typically "GitHub Copilot").
    pub responder_username: String,

    /// The sequence of request/response exchanges in the conversation.
    pub requests: Vec<Request>,
}

/// A single request/response exchange in the conversation.
///
/// Each request represents one user message and the corresponding
/// assistant response, along with metadata like timestamps and model info.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    /// Unix timestamp in milliseconds when the request was made.
    pub timestamp: i64,

    /// The model identifier used for this response (e.g., "claude-sonnet-4").
    ///
    /// May be `None` for older exports or when the model info is unavailable.
    pub model_id: Option<String>,

    /// The user's message that initiated this request.
    pub message: Message,

    /// The assistant's response, which may contain multiple elements.
    pub response: Vec<ResponseElement>,
}

/// A user message in the conversation.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Message {
    /// The text content of the user's message.
    pub text: String,
}

/// An element within an assistant's response.
///
/// Responses are composed of multiple elements that can include plain text,
/// file references, code edits, and tool invocations. This enum represents
/// all the different element types that can appear in a response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponseElement {
    /// Plain text content from the assistant.
    Text(String),

    /// A reference to a file mentioned inline.
    InlineReference {
        /// Optional display name for the reference.
        name: Option<String>,
        /// The file path being referenced.
        path: String,
    },

    /// A URI indicating the source of a code block.
    CodeBlockUri {
        /// The file path associated with the code block.
        path: String,
    },

    /// A group of text edits applied to a file.
    TextEditGroup {
        /// The file path that was edited.
        path: String,
        /// The individual edit operations (replacement text).
        edits: Vec<String>,
    },

    /// A tool invocation performed by the assistant.
    ToolInvocation {
        /// A past-tense description of what the tool did (e.g., "Searched for files").
        past_tense: Option<String>,
    },

    /// An unrecognized or unsupported response element.
    ///
    /// This variant handles forward compatibility with new element types
    /// that may be added to the export format in the future.
    Other,
}

impl<'de> Deserialize<'de> for ResponseElement {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;

        if let Some(kind) = get_str(&value, &["kind"]) {
            return Ok(match kind {
                "inlineReference" => Self::InlineReference {
                    name: get_string(&value, &["name"]),
                    path: get_str(&value, &["inlineReference", "path"])
                        .unwrap_or_default()
                        .to_owned(),
                },
                "codeblockUri" => Self::CodeBlockUri {
                    path: get_str(&value, &["uri", "path"])
                        .unwrap_or_default()
                        .to_owned(),
                },
                "textEditGroup" => Self::TextEditGroup {
                    path: get_str(&value, &["uri", "path"])
                        .unwrap_or_default()
                        .to_owned(),
                    edits: extract_edits(&value),
                },
                "toolInvocationSerialized" => Self::ToolInvocation {
                    past_tense: get_string(&value, &["pastTenseMessage", "value"]),
                },
                _ => Self::Other,
            });
        }

        // No "kind" field: check if it's a text response
        if let Some(text) = get_str(&value, &["value"]) {
            return Ok(Self::Text(text.to_owned()));
        }

        Ok(Self::Other)
    }
}

/// Navigates a JSON path and returns the string value at the end.
///
/// # Arguments
///
/// * `value` - The root JSON value to navigate from
/// * `path` - A sequence of keys to follow through the JSON structure
fn get_str<'a>(value: &'a serde_json::Value, path: &[&str]) -> Option<&'a str> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_str()
}

/// Like [`get_str`] but returns an owned `String`.
fn get_string(value: &serde_json::Value, path: &[&str]) -> Option<String> {
    get_str(value, path).map(str::to_owned)
}

/// Extracts edit texts from the nested edits array structure.
///
/// The JSON format nests edits as: `edits: [[{text: "..."}], [{text: "..."}]]`
fn extract_edits(value: &serde_json::Value) -> Vec<String> {
    value
        .get("edits")
        .and_then(|e| e.as_array())
        .into_iter()
        .flatten()
        .filter_map(|group| group.as_array())
        .flatten()
        .filter_map(|edit| edit.get("text")?.as_str())
        .map(str::to_owned)
        .collect()
}

/// Parses a JSON string into a [`ChatExport`] structure.
///
/// This is the main entry point for parsing Copilot chat exports.
///
/// # Arguments
///
/// * `json_str` - The raw JSON content from a Copilot chat export file
///
/// # Errors
///
/// Returns an error if the JSON is malformed or doesn't match the expected
/// Copilot chat export schema.
///
/// # Example
///
/// ```
/// use cp2md::parser::parse_chat;
///
/// let json = r#"{
///     "responderUsername": "GitHub Copilot",
///     "requests": []
/// }"#;
///
/// let chat = parse_chat(json).unwrap();
/// assert_eq!(chat.responder_username, "GitHub Copilot");
/// ```
pub fn parse_chat(json_str: &str) -> Result<ChatExport, ParseError> {
    serde_json::from_str(json_str).context(JsonSnafu)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_chat_json(requests_json: &str) -> String {
        format!(
            r#"{{
                "responderUsername": "GitHub Copilot",
                "requests": [{requests_json}]
            }}"#
        )
    }

    fn request_json(message: &str, response_elements: &str) -> String {
        format!(
            r#"{{
                "timestamp": 1733356800000,
                "modelId": "claude-sonnet-4",
                "message": {{ "text": "{message}" }},
                "response": [{response_elements}]
            }}"#
        )
    }

    #[test]
    fn parses_minimal_chat() {
        let json = minimal_chat_json(&request_json("Hello", ""));
        let chat = parse_chat(&json).unwrap();

        assert_eq!(chat.responder_username, "GitHub Copilot");
        assert_eq!(chat.requests.len(), 1);
        assert_eq!(chat.requests[0].message.text, "Hello");
        assert_eq!(chat.requests[0].model_id, Some("claude-sonnet-4".into()));
    }

    #[test]
    fn parses_text_response() {
        let json = minimal_chat_json(&request_json("Hi", r#"{"value": "Hello there!"}"#));
        let chat = parse_chat(&json).unwrap();

        match &chat.requests[0].response[0] {
            ResponseElement::Text(text) => assert_eq!(text, "Hello there!"),
            other => panic!("Expected Text, got {other:?}"),
        }
    }

    #[test]
    fn parses_inline_reference() {
        let json = minimal_chat_json(&request_json(
            "Check file",
            r#"{
                "kind": "inlineReference",
                "name": "main.rs",
                "inlineReference": { "path": "/src/main.rs" }
            }"#,
        ));
        let chat = parse_chat(&json).unwrap();

        match &chat.requests[0].response[0] {
            ResponseElement::InlineReference { name, path } => {
                assert_eq!(name.as_deref(), Some("main.rs"));
                assert_eq!(path, "/src/main.rs");
            }
            other => panic!("Expected InlineReference, got {other:?}"),
        }
    }

    #[test]
    fn parses_inline_reference_without_name() {
        let json = minimal_chat_json(&request_json(
            "Check file",
            r#"{
                "kind": "inlineReference",
                "inlineReference": { "path": "/src/lib.rs" }
            }"#,
        ));
        let chat = parse_chat(&json).unwrap();

        match &chat.requests[0].response[0] {
            ResponseElement::InlineReference { name, path } => {
                assert!(name.is_none());
                assert_eq!(path, "/src/lib.rs");
            }
            other => panic!("Expected InlineReference, got {other:?}"),
        }
    }

    #[test]
    fn parses_codeblock_uri() {
        let json = minimal_chat_json(&request_json(
            "Show code",
            r#"{
                "kind": "codeblockUri",
                "uri": { "path": "/src/parser.rs" }
            }"#,
        ));
        let chat = parse_chat(&json).unwrap();

        match &chat.requests[0].response[0] {
            ResponseElement::CodeBlockUri { path } => {
                assert_eq!(path, "/src/parser.rs");
            }
            other => panic!("Expected CodeBlockUri, got {other:?}"),
        }
    }

    #[test]
    fn parses_text_edit_group() {
        let json = minimal_chat_json(&request_json(
            "Edit file",
            r#"{
                "kind": "textEditGroup",
                "uri": { "path": "/src/main.rs" },
                "edits": [
                    [{"text": "fn main() {}"}],
                    [{"text": "// comment"}]
                ]
            }"#,
        ));
        let chat = parse_chat(&json).unwrap();

        match &chat.requests[0].response[0] {
            ResponseElement::TextEditGroup { path, edits } => {
                assert_eq!(path, "/src/main.rs");
                assert_eq!(edits.len(), 2);
                assert_eq!(edits[0], "fn main() {}");
                assert_eq!(edits[1], "// comment");
            }
            other => panic!("Expected TextEditGroup, got {other:?}"),
        }
    }

    #[test]
    fn parses_tool_invocation() {
        let json = minimal_chat_json(&request_json(
            "Search",
            r#"{
                "kind": "toolInvocationSerialized",
                "pastTenseMessage": { "value": "Searched for text" }
            }"#,
        ));
        let chat = parse_chat(&json).unwrap();

        match &chat.requests[0].response[0] {
            ResponseElement::ToolInvocation { past_tense } => {
                assert_eq!(past_tense.as_deref(), Some("Searched for text"));
            }
            other => panic!("Expected ToolInvocation, got {other:?}"),
        }
    }

    #[test]
    fn parses_tool_invocation_without_message() {
        let json = minimal_chat_json(&request_json(
            "Search",
            r#"{"kind": "toolInvocationSerialized"}"#,
        ));
        let chat = parse_chat(&json).unwrap();

        match &chat.requests[0].response[0] {
            ResponseElement::ToolInvocation { past_tense } => {
                assert!(past_tense.is_none());
            }
            other => panic!("Expected ToolInvocation, got {other:?}"),
        }
    }

    #[test]
    fn parses_unknown_kind_as_other() {
        let json = minimal_chat_json(&request_json(
            "Something",
            r#"{"kind": "unknownKind", "data": "whatever"}"#,
        ));
        let chat = parse_chat(&json).unwrap();

        assert!(matches!(
            chat.requests[0].response[0],
            ResponseElement::Other
        ));
    }

    #[test]
    fn parses_object_without_kind_or_value_as_other() {
        let json = minimal_chat_json(&request_json("Something", r#"{"someField": "someValue"}"#));
        let chat = parse_chat(&json).unwrap();

        assert!(matches!(
            chat.requests[0].response[0],
            ResponseElement::Other
        ));
    }

    #[test]
    fn parses_multiple_response_elements() {
        let json = minimal_chat_json(&request_json(
            "Multi",
            r#"{"value": "First"}, {"value": "Second"}"#,
        ));
        let chat = parse_chat(&json).unwrap();

        assert_eq!(chat.requests[0].response.len(), 2);
        match (&chat.requests[0].response[0], &chat.requests[0].response[1]) {
            (ResponseElement::Text(a), ResponseElement::Text(b)) => {
                assert_eq!(a, "First");
                assert_eq!(b, "Second");
            }
            other => panic!("Expected two Text elements, got {other:?}"),
        }
    }

    #[test]
    fn parses_request_without_model_id() {
        let json = r#"{
            "responderUsername": "Copilot",
            "requests": [{
                "timestamp": 1733356800000,
                "message": { "text": "Hi" },
                "response": []
            }]
        }"#;
        let chat = parse_chat(json).unwrap();

        assert!(chat.requests[0].model_id.is_none());
    }

    #[test]
    fn returns_error_for_invalid_json() {
        let result = parse_chat("not valid json");
        assert!(result.is_err());
    }

    #[test]
    fn returns_error_for_missing_required_fields() {
        let result = parse_chat(r#"{"responderUsername": "Copilot"}"#);
        assert!(result.is_err());
    }
}
