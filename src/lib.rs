// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2025 Brian Hetro <whee@smaertness.net>

//! Convert GitHub Copilot chat exports to Markdown.
//!
//! This crate provides parsing and rendering functionality for transforming
//! GitHub Copilot's JSON chat export format into readable Markdown documents.
//!
//! # Overview
//!
//! GitHub Copilot stores chat conversations as JSON files. This crate:
//!
//! 1. Parses the JSON structure into typed Rust representations
//! 2. Renders the conversations as clean Markdown with configurable output
//!
//! # Example
//!
//! ```no_run
//! use cp2md::{parser, renderer};
//!
//! let json = std::fs::read_to_string("chat.json").unwrap();
//! let chat = parser::parse_chat(&json).unwrap();
//!
//! let opts = renderer::RenderOptions {
//!     show_tools: true,
//!     show_timestamps: true,
//!     ..Default::default()
//! };
//!
//! let markdown = renderer::render_chat(&chat, &opts);
//! println!("{markdown}");
//! ```
//!
//! # Modules
//!
//! - [`parser`]: JSON parsing and type definitions for Copilot chat exports
//! - [`renderer`]: Markdown generation with configurable output options

#![deny(missing_docs)]

pub mod parser;
pub mod renderer;
