// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2025 Brian Hetro <whee@smaertness.net>

//! Integration tests for cp2md parsing and rendering.

use cp2md::{parser, renderer};
use std::fs;
use std::path::Path;

/// Parses all JSON files in the chats directory and verifies they produce valid output.
#[test]
fn parses_all_sample_chats() {
    let chats_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("chats");

    if !chats_dir.exists() {
        // Skip if no sample chats directory
        return;
    }

    for entry in fs::read_dir(&chats_dir).expect("Failed to read chats directory") {
        let entry = entry.expect("Failed to read directory entry");
        let path = entry.path();

        if path.extension().is_some_and(|ext| ext == "json") {
            let json = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("Failed to read {}: {e}", path.display()));

            let chat = parser::parse_chat(&json)
                .unwrap_or_else(|e| panic!("Failed to parse {}: {e}", path.display()));

            // Verify basic structure
            assert!(
                !chat.responder_username.is_empty(),
                "Empty responder username in {}",
                path.display()
            );

            // Verify we can render it
            let opts = renderer::RenderOptions::default();
            let markdown = renderer::render_chat(&chat, &opts);

            assert!(
                markdown.starts_with("# Copilot Chat"),
                "Invalid markdown header in {}",
                path.display()
            );
        }
    }
}

/// Tests that the verbose output includes tool invocations.
#[test]
fn verbose_output_includes_tools() {
    let json = r#"{
        "responderUsername": "GitHub Copilot",
        "requests": [{
            "timestamp": 1733356800000,
            "message": { "text": "Search for something" },
            "response": [
                {
                    "kind": "toolInvocationSerialized",
                    "pastTenseMessage": { "value": "Searched for files" }
                },
                { "value": "Found some results." }
            ]
        }]
    }"#;

    let chat = parser::parse_chat(json).unwrap();

    // Without verbose flag
    let quiet_opts = renderer::RenderOptions {
        show_tools: false,
        show_timestamps: false,
        ..Default::default()
    };
    let quiet_output = renderer::render_chat(&chat, &quiet_opts);
    assert!(
        !quiet_output.contains("🔧"),
        "Tool invocation should be hidden without verbose flag"
    );

    // With verbose flag
    let verbose_opts = renderer::RenderOptions {
        show_tools: true,
        show_timestamps: false,
        ..Default::default()
    };
    let verbose_output = renderer::render_chat(&chat, &verbose_opts);
    assert!(
        verbose_output.contains("🔧 Searched for files"),
        "Tool invocation should be visible with verbose flag"
    );
}

/// Tests that timestamps are properly formatted when enabled.
#[test]
fn timestamps_formatted_correctly() {
    let json = r#"{
        "responderUsername": "GitHub Copilot",
        "requests": [{
            "timestamp": 1733356800000,
            "modelId": "claude-sonnet-4",
            "message": { "text": "Hello" },
            "response": []
        }]
    }"#;

    let chat = parser::parse_chat(json).unwrap();

    let opts = renderer::RenderOptions {
        show_tools: false,
        show_timestamps: true,
        ..Default::default()
    };
    let output = renderer::render_chat(&chat, &opts);

    assert!(
        output.contains("2024-12-05 00:00 UTC"),
        "Timestamp should be formatted as date and time"
    );
    assert!(
        output.contains("claude-sonnet-4"),
        "Model ID should be included"
    );
}

/// Tests that file edit summaries are correctly rendered.
#[test]
fn file_edit_summary_rendered() {
    let json = r#"{
        "responderUsername": "GitHub Copilot",
        "requests": [{
            "timestamp": 1733356800000,
            "message": { "text": "Edit the file" },
            "response": [
                {
                    "kind": "textEditGroup",
                    "uri": { "path": "/src/main.rs" },
                    "edits": [
                        [{"text": "fn main() {\n    println!(\"Hello\");\n}"}]
                    ]
                }
            ]
        }]
    }"#;

    let chat = parser::parse_chat(json).unwrap();
    let output = renderer::render_chat(&chat, &renderer::RenderOptions::default());

    assert!(
        output.contains("Modified `main.rs`"),
        "Should show modified filename"
    );
    assert!(output.contains("3 lines"), "Should show line count");
}
