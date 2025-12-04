// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2025 Brian Hetro <whee@smaertness.net>

//! Markdown rendering for parsed Copilot chat exports.
//!
//! This module transforms a [`ChatExport`] into a readable Markdown document.
//! The output format is designed to be clean and readable while preserving
//! the essential conversation structure.
//!
//! # Output Format
//!
//! The rendered Markdown includes:
//! - A top-level `# Copilot Chat` heading
//! - `## User` and `## Assistant` sections for each exchange
//! - Optional metadata (timestamps, model identifiers)
//! - Tool invocation summaries (when enabled)
//! - File edit summaries
//!
//! # Example
//!
//! ```
//! use cp2md::parser::{ChatExport, Request, Message, ResponseElement};
//! use cp2md::renderer::{render_chat, RenderOptions};
//!
//! let chat = ChatExport {
//!     responder_username: "GitHub Copilot".into(),
//!     requests: vec![Request {
//!         timestamp: 1733356800000,
//!         model_id: Some("claude-sonnet-4".into()),
//!         message: Message { text: "Hello!".into() },
//!         response: vec![ResponseElement::Text("Hi there!".into())],
//!     }],
//! };
//!
//! let opts = RenderOptions::default();
//! let markdown = render_chat(&chat, &opts);
//!
//! assert!(markdown.contains("# Copilot Chat"));
//! assert!(markdown.contains("Hello!"));
//! assert!(markdown.contains("Hi there!"));
//! ```

use crate::parser::{ChatExport, Request, ResponseElement};
use chrono::DateTime;
use std::fmt::Write;
use std::path::Path;

/// Configuration options for Markdown rendering.
///
/// Controls which optional elements are included in the rendered output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderOptions {
    /// Whether to include tool invocation summaries in the output.
    ///
    /// When enabled, tool calls (file reads, searches, etc.) are shown
    /// as blockquoted lines with a 🔧 prefix.
    pub show_tools: bool,

    /// Whether to include timestamps in the conversation metadata.
    ///
    /// When enabled, each user message shows when it was sent.
    pub show_timestamps: bool,

    /// Whether to include model identifiers in the conversation metadata.
    ///
    /// When disabled, model IDs like "claude-sonnet-4" are hidden.
    pub show_model: bool,

    /// Number of heading levels to shift (0-5).
    ///
    /// A value of 0 produces H1/H2 headings (default).
    /// A value of 1 produces H2/H3 headings, useful for embedding.
    pub heading_offset: u8,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            show_tools: false,
            show_timestamps: false,
            show_model: true,
            heading_offset: 0,
        }
    }
}

/// Returns a markdown heading prefix with the given level and offset.
///
/// The heading level is clamped to a maximum of 6 (H6).
fn heading(level: u8, offset: u8) -> String {
    let actual = (level + offset).min(6);
    "#".repeat(actual as usize)
}

/// Renders a parsed chat export as Markdown.
///
/// This is the main entry point for rendering. It processes all requests
/// in the chat and produces a complete Markdown document.
///
/// # Arguments
///
/// * `chat` - The parsed chat export to render
/// * `opts` - Configuration options controlling the output format
///
/// # Returns
///
/// A `String` containing the complete Markdown document.
#[must_use]
pub fn render_chat(chat: &ChatExport, opts: &RenderOptions) -> String {
    let mut out = String::new();
    writeln!(out, "{} Copilot Chat\n", heading(1, opts.heading_offset)).unwrap();

    for request in &chat.requests {
        render_request(&mut out, request, opts);
    }

    out
}

fn render_request(out: &mut String, req: &Request, opts: &RenderOptions) {
    let timestamp = DateTime::from_timestamp_millis(req.timestamp)
        .map(|dt| dt.format("%Y-%m-%d %H:%M UTC").to_string());

    let model_id = if opts.show_model {
        req.model_id.as_deref()
    } else {
        None
    };

    let metadata = match (opts.show_timestamps, &timestamp, model_id) {
        (true, Some(ts), Some(model)) => format!("*{ts} · {model}*"),
        (true, Some(ts), None) => format!("*{ts}*"),
        (false, _, Some(model)) => format!("*{model}*"),
        _ => String::new(),
    };

    writeln!(out, "{} User\n", heading(2, opts.heading_offset)).unwrap();
    if !metadata.is_empty() {
        writeln!(out, "{metadata}\n").unwrap();
    }
    writeln!(out, "{}\n", escape_xml_tags(&req.message.text)).unwrap();

    if opts.show_tools {
        render_tool_invocations(out, &req.response);
    }

    writeln!(out, "{} Assistant\n", heading(2, opts.heading_offset)).unwrap();
    render_response(out, &req.response);
}

fn render_tool_invocations(out: &mut String, elements: &[ResponseElement]) {
    let mut any_rendered = false;
    for elem in elements {
        if let ResponseElement::ToolInvocation {
            past_tense: Some(msg),
        } = elem
        {
            writeln!(out, "> 🔧 {}", escape_xml_tags(msg)).unwrap();
            any_rendered = true;
        }
    }
    if any_rendered {
        out.push('\n');
    }
}

fn render_response(out: &mut String, elements: &[ResponseElement]) {
    for elem in elements {
        match elem {
            ResponseElement::Text(text) => {
                let trimmed = text.trim();
                if trimmed.is_empty() || is_only_code_fences(trimmed) {
                    continue;
                }
                out.push_str(&escape_xml_tags(text));
            }
            ResponseElement::InlineReference { name, path } => {
                let display = name
                    .as_deref()
                    .or_else(|| Path::new(path).file_name()?.to_str())
                    .unwrap_or(path);
                write!(out, "`{}`", escape_for_inline_code(display)).unwrap();
            }
            ResponseElement::TextEditGroup { path, edits } if !edits.is_empty() => {
                let filename = Path::new(path)
                    .file_name()
                    .and_then(|f| f.to_str())
                    .unwrap_or(path);
                let line_count: usize = edits.iter().map(|e| e.lines().count()).sum();
                writeln!(
                    out,
                    "\n*Modified `{}` ({line_count} lines)*\n",
                    escape_for_inline_code(filename)
                )
                .unwrap();
            }
            _ => {}
        }
    }
    out.push_str("\n\n");
}

/// Returns `true` if the string contains only code fence markers and whitespace.
///
/// These are streaming artifacts from the Copilot response that shouldn't
/// appear in rendered output.
fn is_only_code_fences(s: &str) -> bool {
    s.lines().all(|line| {
        let trimmed = line.trim();
        trimmed.is_empty() || trimmed == "```"
    })
}

/// Escapes backticks in a string for use inside inline code spans.
///
/// Replaces backticks with single quotes to avoid breaking the inline code
/// syntax when displaying filenames that contain backticks.
fn escape_for_inline_code(s: &str) -> String {
    s.replace('`', "'")
}

/// Escapes XML/HTML-like tags so they render literally in Markdown.
///
/// Uses HTML entities (`&lt;` `&gt;`) which are more reliably rendered across
/// markdown viewers. Only escapes `<` when followed by a letter, `/`, or `!`
/// to avoid false positives on mathematical comparisons like `x < 5`.
fn escape_xml_tags(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 2);
    let mut chars = s.chars().peekable();
    let mut in_tag = false;

    while let Some(c) = chars.next() {
        if c == '<' {
            let is_tag_start = chars
                .peek()
                .is_some_and(|&next| next.is_ascii_alphabetic() || next == '/' || next == '!');

            if is_tag_start {
                result.push_str("&lt;");
                in_tag = true;
            } else {
                result.push(c);
            }
        } else if c == '>' && in_tag {
            result.push_str("&gt;");
            in_tag = false;
        } else {
            result.push(c);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{ChatExport, Message, Request, ResponseElement};

    fn make_chat(requests: Vec<Request>) -> ChatExport {
        ChatExport {
            responder_username: "GitHub Copilot".into(),
            requests,
        }
    }

    fn make_request(message: &str, response: Vec<ResponseElement>) -> Request {
        Request {
            timestamp: 1_733_356_800_000, // 2024-12-05 00:00:00 UTC
            model_id: Some("claude-sonnet-4".into()),
            message: Message {
                text: message.into(),
            },
            response,
        }
    }

    fn default_opts() -> RenderOptions {
        RenderOptions::default()
    }

    #[test]
    fn renders_basic_chat_structure() {
        let chat = make_chat(vec![make_request("Hello", vec![])]);
        let output = render_chat(&chat, &default_opts());

        assert!(output.starts_with("# Copilot Chat\n\n"));
        assert!(output.contains("## User\n"));
        assert!(output.contains("## Assistant\n"));
    }

    #[test]
    fn renders_user_message() {
        let chat = make_chat(vec![make_request("What is Rust?", vec![])]);
        let output = render_chat(&chat, &default_opts());

        assert!(output.contains("What is Rust?"));
    }

    #[test]
    fn renders_text_response() {
        let chat = make_chat(vec![make_request(
            "Hi",
            vec![ResponseElement::Text("Hello there!".into())],
        )]);
        let output = render_chat(&chat, &default_opts());

        assert!(output.contains("Hello there!"));
    }

    #[test]
    fn renders_multiple_text_responses_concatenated() {
        let chat = make_chat(vec![make_request(
            "Hi",
            vec![
                ResponseElement::Text("First ".into()),
                ResponseElement::Text("Second".into()),
            ],
        )]);
        let output = render_chat(&chat, &default_opts());

        assert!(output.contains("First Second"));
    }

    #[test]
    fn renders_model_id_when_no_timestamps() {
        let chat = make_chat(vec![make_request("Hi", vec![])]);
        let opts = RenderOptions {
            show_tools: false,
            show_timestamps: false,
            ..Default::default()
        };
        let output = render_chat(&chat, &opts);

        assert!(output.contains("*claude-sonnet-4*"));
    }

    #[test]
    fn renders_timestamp_and_model_when_enabled() {
        let chat = make_chat(vec![make_request("Hi", vec![])]);
        let opts = RenderOptions {
            show_tools: false,
            show_timestamps: true,
            ..Default::default()
        };
        let output = render_chat(&chat, &opts);

        assert!(output.contains("2024-12-05 00:00 UTC"));
        assert!(output.contains("claude-sonnet-4"));
    }

    #[test]
    fn renders_inline_reference_with_name() {
        let chat = make_chat(vec![make_request(
            "Check",
            vec![ResponseElement::InlineReference {
                name: Some("main.rs".into()),
                path: "/src/main.rs".into(),
            }],
        )]);
        let output = render_chat(&chat, &default_opts());

        assert!(output.contains("`main.rs`"));
    }

    #[test]
    fn renders_inline_reference_extracts_filename_from_path() {
        let chat = make_chat(vec![make_request(
            "Check",
            vec![ResponseElement::InlineReference {
                name: None,
                path: "/some/deep/path/to/file.rs".into(),
            }],
        )]);
        let output = render_chat(&chat, &default_opts());

        assert!(output.contains("`file.rs`"));
    }

    #[test]
    fn renders_text_edit_group_summary() {
        let chat = make_chat(vec![make_request(
            "Edit",
            vec![ResponseElement::TextEditGroup {
                path: "/src/main.rs".into(),
                edits: vec!["fn main() {\n    println!(\"hi\");\n}".into()],
            }],
        )]);
        let output = render_chat(&chat, &default_opts());

        assert!(output.contains("*Modified `main.rs`"));
        assert!(output.contains("3 lines"));
    }

    #[test]
    fn skips_empty_text_edit_group() {
        let chat = make_chat(vec![make_request(
            "Edit",
            vec![ResponseElement::TextEditGroup {
                path: "/src/main.rs".into(),
                edits: vec![],
            }],
        )]);
        let output = render_chat(&chat, &default_opts());

        assert!(!output.contains("Modified"));
    }

    #[test]
    fn hides_tool_invocations_by_default() {
        let chat = make_chat(vec![make_request(
            "Search",
            vec![ResponseElement::ToolInvocation {
                past_tense: Some("Searched for files".into()),
            }],
        )]);
        let opts = RenderOptions {
            show_tools: false,
            show_timestamps: false,
            ..Default::default()
        };
        let output = render_chat(&chat, &opts);

        assert!(!output.contains("Searched for files"));
        assert!(!output.contains("🔧"));
    }

    #[test]
    fn shows_tool_invocations_when_enabled() {
        let chat = make_chat(vec![make_request(
            "Search",
            vec![ResponseElement::ToolInvocation {
                past_tense: Some("Searched for files".into()),
            }],
        )]);
        let opts = RenderOptions {
            show_tools: true,
            show_timestamps: false,
            ..Default::default()
        };
        let output = render_chat(&chat, &opts);

        assert!(output.contains("> 🔧 Searched for files"));
    }

    #[test]
    fn skips_tool_invocation_without_message() {
        let chat = make_chat(vec![make_request(
            "Search",
            vec![ResponseElement::ToolInvocation { past_tense: None }],
        )]);
        let opts = RenderOptions {
            show_tools: true,
            show_timestamps: false,
            ..Default::default()
        };
        let output = render_chat(&chat, &opts);

        assert!(!output.contains("🔧"));
    }

    #[test]
    fn skips_codeblock_uri_and_other() {
        let chat = make_chat(vec![make_request(
            "Mixed",
            vec![
                ResponseElement::Text("visible".into()),
                ResponseElement::CodeBlockUri {
                    path: "/src/main.rs".into(),
                },
                ResponseElement::Other,
            ],
        )]);
        let output = render_chat(&chat, &default_opts());

        assert!(output.contains("visible"));
        // CodeBlockUri and Other should not produce visible output
        assert!(!output.contains("/src/main.rs"));
    }

    #[test]
    fn skips_empty_text() {
        let chat = make_chat(vec![make_request(
            "Hi",
            vec![
                ResponseElement::Text(String::new()),
                ResponseElement::Text("   ".into()),
                ResponseElement::Text("visible".into()),
            ],
        )]);
        let output = render_chat(&chat, &default_opts());

        let assistant_section = output.split("## Assistant").nth(1).unwrap();
        // Should only contain "visible", not empty strings
        assert!(assistant_section.contains("visible"));
    }

    #[test]
    fn skips_code_fence_only_text() {
        let chat = make_chat(vec![make_request(
            "Hi",
            vec![
                ResponseElement::Text("```\n```".into()),
                ResponseElement::Text("```".into()),
                ResponseElement::Text("real content".into()),
            ],
        )]);
        let output = render_chat(&chat, &default_opts());

        assert!(output.contains("real content"));
    }

    // Tests for escape_xml_tags helper
    #[test]
    fn escapes_xml_tags() {
        assert_eq!(escape_xml_tags("<div>"), "&lt;div&gt;");
        assert_eq!(escape_xml_tags("</div>"), "&lt;/div&gt;");
        assert_eq!(escape_xml_tags("<!DOCTYPE>"), "&lt;!DOCTYPE&gt;");
    }

    #[test]
    fn preserves_non_tag_less_than() {
        assert_eq!(escape_xml_tags("a < b"), "a < b");
        assert_eq!(escape_xml_tags("x<5"), "x<5");
        assert_eq!(escape_xml_tags("3 < 4 < 5"), "3 < 4 < 5");
    }

    #[test]
    fn escapes_mixed_content() {
        assert_eq!(
            escape_xml_tags("Use <code> for x < 5"),
            "Use &lt;code&gt; for x < 5"
        );
    }

    #[test]
    fn handles_empty_string() {
        assert_eq!(escape_xml_tags(""), "");
    }

    #[test]
    fn handles_lone_less_than_at_end() {
        assert_eq!(escape_xml_tags("value<"), "value<");
    }

    // Tests for is_only_code_fences helper
    #[test]
    fn detects_code_fence_only() {
        assert!(is_only_code_fences("```"));
        assert!(is_only_code_fences("```\n```"));
        assert!(is_only_code_fences("  ```  "));
        assert!(is_only_code_fences("\n```\n\n```\n"));
    }

    #[test]
    fn detects_non_code_fence_content() {
        assert!(!is_only_code_fences("```rust\nfn main() {}\n```"));
        assert!(!is_only_code_fences("some text"));
        assert!(!is_only_code_fences("``` more"));
    }

    #[test]
    fn renders_multiple_requests() {
        let chat = make_chat(vec![
            make_request(
                "First question",
                vec![ResponseElement::Text("First answer".into())],
            ),
            make_request(
                "Second question",
                vec![ResponseElement::Text("Second answer".into())],
            ),
        ]);
        let output = render_chat(&chat, &default_opts());

        assert!(output.contains("First question"));
        assert!(output.contains("First answer"));
        assert!(output.contains("Second question"));
        assert!(output.contains("Second answer"));

        // Should have two User sections
        assert_eq!(output.matches("## User").count(), 2);
        assert_eq!(output.matches("## Assistant").count(), 2);
    }

    #[test]
    fn escapes_xml_in_user_message() {
        let chat = make_chat(vec![make_request(
            "<instructions>do stuff</instructions>",
            vec![],
        )]);
        let output = render_chat(&chat, &default_opts());

        assert!(output.contains("&lt;instructions&gt;"));
        assert!(output.contains("&lt;/instructions&gt;"));
    }

    #[test]
    fn escapes_xml_in_response_text() {
        let chat = make_chat(vec![make_request(
            "Hi",
            vec![ResponseElement::Text("<result>success</result>".into())],
        )]);
        let output = render_chat(&chat, &default_opts());

        assert!(output.contains("&lt;result&gt;"));
    }

    #[test]
    fn escapes_xml_in_tool_message() {
        let chat = make_chat(vec![make_request(
            "Search",
            vec![ResponseElement::ToolInvocation {
                past_tense: Some("Found <file> tag".into()),
            }],
        )]);
        let opts = RenderOptions {
            show_tools: true,
            show_timestamps: false,
            ..Default::default()
        };
        let output = render_chat(&chat, &opts);

        assert!(output.contains("&lt;file&gt;"));
    }

    #[test]
    fn escapes_backticks_in_inline_reference() {
        let chat = make_chat(vec![make_request(
            "Check",
            vec![ResponseElement::InlineReference {
                name: Some("`config`.json".into()),
                path: "/src/`config`.json".into(),
            }],
        )]);
        let output = render_chat(&chat, &default_opts());

        assert!(output.contains("`'config'.json`"));
        assert!(!output.contains("``"));
    }

    #[test]
    fn escapes_backticks_in_file_edit_summary() {
        let chat = make_chat(vec![make_request(
            "Edit",
            vec![ResponseElement::TextEditGroup {
                path: "/src/`test`.rs".into(),
                edits: vec!["fn main() {}".into()],
            }],
        )]);
        let output = render_chat(&chat, &default_opts());

        assert!(output.contains("*Modified `'test'.rs`"));
    }

    #[test]
    fn adds_blank_line_before_subsequent_user_sections() {
        let chat = make_chat(vec![
            make_request(
                "First question",
                vec![ResponseElement::Text("First answer".into())],
            ),
            make_request(
                "Second question",
                vec![ResponseElement::Text("Second answer".into())],
            ),
        ]);
        let output = render_chat(&chat, &default_opts());

        // Should have a blank line before the second "## User"
        // The pattern should be: response text, newline, newline, "## User"
        assert!(output.contains("First answer\n\n## User"));
    }
}
