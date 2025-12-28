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
//! use chrono::{TimeZone, Utc};
//! use cp2md::parser::{ChatExport, Request, Message, ResponseElement};
//! use cp2md::renderer::{render_chat, RenderOptions};
//!
//! let chat = ChatExport {
//!     responder_username: "GitHub Copilot".into(),
//!     requests: vec![Request {
//!         timestamp: Some(Utc.timestamp_millis_opt(1_733_356_800_000).single().unwrap()),
//!         model_id: Some("claude-sonnet-4".into()),
//!         agent_name: None,
//!         context: vec![],
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

use crate::parser::{ChatExport, ContextItem, Request, ResponseElement};
use chrono::{DateTime, Local, Utc};

/// Timezone rendering selection for timestamps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimestampZone {
    /// Render timestamps in UTC (default).
    Utc,
    /// Render timestamps in the system local timezone.
    Local,
    /// Render both local and UTC timestamps.
    Both,
}

/// Simple on/off visibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    /// Do not render the element.
    Hidden,
    /// Render the element.
    Shown,
}

/// How much detail to include for file edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditDisplay {
    /// Only show a summary line.
    SummaryOnly,
    /// Include code blocks for the edits.
    WithCode,
}

/// Timestamp rendering selection, including hiding entirely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimestampDisplay {
    /// Do not render timestamps.
    Hidden,
    /// Render timestamps in the selected zone.
    Zoned(TimestampZone),
}
use std::fmt::{self, Display, Formatter, Write};
use std::path::Path;

/// Configuration options for Markdown rendering.
///
/// Controls which optional elements are included in the rendered output.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct RenderOptions {
    /// Whether to include tool invocation summaries in the output.
    ///
    /// When enabled, tool calls (file reads, searches, etc.) are shown
    /// as blockquoted lines with a 🔧 prefix.
    pub tools: Visibility,

    /// Whether to include timestamps in the conversation metadata.
    pub timestamps: TimestampDisplay,

    /// Whether to include model identifiers in the conversation metadata.
    ///
    /// When disabled, model IDs like "claude-sonnet-4" are hidden.
    pub model: Visibility,

    /// Whether to include the VS Code agent name in the conversation metadata.
    ///
    /// When enabled, shows the agent used (e.g., "@agent", "@documentation-reviewer").
    pub agent: Visibility,

    /// Whether to include attached context in the output.
    ///
    /// When enabled, shows files, selections, and instruction files that were
    /// attached to each request in a collapsible details block.
    pub context: Visibility,

    /// Whether to include the actual code content of file edits.
    ///
    /// When enabled, `TextEditGroup` elements show the full code in a fenced
    /// block after the summary line. Default is off (summary only).
    pub edits: EditDisplay,

    /// Number of heading levels to shift (0-5).
    ///
    /// A value of 0 produces H1/H2 headings (default).
    /// A value of 1 produces H2/H3 headings, useful for embedding.
    pub heading_offset: u8,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            tools: Visibility::Hidden,
            timestamps: TimestampDisplay::Hidden,
            model: Visibility::Shown,
            agent: Visibility::Shown,
            context: Visibility::Shown,
            edits: EditDisplay::SummaryOnly,
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

#[derive(Debug, Default)]
struct CollectedResponse {
    tools: Vec<String>,
    chunks: Vec<RenderChunk>,
}

#[derive(Debug)]
enum RenderChunk {
    Text(String),
    InlineReference(String),
    TextEditSummary {
        filename: String,
        line_count: usize,
        code: Option<RenderedCodeBlock>,
    },
}

impl Display for RenderChunk {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text(text) => f.write_str(text),
            Self::InlineReference(display) => write!(f, "`{display}`"),
            Self::TextEditSummary {
                filename,
                line_count,
                code,
            } => {
                writeln!(f, "\n*Modified `{filename}` ({line_count} lines)*\n")?;
                if let Some(block) = code {
                    write!(f, "{block}")?;
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug)]
struct RenderedCodeBlock {
    lang: &'static str,
    edits: Vec<String>,
}

impl Display for RenderedCodeBlock {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        writeln!(f, "```{}", self.lang)?;
        for (i, edit) in self.edits.iter().enumerate() {
            if i > 0 {
                f.write_str("\n// ...\n\n")?;
            }
            f.write_str(edit)?;
        }
        writeln!(f, "\n```")
    }
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
    let timestamp = match opts.timestamps {
        TimestampDisplay::Hidden => None,
        TimestampDisplay::Zoned(zone) => {
            req.timestamp.as_ref().map(|dt| format_timestamp(dt, zone))
        }
    };

    let model_id = if opts.model == Visibility::Shown {
        req.model_id.as_deref()
    } else {
        None
    };

    let agent_name = if opts.agent == Visibility::Shown {
        req.agent_name.as_deref()
    } else {
        None
    };

    // Build metadata parts
    let mut parts: Vec<String> = Vec::new();
    if let Some(ts) = &timestamp {
        parts.push(ts.clone());
    }
    if let Some(model) = model_id {
        parts.push(model.to_string());
    }
    if let Some(agent) = agent_name {
        parts.push(format!("@{agent}"));
    }

    let metadata = if parts.is_empty() {
        String::new()
    } else {
        format!("*{}*", parts.join(" · "))
    };

    writeln!(out, "{} User\n", heading(2, opts.heading_offset)).unwrap();
    if !metadata.is_empty() {
        writeln!(out, "{metadata}\n").unwrap();
    }

    // Render context if enabled and non-empty
    if opts.context == Visibility::Shown && !req.context.is_empty() {
        render_context(out, &req.context);
    }

    // Shift headings in user content to prevent them from competing with
    // our document structure (H1 title, H2 sections). Shift by 2 + offset
    // so user H1 becomes H3+ (below our H2 section headers).
    let heading_shift = 2 + opts.heading_offset;
    let shifted = shift_headings(&req.message.text, heading_shift);
    writeln!(out, "{}\n", escape_xml_tags(&shifted)).unwrap();

    let response = collect_response(&req.response, opts, heading_shift);

    if !response.tools.is_empty() {
        render_tools(out, &response.tools);
    }

    writeln!(out, "{} Assistant\n", heading(2, opts.heading_offset)).unwrap();
    render_chunks(out, &response.chunks);
}

fn render_context(out: &mut String, context: &[ContextItem]) {
    writeln!(out, "<details>").unwrap();
    writeln!(out, "<summary>📎 Context</summary>\n").unwrap();

    for item in context {
        writeln!(out, "- {item}").unwrap();
    }

    writeln!(out, "\n</details>\n").unwrap();
}

impl Display for ContextItem {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::File { name, path } => {
                write!(f, "{} (file)", format_path_display(name, path))
            }
            Self::Selection {
                name,
                path,
                start_line,
                end_line,
            } => {
                let range = if start_line == end_line {
                    format!(":{start_line}")
                } else {
                    format!(":{start_line}-{end_line}")
                };
                write!(f, "{}{range} (selection)", format_path_display(name, path))
            }
            Self::Folder { name, path } => {
                write!(f, "{} (folder)", format_path_display(name, path))
            }
            Self::Instructions { name } => {
                write!(f, "`{name}` (instructions)")
            }
        }
    }
}

/// Formats a path for display with smart truncation.
///
/// For paths longer than 30 characters, shows just the filename with a
/// Markdown link containing the full path as a title. For shorter paths,
/// shows the path directly.
fn format_path_display(name: &str, path: &str) -> String {
    const MAX_INLINE_PATH_LEN: usize = 30;

    if path.is_empty() || path.len() <= MAX_INLINE_PATH_LEN {
        // Short path or no path: just show the name in backticks
        format!("`{name}`")
    } else {
        // Long path: show name with full path in link title
        format!("[`{name}`]({path} \"{path}\")")
    }
}

fn collect_response(
    elements: &[ResponseElement],
    opts: &RenderOptions,
    heading_shift: u8,
) -> CollectedResponse {
    let mut collected = CollectedResponse::default();

    for elem in elements {
        match elem {
            ResponseElement::Text(text) => {
                let trimmed = text.trim();
                if trimmed.is_empty() || is_only_code_fences(trimmed) {
                    continue;
                }

                let shifted = shift_headings(text, heading_shift);
                collected
                    .chunks
                    .push(RenderChunk::Text(escape_xml_tags(&shifted)));
            }
            ResponseElement::InlineReference { name, path } => {
                let display = name
                    .as_deref()
                    .or_else(|| Path::new(path).file_name()?.to_str())
                    .unwrap_or(path);
                collected
                    .chunks
                    .push(RenderChunk::InlineReference(escape_for_inline_code(
                        display,
                    )));
            }
            ResponseElement::TextEditGroup { path, edits } if !edits.is_empty() => {
                let filename = Path::new(path)
                    .file_name()
                    .and_then(|f| f.to_str())
                    .unwrap_or(path);
                let line_count: usize = edits.iter().map(|e| e.lines().count()).sum();
                let code = if opts.edits == EditDisplay::WithCode {
                    Some(RenderedCodeBlock {
                        lang: extension_to_language(path),
                        edits: edits.clone(),
                    })
                } else {
                    None
                };

                collected.chunks.push(RenderChunk::TextEditSummary {
                    filename: escape_for_inline_code(filename),
                    line_count,
                    code,
                });
            }
            ResponseElement::ToolInvocation {
                past_tense: Some(msg),
            } if opts.tools == Visibility::Shown => {
                collected.tools.push(escape_xml_tags(msg));
            }
            _ => {}
        }
    }

    collected
}

fn render_tools(out: &mut String, tools: &[String]) {
    for msg in tools {
        writeln!(out, "> 🔧 {msg}").unwrap();
    }

    if !tools.is_empty() {
        out.push('\n');
    }
}

fn render_chunks(out: &mut String, chunks: &[RenderChunk]) {
    for chunk in chunks {
        write!(out, "{chunk}").unwrap();
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

/// Maps file extensions to markdown language identifiers for syntax highlighting.
fn extension_to_language(path: &str) -> &'static str {
    match Path::new(path).extension().and_then(|e| e.to_str()) {
        // Systems languages
        Some("rs") => "rust",
        Some("c" | "h") => "c",
        Some("cpp" | "cc" | "cxx" | "hpp") => "cpp",
        Some("go") => "go",
        Some("zig") => "zig",
        // JVM languages
        Some("java") => "java",
        Some("kt" | "kts") => "kotlin",
        Some("scala" | "sc") => "scala",
        Some("clj" | "cljs" | "cljc") => "clojure",
        // .NET languages
        Some("cs") => "csharp",
        Some("fs" | "fsx") => "fsharp",
        // Dynamic/scripting languages
        Some("py") => "python",
        Some("rb") => "ruby",
        Some("php") => "php",
        Some("pl" | "pm") => "perl",
        Some("lua") => "lua",
        Some("r" | "R") => "r",
        // Functional languages
        Some("hs" | "lhs") => "haskell",
        Some("ml" | "mli") => "ocaml",
        Some("ex" | "exs") => "elixir",
        Some("erl" | "hrl") => "erlang",
        Some("nim") => "nim",
        // Web frontend
        Some("js") => "javascript",
        Some("ts") => "typescript",
        Some("jsx") => "jsx",
        Some("tsx") => "tsx",
        Some("vue") => "vue",
        Some("svelte") => "svelte",
        // Apple/mobile
        Some("swift") => "swift",
        Some("m" | "mm") => "objectivec",
        Some("dart") => "dart",
        // Shell
        Some("sh") => "shell",
        Some("bash") => "bash",
        Some("zsh") => "zsh",
        Some("fish") => "fish",
        Some("ps1") => "powershell",
        // Data/config formats
        Some("json") => "json",
        Some("yaml" | "yml") => "yaml",
        Some("toml") => "toml",
        Some("xml") => "xml",
        Some("proto") => "protobuf",
        Some("tf" | "hcl") => "hcl",
        // Markup/docs
        Some("md" | "markdown") => "markdown",
        Some("html" | "htm") => "html",
        // Stylesheets
        Some("css") => "css",
        Some("scss") => "scss",
        Some("less") => "less",
        // Query languages
        Some("sql") => "sql",
        Some("graphql" | "gql") => "graphql",
        // Other
        Some("diff" | "patch") => "diff",
        Some("dockerfile") => "dockerfile",
        Some("makefile") => "makefile",
        _ => "",
    }
}

/// Shifts Markdown heading levels down by a specified amount.
///
/// This prevents user-supplied content from injecting top-level structure
/// into the rendered output. For example, with a shift of 2, a `## Heading`
/// in user content becomes `#### Heading`.
///
/// Headings inside fenced code blocks are left unchanged.
/// Caps at H6 (######) since Markdown doesn't support deeper heading levels.
fn shift_headings(s: &str, levels: u8) -> String {
    if levels == 0 {
        return s.to_string();
    }

    let mut result = Vec::new();
    let mut in_code_block = false;

    for line in s.lines() {
        let trimmed = line.trim_start();

        // Track fenced code block boundaries
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_code_block = !in_code_block;
            result.push(line.to_string());
            continue;
        }

        // Only transform headings outside code blocks
        if !in_code_block && line.starts_with('#') {
            let hash_count = line.chars().take_while(|&c| c == '#').count();
            // Valid ATX heading: 1-6 hashes followed by a space
            if hash_count <= 6 && line.chars().nth(hash_count) == Some(' ') {
                let new_level = (hash_count + levels as usize).min(6);
                result.push(format!("{}{}", "#".repeat(new_level), &line[hash_count..]));
                continue;
            }
        }

        result.push(line.to_string());
    }

    result.join("\n")
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

fn format_timestamp(dt: &DateTime<Utc>, zone: TimestampZone) -> String {
    match zone {
        TimestampZone::Utc => dt.format("%Y-%m-%d %H:%M UTC").to_string(),
        TimestampZone::Local => dt
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M %Z")
            .to_string(),
        TimestampZone::Both => {
            let local = dt
                .with_timezone(&Local)
                .format("%Y-%m-%d %H:%M %Z")
                .to_string();
            let utc = dt.format("%Y-%m-%d %H:%M UTC").to_string();
            format!("{local} / {utc}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{ChatExport, Message, Request, ResponseElement};
    use chrono::{Local, TimeZone, Utc};

    const TS_MS: i64 = 1_733_356_800_000; // 2024-12-05 00:00:00 UTC

    fn make_chat(requests: Vec<Request>) -> ChatExport {
        ChatExport {
            responder_username: "GitHub Copilot".into(),
            requests,
        }
    }

    fn make_request(message: &str, response: Vec<ResponseElement>) -> Request {
        Request {
            timestamp: Some(Utc.timestamp_millis_opt(TS_MS).single().unwrap()),
            model_id: Some("claude-sonnet-4".into()),
            agent_name: None,
            context: vec![],
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
            tools: Visibility::Hidden,
            timestamps: TimestampDisplay::Hidden,
            ..Default::default()
        };
        let output = render_chat(&chat, &opts);

        assert!(output.contains("*claude-sonnet-4*"));
    }

    #[test]
    fn renders_timestamp_and_model_when_enabled() {
        let chat = make_chat(vec![make_request("Hi", vec![])]);
        let opts = RenderOptions {
            tools: Visibility::Hidden,
            timestamps: TimestampDisplay::Zoned(TimestampZone::Utc),
            ..Default::default()
        };
        let output = render_chat(&chat, &opts);

        assert!(output.contains("2024-12-05 00:00 UTC"));
        assert!(output.contains("claude-sonnet-4"));
    }

    #[test]
    fn renders_timestamp_in_local_timezone_when_requested() {
        let chat = make_chat(vec![make_request("Hi", vec![])]);
        let opts = RenderOptions {
            tools: Visibility::Hidden,
            timestamps: TimestampDisplay::Zoned(TimestampZone::Local),
            ..Default::default()
        };
        let output = render_chat(&chat, &opts);

        let expected = Utc
            .timestamp_millis_opt(TS_MS)
            .single()
            .unwrap()
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M %Z")
            .to_string();

        assert!(output.contains(&expected));
        assert!(!output.contains("UTC"));
    }

    #[test]
    fn renders_timestamp_in_both_timezones_when_requested() {
        let chat = make_chat(vec![make_request("Hi", vec![])]);
        let opts = RenderOptions {
            tools: Visibility::Hidden,
            timestamps: TimestampDisplay::Zoned(TimestampZone::Both),
            ..Default::default()
        };
        let output = render_chat(&chat, &opts);

        let utc = Utc
            .timestamp_millis_opt(TS_MS)
            .single()
            .unwrap()
            .format("%Y-%m-%d %H:%M UTC")
            .to_string();
        let local = Utc
            .timestamp_millis_opt(TS_MS)
            .single()
            .unwrap()
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M %Z")
            .to_string();

        let expected = format!("{local} / {utc}");

        assert!(output.contains(&expected));
    }

    #[test]
    fn omits_timestamp_when_missing() {
        let mut req = make_request("Hi", vec![]);
        req.timestamp = None;

        let chat = make_chat(vec![req]);
        let opts = RenderOptions {
            tools: Visibility::Hidden,
            timestamps: TimestampDisplay::Zoned(TimestampZone::Utc),
            ..Default::default()
        };
        let output = render_chat(&chat, &opts);

        assert!(!output.contains("UTC"));
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
            tools: Visibility::Hidden,
            timestamps: TimestampDisplay::Hidden,
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
            tools: Visibility::Shown,
            timestamps: TimestampDisplay::Hidden,
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
            tools: Visibility::Shown,
            timestamps: TimestampDisplay::Hidden,
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
            tools: Visibility::Shown,
            timestamps: TimestampDisplay::Hidden,
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

    // Tests for shift_headings helper
    #[test]
    fn shift_headings_basic() {
        assert_eq!(shift_headings("# H1", 2), "### H1");
        assert_eq!(shift_headings("## H2", 2), "#### H2");
        assert_eq!(shift_headings("### H3", 2), "##### H3");
    }

    #[test]
    fn shift_headings_caps_at_h6() {
        assert_eq!(shift_headings("##### H5", 2), "###### H5");
        assert_eq!(shift_headings("###### H6", 2), "###### H6");
        assert_eq!(shift_headings("#### H4", 3), "###### H4");
    }

    #[test]
    fn shift_headings_preserves_content_after_heading() {
        assert_eq!(
            shift_headings("## Title with **bold** and `code`", 2),
            "#### Title with **bold** and `code`"
        );
    }

    #[test]
    fn shift_headings_multiline() {
        let input = "## First\n\nSome text\n\n### Second";
        let expected = "#### First\n\nSome text\n\n##### Second";
        assert_eq!(shift_headings(input, 2), expected);
    }

    #[test]
    fn shift_headings_ignores_non_headings() {
        // No space after # - not a heading
        assert_eq!(shift_headings("#hashtag", 2), "#hashtag");
        // Just hashes
        assert_eq!(shift_headings("###", 2), "###");
        // Regular text
        assert_eq!(shift_headings("regular text", 2), "regular text");
    }

    #[test]
    fn shift_headings_skips_code_blocks() {
        let input = "## Real heading\n\n```\n## Not a heading\n```\n\n## Another real one";
        let expected = "#### Real heading\n\n```\n## Not a heading\n```\n\n#### Another real one";
        assert_eq!(shift_headings(input, 2), expected);
    }

    #[test]
    fn shift_headings_skips_tilde_code_blocks() {
        let input = "## Heading\n\n~~~\n# Code comment\n~~~";
        let expected = "#### Heading\n\n~~~\n# Code comment\n~~~";
        assert_eq!(shift_headings(input, 2), expected);
    }

    #[test]
    fn shift_headings_handles_nested_code_blocks() {
        let input = "## Start\n\n```\ncode\n```\n\n## Middle\n\n```\nmore\n```\n\n## End";
        let expected = "#### Start\n\n```\ncode\n```\n\n#### Middle\n\n```\nmore\n```\n\n#### End";
        assert_eq!(shift_headings(input, 2), expected);
    }

    #[test]
    fn shift_headings_empty_input() {
        assert_eq!(shift_headings("", 2), "");
    }

    #[test]
    fn shift_headings_preserves_leading_whitespace() {
        // Indented headings aren't valid Markdown headings, should be unchanged
        assert_eq!(shift_headings("  ## Indented", 2), "  ## Indented");
    }

    #[test]
    fn shift_headings_zero_shift() {
        assert_eq!(shift_headings("## Heading", 0), "## Heading");
    }

    #[test]
    fn user_message_headings_are_shifted() {
        let chat = make_chat(vec![make_request(
            "## My Heading\n\nSome content\n\n### Subheading",
            vec![ResponseElement::Text("Response".into())],
        )]);
        let output = render_chat(&chat, &default_opts());

        // User's ## should become #### (shifted by 2)
        assert!(output.contains("#### My Heading"));
        // User's ### should become ##### (shifted by 2)
        assert!(output.contains("##### Subheading"));
        // Our structure should remain unchanged
        assert!(output.contains("## User"));
        assert!(output.contains("## Assistant"));
    }

    #[test]
    fn user_message_headings_shifted_with_offset() {
        let chat = make_chat(vec![make_request(
            "# Top heading",
            vec![ResponseElement::Text("Response".into())],
        )]);
        let opts = RenderOptions {
            heading_offset: 1,
            ..Default::default()
        };
        let output = render_chat(&chat, &opts);

        // With offset 1: our H2 becomes H3, so user H1 shifts by 3 → H4
        assert!(output.contains("#### Top heading"));
        // Our structure uses offset
        assert!(output.contains("### User"));
    }

    // Tests for show_edits option
    #[test]
    fn hides_edit_code_by_default() {
        let chat = make_chat(vec![make_request(
            "Edit",
            vec![ResponseElement::TextEditGroup {
                path: "/src/main.rs".into(),
                edits: vec!["fn main() {}".into()],
            }],
        )]);
        let output = render_chat(&chat, &default_opts());

        assert!(output.contains("*Modified `main.rs`"));
        assert!(!output.contains("fn main()"));
    }

    #[test]
    fn renders_text_edit_with_code_when_show_edits_enabled() {
        let chat = make_chat(vec![make_request(
            "Edit",
            vec![ResponseElement::TextEditGroup {
                path: "/src/main.rs".into(),
                edits: vec!["fn main() {\n    println!(\"hello\");\n}".into()],
            }],
        )]);
        let opts = RenderOptions {
            edits: EditDisplay::WithCode,
            ..Default::default()
        };
        let output = render_chat(&chat, &opts);

        assert!(output.contains("*Modified `main.rs`"));
        assert!(output.contains("```rust"));
        assert!(output.contains("fn main()"));
        assert!(output.contains("println!"));
    }

    #[test]
    fn renders_multiple_edits_with_separator() {
        let chat = make_chat(vec![make_request(
            "Edit",
            vec![ResponseElement::TextEditGroup {
                path: "/src/lib.rs".into(),
                edits: vec!["fn first() {}".into(), "fn second() {}".into()],
            }],
        )]);
        let opts = RenderOptions {
            edits: EditDisplay::WithCode,
            ..Default::default()
        };
        let output = render_chat(&chat, &opts);

        assert!(output.contains("fn first()"));
        assert!(output.contains("// ..."));
        assert!(output.contains("fn second()"));
    }

    // Tests for extension_to_language helper
    #[test]
    fn extension_to_language_common_extensions() {
        // Systems
        assert_eq!(extension_to_language("/src/main.rs"), "rust");
        assert_eq!(extension_to_language("/cmd/main.go"), "go");
        assert_eq!(extension_to_language("/src/main.zig"), "zig");
        // JVM
        assert_eq!(extension_to_language("/App.java"), "java");
        assert_eq!(extension_to_language("/App.scala"), "scala");
        assert_eq!(extension_to_language("/core.clj"), "clojure");
        // Dynamic
        assert_eq!(extension_to_language("/app/server.py"), "python");
        assert_eq!(extension_to_language("/index.php"), "php");
        assert_eq!(extension_to_language("/script.lua"), "lua");
        assert_eq!(extension_to_language("/analysis.r"), "r");
        // Functional
        assert_eq!(extension_to_language("/Main.hs"), "haskell");
        assert_eq!(extension_to_language("/app.ex"), "elixir");
        // Web
        assert_eq!(extension_to_language("/lib/utils.js"), "javascript");
        assert_eq!(extension_to_language("/src/app.ts"), "typescript");
        assert_eq!(extension_to_language("/App.vue"), "vue");
        assert_eq!(extension_to_language("/App.svelte"), "svelte");
        // Mobile
        assert_eq!(extension_to_language("/ViewController.m"), "objectivec");
        assert_eq!(extension_to_language("/main.dart"), "dart");
        // Shell
        assert_eq!(extension_to_language("/script.sh"), "shell");
        assert_eq!(extension_to_language("/script.bash"), "bash");
        // Config
        assert_eq!(extension_to_language("/config.json"), "json");
        assert_eq!(extension_to_language("/main.tf"), "hcl");
        assert_eq!(extension_to_language("/schema.proto"), "protobuf");
        // Styles
        assert_eq!(extension_to_language("/styles.css"), "css");
        assert_eq!(extension_to_language("/styles.less"), "less");
        // Other
        assert_eq!(extension_to_language("/changes.diff"), "diff");
    }

    #[test]
    fn extension_to_language_unknown_returns_empty() {
        assert_eq!(extension_to_language("/file.xyz"), "");
        assert_eq!(extension_to_language("/no_extension"), "");
    }
}
