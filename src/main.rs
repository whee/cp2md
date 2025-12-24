// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2025 Brian Hetro <whee@smaertness.net>

//! Command-line interface for cp2md.
//!
//! This binary provides the `cp2md` command for converting GitHub Copilot
//! chat exports from JSON to Markdown format.

use cp2md::{parser, renderer};
use lexopt::prelude::*;
use snafu::{OptionExt, ensure, prelude::*};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Where to write the rendered output.
#[derive(Clone)]
enum OutputTarget {
    /// Write each file to the specified directory.
    Directory(PathBuf),
    /// Write to stdout.
    Stdout,
}

#[allow(clippy::struct_excessive_bools)]
struct Cli {
    input: Vec<PathBuf>,
    output: OutputTarget,
    concat: bool,
    show_tools: bool,
    show_timestamps: bool,
    show_model: bool,
    show_agent: bool,
    show_context: bool,
    heading_offset: u8,
    quiet: bool,
    dry_run: bool,
    force: bool,
}

#[derive(Debug, Snafu)]
enum Error {
    #[snafu(display("failed to parse arguments: {source}"))]
    ParseArgs { source: lexopt::Error },

    #[snafu(display("heading-offset must be 0-5"))]
    InvalidHeadingOffset,

    #[snafu(display("at least one input file or directory is required"))]
    NoInputFiles,

    #[snafu(display("cannot output multiple files to stdout without --concat"))]
    MultipleFilesToStdout,

    #[snafu(display("failed to create output directory: {source}"))]
    CreateOutputDir { source: std::io::Error },

    #[snafu(display("failed to read {}: {source}", path.display()))]
    ReadFile {
        path: PathBuf,
        source: std::io::Error,
    },

    #[snafu(display("failed to parse {}: {source}", path.display()))]
    ParseFile {
        path: PathBuf,
        source: parser::ParseError,
    },

    #[snafu(display("invalid input filename: no file stem"))]
    InvalidFilename,

    #[snafu(display("failed to write {}: {source}", path.display()))]
    WriteFile {
        path: PathBuf,
        source: std::io::Error,
    },
}

fn print_help() {
    println!(
        "\
{name} {version}
Convert GitHub Copilot chat exports to Markdown

Usage: {name} [OPTIONS] -o <OUTPUT> <INPUT>...

Arguments:
  <INPUT>...  Input JSON files or directories containing exports

Options:
  -o, --output <OUTPUT>     Output directory (or file with --concat, or - for stdout)
      --concat              Combine all inputs into a single output
      --heading-offset <N>  Shift heading levels by N (0-5, default: 0)

Metadata display (use --show-* or --hide-*):
      --show-timestamps     Include timestamps (default: off)
      --hide-timestamps     Hide timestamps
      --show-model          Include model ID (default: on)
      --hide-model          Hide model ID
      --show-agent          Include agent name (default: on)
      --hide-agent          Hide agent name
      --show-context        Include attached context (default: on)
      --hide-context        Hide attached context
      --show-tools          Include tool invocations (default: off)
      --hide-tools          Hide tool invocations
  -v, --verbose             Alias for --show-tools

Other options:
  -q, --quiet               Suppress progress messages
  -n, --dry-run             Show what would be processed without writing
  -f, --force               Overwrite existing output files
  -h, --help                Print help
  -V, --version             Print version",
        name = env!("CARGO_PKG_NAME"),
        version = env!("CARGO_PKG_VERSION"),
    );
}

fn parse_args() -> Result<Cli, lexopt::Error> {
    // Show help if no arguments provided
    if std::env::args().len() == 1 {
        print_help();
        std::process::exit(0);
    }

    let mut input = Vec::new();
    let mut output: Option<OutputTarget> = None;
    let mut concat = false;
    // Defaults: tools off, timestamps off, model on, agent on, context on
    let mut show_tools = false;
    let mut show_timestamps = false;
    let mut show_model = true;
    let mut show_agent = true;
    let mut show_context = true;
    let mut heading_offset: u8 = 0;
    let mut quiet = false;
    let mut dry_run = false;
    let mut force = false;

    let mut parser = lexopt::Parser::from_env();
    while let Some(arg) = parser.next()? {
        match arg {
            Short('o') | Long("output") => {
                let val: PathBuf = parser.value()?.parse()?;
                output = Some(if val == Path::new("-") {
                    OutputTarget::Stdout
                } else {
                    OutputTarget::Directory(val)
                });
            }
            Long("concat") => concat = true,
            // Show/hide flags - last one wins
            Short('v') | Long("verbose" | "show-tools") => show_tools = true,
            Long("hide-tools") => show_tools = false,
            Long("show-timestamps") => show_timestamps = true,
            Long("hide-timestamps") => show_timestamps = false,
            Long("show-model") => show_model = true,
            Long("hide-model" | "no-model") => show_model = false,
            Long("show-agent") => show_agent = true,
            Long("hide-agent") => show_agent = false,
            Long("show-context") => show_context = true,
            Long("hide-context") => show_context = false,
            Long("heading-offset") => {
                let val: u8 = parser
                    .value()?
                    .parse()
                    .map_err(|_| "heading-offset must be a number 0-5")?;
                if val > 5 {
                    return Err("heading-offset must be 0-5".into());
                }
                heading_offset = val;
            }
            Short('q') | Long("quiet") => quiet = true,
            Short('n') | Long("dry-run") => dry_run = true,
            Short('f') | Long("force") => force = true,
            Short('h') | Long("help") => {
                print_help();
                std::process::exit(0);
            }
            Short('V') | Long("version") => {
                println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            Value(val) => input.push(val.parse()?),
            _ => return Err(arg.unexpected()),
        }
    }

    Ok(Cli {
        input,
        output: output.ok_or("missing required option: --output")?,
        concat,
        show_tools,
        show_timestamps,
        show_model,
        show_agent,
        show_context,
        heading_offset,
        quiet,
        dry_run,
        force,
    })
}

fn main() -> Result<(), Error> {
    let cli = parse_args().context(ParseArgsSnafu)?;

    ensure!(!cli.input.is_empty(), NoInputFilesSnafu);

    // Collect all input files first
    let files = collect_input_files(&cli.input);

    if cli.concat {
        process_concat(&files, &cli)?;
    } else {
        match &cli.output {
            OutputTarget::Stdout => {
                // Without concat, we can only output one file to stdout
                ensure!(files.len() == 1, MultipleFilesToStdoutSnafu);
                process_to_stdout(&files[0], &cli)?;
            }
            OutputTarget::Directory(dir) => {
                if !cli.dry_run {
                    std::fs::create_dir_all(dir).context(CreateOutputDirSnafu)?;
                }
                for file in &files {
                    process_file(file, dir, &cli)?;
                }
            }
        }
    }

    Ok(())
}

/// Collects all JSON files from the given inputs (files and directories).
fn collect_input_files(inputs: &[PathBuf]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for input in inputs {
        if input.is_dir() {
            for entry in WalkDir::new(input)
                .into_iter()
                .filter_map(Result::ok)
                .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
            {
                files.push(entry.path().to_path_buf());
            }
        } else {
            files.push(input.clone());
        }
    }
    files
}

/// Creates render options from CLI arguments.
#[allow(clippy::missing_const_for_fn)]
fn make_render_options(cli: &Cli) -> renderer::RenderOptions {
    renderer::RenderOptions {
        show_tools: cli.show_tools,
        show_timestamps: cli.show_timestamps,
        show_model: cli.show_model,
        show_agent: cli.show_agent,
        show_context: cli.show_context,
        heading_offset: cli.heading_offset,
    }
}

/// Processes a single file and outputs to stdout.
fn process_to_stdout(input: &Path, cli: &Cli) -> Result<(), Error> {
    if cli.dry_run {
        eprintln!("Would output {}", input.display());
        return Ok(());
    }

    let json = std::fs::read_to_string(input).context(ReadFileSnafu { path: input })?;
    let chat = parser::parse_chat(&json).context(ParseFileSnafu { path: input })?;

    let opts = make_render_options(cli);
    let markdown = renderer::render_chat(&chat, &opts);

    print!("{markdown}");
    Ok(())
}

/// Processes multiple files and concatenates them into a single output.
fn process_concat(files: &[PathBuf], cli: &Cli) -> Result<(), Error> {
    let opts = make_render_options(cli);
    let mut output = String::new();

    for (i, path) in files.iter().enumerate() {
        if i > 0 {
            output.push_str("\n---\n\n");
        }
        let json = std::fs::read_to_string(path).context(ReadFileSnafu { path })?;
        let chat = parser::parse_chat(&json).context(ParseFileSnafu { path })?;
        output.push_str(&renderer::render_chat(&chat, &opts));
    }

    match &cli.output {
        OutputTarget::Stdout => {
            if cli.dry_run {
                eprintln!("Would output {} files concatenated", files.len());
            } else {
                print!("{output}");
            }
        }
        OutputTarget::Directory(path) => {
            // In concat mode, treat path as a file, not directory
            if cli.dry_run {
                eprintln!(
                    "Would write {} ({} files concatenated)",
                    path.display(),
                    files.len()
                );
            } else if path.exists() && !cli.force {
                eprintln!(
                    "Skipping {} (already exists, use --force to overwrite)",
                    path.display()
                );
            } else {
                // Create parent directory if needed
                if let Some(parent) = path.parent()
                    && !parent.as_os_str().is_empty()
                {
                    std::fs::create_dir_all(parent).context(CreateOutputDirSnafu)?;
                }
                std::fs::write(path, &output).context(WriteFileSnafu { path })?;
                if !cli.quiet {
                    eprintln!("Wrote {} ({} files)", path.display(), files.len());
                }
            }
        }
    }

    Ok(())
}

/// Processes a single file and writes to the output directory.
fn process_file(input: &Path, out_dir: &Path, cli: &Cli) -> Result<(), Error> {
    let out_name = input.file_stem().context(InvalidFilenameSnafu)?;
    let out_path = out_dir.join(format!("{}.md", out_name.to_string_lossy()));

    // Handle dry-run mode
    if cli.dry_run {
        eprintln!("Would write {}", out_path.display());
        return Ok(());
    }

    // Check if output exists and handle overwrite
    if out_path.exists() && !cli.force {
        eprintln!(
            "Skipping {} (already exists, use --force to overwrite)",
            out_path.display()
        );
        return Ok(());
    }

    let json = std::fs::read_to_string(input).context(ReadFileSnafu { path: input })?;
    let chat = parser::parse_chat(&json).context(ParseFileSnafu { path: input })?;

    let opts = make_render_options(cli);
    let markdown = renderer::render_chat(&chat, &opts);

    std::fs::write(&out_path, &markdown).context(WriteFileSnafu { path: &out_path })?;

    if !cli.quiet {
        eprintln!("Wrote {}", out_path.display());
    }
    Ok(())
}
