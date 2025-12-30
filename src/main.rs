// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2025 Brian Hetro <whee@smaertness.net>

//! Command-line interface for cp2md.
//!
//! This binary provides the `cp2md` command for converting GitHub Copilot
//! chat exports from JSON to Markdown format.

use cp2md::{parser, renderer};
use itertools::Itertools;
use lexopt::prelude::*;
use snafu::{OptionExt, ensure, prelude::*};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use walkdir::WalkDir;
/// Where to write the rendered output.
#[derive(Clone, Debug)]
enum OutputTarget {
    /// Write each file to the specified directory.
    Directory(PathBuf),
    /// Write concatenated output to a single file.
    File(PathBuf),
    /// Write to stdout.
    Stdout,
}

#[derive(Debug)]
#[allow(clippy::struct_excessive_bools)]
struct Cli {
    input: Vec<PathBuf>,
    output: OutputTarget,
    concat: bool,
    tools: renderer::Visibility,
    timestamps: renderer::TimestampDisplay,
    model: renderer::Visibility,
    agent: renderer::Visibility,
    context: renderer::Visibility,
    edits: renderer::EditDisplay,
    heading_offset: u8,
    quiet: bool,
    dry_run: bool,
    force: bool,
}

#[derive(Debug, Snafu)]
enum Error {
    #[snafu(display("failed to parse arguments"))]
    ParseArgs { source: lexopt::Error },

    #[snafu(display("heading-offset must be 0-5"))]
    InvalidHeadingOffset,

    #[snafu(display(
        "conflicting timestamp flags: already set to {:?}, got {:?}",
        first,
        second
    ))]
    ConflictingTimestampFlags {
        first: renderer::TimestampZone,
        second: renderer::TimestampZone,
    },

    #[snafu(display("missing required option: --output"))]
    MissingOutput,

    #[snafu(display("failed to list inputs under {}", path.display()))]
    ListInputs {
        path: PathBuf,
        source: walkdir::Error,
    },

    #[snafu(display("at least one input file or directory is required"))]
    NoInputFiles,

    #[snafu(display("cannot output multiple files to stdout without --concat"))]
    MultipleFilesToStdout,

    #[snafu(display("failed to create output directory {}", path.display()))]
    CreateOutputDir {
        path: PathBuf,
        source: std::io::Error,
    },

    #[snafu(display("failed to read {}", path.display()))]
    ReadFile {
        path: PathBuf,
        source: std::io::Error,
    },

    #[snafu(display("failed to parse {}", path.display()))]
    ParseFile {
        path: PathBuf,
        source: parser::ParseError,
    },

    #[snafu(display("invalid input filename: no file stem"))]
    InvalidFilename,

    #[snafu(display("failed to write {}", path.display()))]
    WriteFile {
        path: PathBuf,
        source: std::io::Error,
    },

    #[snafu(display("file output requires --concat (got {})", path.display()))]
    FileOutputRequiresConcat { path: PathBuf },
}

#[derive(Clone, Debug)]
struct RenderFlagState {
    tools: renderer::Visibility,
    model: renderer::Visibility,
    agent: renderer::Visibility,
    context: renderer::Visibility,
    edits: renderer::EditDisplay,
    timestamps: renderer::TimestampDisplay,
    timestamp_preference: Option<renderer::TimestampZone>,
}

impl RenderFlagState {
    const fn new() -> Self {
        Self {
            tools: renderer::Visibility::Hidden,
            model: renderer::Visibility::Shown,
            agent: renderer::Visibility::Shown,
            context: renderer::Visibility::Shown,
            edits: renderer::EditDisplay::SummaryOnly,
            timestamps: renderer::TimestampDisplay::Hidden,
            timestamp_preference: None,
        }
    }

    const fn apply_compact(&mut self) {
        self.tools = renderer::Visibility::Hidden;
        self.model = renderer::Visibility::Hidden;
        self.agent = renderer::Visibility::Hidden;
        self.context = renderer::Visibility::Hidden;
        self.timestamps = renderer::TimestampDisplay::Hidden;
    }

    fn set_timestamp_zone(&mut self, zone: renderer::TimestampZone) -> Result<(), Error> {
        if let Some(first) = self.timestamp_preference.filter(|current| *current != zone) {
            return ConflictingTimestampFlagsSnafu {
                first,
                second: zone,
            }
            .fail();
        }

        self.timestamp_preference = Some(zone);

        self.timestamps = renderer::TimestampDisplay::Zoned(zone);

        Ok(())
    }

    fn show_timestamps(&mut self) {
        let zone = self
            .timestamp_preference
            .unwrap_or(renderer::TimestampZone::Utc);
        self.timestamps = renderer::TimestampDisplay::Zoned(zone);
    }

    const fn hide_timestamps(&mut self) {
        self.timestamps = renderer::TimestampDisplay::Hidden;
    }

    fn finalize(self) -> RenderFlags {
        let zone = self
            .timestamp_preference
            .unwrap_or(renderer::TimestampZone::Utc);

        let timestamps = match self.timestamps {
            renderer::TimestampDisplay::Hidden => renderer::TimestampDisplay::Hidden,
            renderer::TimestampDisplay::Zoned(_) => renderer::TimestampDisplay::Zoned(zone),
        };

        RenderFlags {
            tools: self.tools,
            model: self.model,
            agent: self.agent,
            context: self.context,
            edits: self.edits,
            timestamps,
        }
    }
}

#[derive(Clone, Debug)]
struct RenderFlags {
    tools: renderer::Visibility,
    model: renderer::Visibility,
    agent: renderer::Visibility,
    context: renderer::Visibility,
    edits: renderer::EditDisplay,
    timestamps: renderer::TimestampDisplay,
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
    -o, --output <OUTPUT>     Output directory (or file with --concat; - for stdout)
      --concat              Combine all inputs into a single output
      --heading-offset <N>  Shift heading levels by N (0-5, default: 0)

Metadata display (use --show-* or --hide-*):
      --show-timestamps     Include timestamps (default: off)
      --hide-timestamps     Hide timestamps
      --local-time          Render timestamps in the local timezone (default: UTC)
      --utc-time            Render timestamps in UTC (default)
      --timestamps-both     Render timestamps as <local> / <utc>
      --show-model          Include model ID (default: on)
      --hide-model          Hide model ID
      --show-agent          Include agent name (default: on)
      --hide-agent          Hide agent name
      --show-context        Include attached context (default: on)
      --hide-context        Hide attached context
      --show-tools          Include tool invocations (default: off)
      --hide-tools          Hide tool invocations
      --show-edits          Include full code for file edits (default: off)
      --hide-edits          Hide full code for file edits
  -v, --verbose             Alias for --show-tools
      --compact             Hide all metadata (model, agent, context, tools, timestamps)

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

fn parse_args() -> Result<Cli, Error> {
    // Show help if no arguments provided
    if std::env::args().len() == 1 {
        print_help();
        std::process::exit(0);
    }
    parse_args_from(std::env::args().skip(1))
}

fn parse_args_from(
    args: impl IntoIterator<Item = impl Into<std::ffi::OsString>>,
) -> Result<Cli, Error> {
    let mut input = Vec::new();
    let mut output: Option<OutputTarget> = None;
    let mut concat = false;
    let mut flags = RenderFlagState::new();
    let mut heading_offset: u8 = 0;
    let mut quiet = false;
    let mut dry_run = false;
    let mut force = false;

    let mut parser = lexopt::Parser::from_args(args);
    while let Some(arg) = parser.next().context(ParseArgsSnafu)? {
        match arg {
            Short('o') | Long("output") => {
                let val: PathBuf = parser
                    .value()
                    .context(ParseArgsSnafu)?
                    .parse()
                    .context(ParseArgsSnafu)?;
                output = Some(if val == Path::new("-") {
                    OutputTarget::Stdout
                } else {
                    OutputTarget::Directory(val)
                });
            }
            Long("concat") => concat = true,
            // Show/hide toggles; timestamp flags are validated for conflicts
            Short('v') | Long("verbose" | "show-tools") => {
                flags.tools = renderer::Visibility::Shown;
            }
            Long("hide-tools") => {
                flags.tools = renderer::Visibility::Hidden;
            }
            Long("show-timestamps") => {
                flags.show_timestamps();
            }
            Long("hide-timestamps") => {
                flags.hide_timestamps();
            }
            Long("local-time") => flags.set_timestamp_zone(renderer::TimestampZone::Local)?,
            Long("utc-time") => flags.set_timestamp_zone(renderer::TimestampZone::Utc)?,
            Long("timestamps-both") => flags.set_timestamp_zone(renderer::TimestampZone::Both)?,
            Long("show-model") => flags.model = renderer::Visibility::Shown,
            Long("hide-model" | "no-model") => flags.model = renderer::Visibility::Hidden,
            Long("show-agent") => flags.agent = renderer::Visibility::Shown,
            Long("hide-agent") => flags.agent = renderer::Visibility::Hidden,
            Long("show-context") => flags.context = renderer::Visibility::Shown,
            Long("hide-context") => flags.context = renderer::Visibility::Hidden,
            Long("show-edits") => flags.edits = renderer::EditDisplay::WithCode,
            Long("hide-edits") => flags.edits = renderer::EditDisplay::SummaryOnly,
            Long("compact") => {
                flags.apply_compact();
            }
            Long("heading-offset") => {
                let val: u8 = parser
                    .value()
                    .context(ParseArgsSnafu)?
                    .parse()
                    .context(ParseArgsSnafu)?;
                ensure!(val <= 5, InvalidHeadingOffsetSnafu);
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
            Value(val) => input.push(val.parse().context(ParseArgsSnafu)?),
            _ => return Err(arg.unexpected()).context(ParseArgsSnafu),
        }
    }

    let output = output.context(MissingOutputSnafu)?;
    let output = match (concat, &output) {
        (true, OutputTarget::Directory(path)) => OutputTarget::File(path.clone()),
        _ => output,
    };

    let flags = flags.finalize();

    Ok(Cli {
        input,
        output,
        concat,
        tools: flags.tools,
        timestamps: flags.timestamps,
        model: flags.model,
        agent: flags.agent,
        context: flags.context,
        edits: flags.edits,
        heading_offset,
        quiet,
        dry_run,
        force,
    })
}

#[snafu::report]
fn main() -> Result<(), Error> {
    let cli = parse_args()?;

    ensure!(!cli.input.is_empty(), NoInputFilesSnafu);

    // Collect all input files first
    let files = collect_input_files(&cli.input)?;

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
                    fs::create_dir_all(dir).context(CreateOutputDirSnafu { path: dir })?;
                }
                for file in &files {
                    process_file(file, dir, &cli)?;
                }
            }
            OutputTarget::File(path) => {
                return FileOutputRequiresConcatSnafu { path: path.clone() }.fail();
            }
        }
    }

    Ok(())
}

/// Describes an input for file collection: either a direct file or a directory
/// with its pre-walked contents (sorted, flattened).
#[derive(Debug)]
enum InputDescriptor {
    File(PathBuf),
    Directory { contents: Vec<PathBuf> },
}

/// Pure function: collects and deduplicates .json files from input descriptors.
///
/// Direct files are added as-is (no extension filtering). Directory contents
/// are filtered to .json files only. Order is preserved and duplicates removed.
fn collect_from_descriptors(inputs: &[InputDescriptor]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut seen = HashSet::new();

    for input in inputs {
        match input {
            InputDescriptor::File(path) => {
                if seen.insert(path.clone()) {
                    files.push(path.clone());
                }
            }
            InputDescriptor::Directory { contents } => {
                for path in contents {
                    if path.extension().is_some_and(|ext| ext == "json")
                        && seen.insert(path.clone())
                    {
                        files.push(path.clone());
                    }
                }
            }
        }
    }

    files
}

/// Collects all JSON files from the given inputs (files and directories).
///
/// Directory traversal is sorted and deduplicated so multi-run output is
/// deterministic and we never re-render the same file twice. Traversal errors
/// are surfaced instead of silently skipping entries so the caller can fail
/// fast when input discovery is incomplete.
fn collect_input_files(inputs: &[PathBuf]) -> Result<Vec<PathBuf>, Error> {
    let mut descriptors = Vec::with_capacity(inputs.len());

    for input in inputs {
        if input.is_dir() {
            let mut contents = Vec::new();
            for entry in WalkDir::new(input).sort_by_file_name() {
                let entry = entry.context(ListInputsSnafu {
                    path: input.clone(),
                })?;
                contents.push(entry.into_path());
            }
            descriptors.push(InputDescriptor::Directory { contents });
        } else {
            descriptors.push(InputDescriptor::File(input.clone()));
        }
    }

    Ok(collect_from_descriptors(&descriptors))
}

/// Creates render options from CLI arguments.
const fn make_render_options(cli: &Cli) -> renderer::RenderOptions {
    renderer::RenderOptions {
        tools: cli.tools,
        timestamps: cli.timestamps,
        model: cli.model,
        agent: cli.agent,
        context: cli.context,
        edits: cli.edits,
        heading_offset: cli.heading_offset,
    }
}

/// Loads a chat file, ensuring all callers surface consistent error context.
fn load_chat(path: &Path) -> Result<parser::ChatExport, Error> {
    let json = fs::read_to_string(path).context(ReadFileSnafu { path })?;
    parser::parse_chat(&json).context(ParseFileSnafu { path })
}

/// Processes a single file and outputs to stdout via a shared plan.
fn process_to_stdout(input: &Path, cli: &Cli) -> Result<(), Error> {
    let chat = load_chat(input)?;

    let opts = make_render_options(cli);
    let markdown = renderer::render_chat(&chat, &opts);

    let plan = OutputPlan {
        destination: OutputDestination::Stdout,
        description: Some(input.display().to_string()),
        content: markdown,
    };

    apply_output_plan(plan, cli)
}

/// Planned output destination.
enum OutputDestination {
    Stdout,
    File(PathBuf),
}

/// A rendered artifact and where it should be written.
struct OutputPlan {
    destination: OutputDestination,
    description: Option<String>,
    content: String,
}

fn apply_output_plan(plan: OutputPlan, cli: &Cli) -> Result<(), Error> {
    match plan.destination {
        OutputDestination::Stdout => {
            if cli.dry_run {
                if let Some(desc) = plan.description.as_deref() {
                    eprintln!("Would output {desc}");
                } else {
                    eprintln!("Would output to stdout");
                }
            } else {
                print!("{}", plan.content);
            }
        }
        OutputDestination::File(path) => {
            if cli.dry_run {
                eprintln!("Would write {}", path.display());
                return Ok(());
            }

            if path.exists() && !cli.force {
                eprintln!(
                    "Skipping {} (already exists, use --force to overwrite)",
                    path.display()
                );
                return Ok(());
            }

            if let Some(parent) = path.parent()
                && !parent.as_os_str().is_empty()
            {
                fs::create_dir_all(parent).context(CreateOutputDirSnafu { path: parent })?;
            }

            fs::write(&path, &plan.content).context(WriteFileSnafu { path: &path })?;

            if !cli.quiet {
                if let Some(desc) = plan.description.as_deref() {
                    eprintln!("Wrote {} ({desc})", path.display());
                } else {
                    eprintln!("Wrote {}", path.display());
                }
            }
        }
    }

    Ok(())
}

/// Pure: renders multiple chats into a single concatenated output.
fn render_concat(chats: &[parser::ChatExport], opts: &renderer::RenderOptions) -> String {
    chats
        .iter()
        .map(|chat| renderer::render_chat(chat, opts))
        .join("\n---\n\n")
}

/// Processes multiple files and concatenates them into a single output.
fn process_concat(files: &[PathBuf], cli: &Cli) -> Result<(), Error> {
    let chats: Vec<_> = files
        .iter()
        .map(|p| load_chat(p))
        .collect::<Result<_, _>>()?;
    let opts = make_render_options(cli);
    let output = render_concat(&chats, &opts);

    let destination = match &cli.output {
        OutputTarget::Stdout => OutputDestination::Stdout,
        OutputTarget::File(path) | OutputTarget::Directory(path) => {
            OutputDestination::File(path.clone())
        }
    };

    let plan = OutputPlan {
        destination,
        description: Some(format!("{} files concatenated", files.len())),
        content: output,
    };

    apply_output_plan(plan, cli)
}

/// Processes a single file and writes to the output directory.
fn process_file(input: &Path, out_dir: &Path, cli: &Cli) -> Result<(), Error> {
    let out_name = input.file_stem().context(InvalidFilenameSnafu)?;
    let out_path = out_dir.join(format!("{}.md", out_name.to_string_lossy()));

    if !cli.dry_run && out_path.exists() && !cli.force {
        eprintln!(
            "Skipping {} (already exists, use --force to overwrite)",
            out_path.display()
        );
        return Ok(());
    }

    let chat = load_chat(input)?;

    let opts = make_render_options(cli);
    let markdown = renderer::render_chat(&chat, &opts);

    let plan = OutputPlan {
        destination: OutputDestination::File(out_path),
        description: None,
        content: markdown,
    };

    apply_output_plan(plan, cli)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to create args from a string for testing.
    fn args(s: &str) -> impl Iterator<Item = &str> {
        s.split_whitespace()
    }

    // =========================================================================
    // Pure argument parsing tests (no I/O)
    // =========================================================================

    #[test]
    fn parses_output_to_stdout() {
        let cli = parse_args_from(args("input.json -o -")).unwrap();
        assert!(matches!(cli.output, OutputTarget::Stdout));
    }

    #[test]
    fn parses_output_to_directory() {
        let cli = parse_args_from(args("input.json -o out/")).unwrap();
        assert!(matches!(cli.output, OutputTarget::Directory(_)));
    }

    #[test]
    fn error_on_missing_output() {
        let err = parse_args_from(args("input.json")).unwrap_err();
        assert!(matches!(err, Error::MissingOutput));
    }

    #[test]
    fn error_on_invalid_heading_offset() {
        let err = parse_args_from(args("-o - --heading-offset 7 x.json")).unwrap_err();
        assert!(matches!(err, Error::InvalidHeadingOffset));
    }

    #[test]
    fn concat_converts_directory_to_file_target() {
        let cli = parse_args_from(args("--concat -o out.md input.json")).unwrap();
        assert!(matches!(cli.output, OutputTarget::File(_)));
    }

    #[test]
    fn verbose_enables_show_tools() {
        let cli = parse_args_from(args("-v -o - x.json")).unwrap();
        assert!(matches!(cli.tools, renderer::Visibility::Shown));
    }

    #[test]
    fn last_flag_wins() {
        let cli = parse_args_from(args("--show-model --hide-model -o - x.json")).unwrap();
        assert!(matches!(cli.model, renderer::Visibility::Hidden));
    }

    #[test]
    fn show_edits_flag_parsed() {
        let cli = parse_args_from(args("--show-edits -o - x.json")).unwrap();
        assert!(matches!(cli.edits, renderer::EditDisplay::WithCode));
    }

    #[test]
    fn local_time_flag_parsed() {
        let cli = parse_args_from(args("--local-time -o - x.json")).unwrap();
        assert!(matches!(
            cli.timestamps,
            renderer::TimestampDisplay::Zoned(renderer::TimestampZone::Local)
        ));
    }

    #[test]
    fn errors_on_conflicting_timestamp_flags() {
        let err = parse_args_from(args("--local-time --utc-time -o - x.json")).unwrap_err();
        assert!(matches!(err, Error::ConflictingTimestampFlags { .. }));
    }

    #[test]
    fn timestamps_both_conflicts_with_specific_zone() {
        let err = parse_args_from(args("--timestamps-both --local-time -o - x.json")).unwrap_err();
        assert!(matches!(err, Error::ConflictingTimestampFlags { .. }));
    }

    #[test]
    fn both_timezones_flag_parsed() {
        let cli = parse_args_from(args("--timestamps-both -o - x.json")).unwrap();
        assert!(matches!(
            cli.timestamps,
            renderer::TimestampDisplay::Zoned(renderer::TimestampZone::Both)
        ));
    }

    #[test]
    fn compact_disables_all_metadata() {
        let cli = parse_args_from(args("--compact -o - x.json")).unwrap();
        assert!(matches!(cli.model, renderer::Visibility::Hidden));
        assert!(matches!(cli.agent, renderer::Visibility::Hidden));
        assert!(matches!(cli.context, renderer::Visibility::Hidden));
        assert!(matches!(cli.tools, renderer::Visibility::Hidden));
        assert!(matches!(cli.timestamps, renderer::TimestampDisplay::Hidden));
        assert!(matches!(cli.edits, renderer::EditDisplay::SummaryOnly));
    }

    #[test]
    fn compact_can_be_overridden() {
        let cli = parse_args_from(args("--compact --show-model --show-edits -o - x.json")).unwrap();
        // Last flag wins: show-model after compact re-enables it
        assert!(matches!(cli.model, renderer::Visibility::Shown));
        assert!(matches!(cli.edits, renderer::EditDisplay::WithCode));
        // These remain disabled from compact
        assert!(matches!(cli.agent, renderer::Visibility::Hidden));
        assert!(matches!(cli.context, renderer::Visibility::Hidden));
    }

    #[test]
    fn dry_run_flag_parsed() {
        let cli = parse_args_from(args("-n -o - x.json")).unwrap();
        assert!(cli.dry_run);

        let cli = parse_args_from(args("--dry-run -o - x.json")).unwrap();
        assert!(cli.dry_run);
    }

    #[test]
    fn force_flag_parsed() {
        let cli = parse_args_from(args("-f -o - x.json")).unwrap();
        assert!(cli.force);

        let cli = parse_args_from(args("--force -o - x.json")).unwrap();
        assert!(cli.force);
    }

    #[test]
    fn quiet_flag_parsed() {
        let cli = parse_args_from(args("-q -o - x.json")).unwrap();
        assert!(cli.quiet);

        let cli = parse_args_from(args("--quiet -o - x.json")).unwrap();
        assert!(cli.quiet);
    }

    #[test]
    fn valid_heading_offset_parsed() {
        for offset in 0..=5 {
            let cli =
                parse_args_from(args(&format!("--heading-offset {offset} -o - x.json"))).unwrap();
            assert_eq!(cli.heading_offset, offset);
        }
    }

    #[test]
    fn flags_default_to_false() {
        let cli = parse_args_from(args("-o - x.json")).unwrap();
        assert!(!cli.dry_run);
        assert!(!cli.force);
        assert!(!cli.quiet);
        assert_eq!(cli.heading_offset, 0);
    }

    // =========================================================================
    // Pure rendering tests (no I/O)
    // =========================================================================

    #[test]
    fn render_concat_joins_with_separator() {
        let chat1 = parser::parse_chat(r#"{"responderUsername":"Copilot","requests":[]}"#).unwrap();
        let chat2 = parser::parse_chat(r#"{"responderUsername":"Copilot","requests":[]}"#).unwrap();

        let output = render_concat(&[chat1, chat2], &renderer::RenderOptions::default());

        assert_eq!(output.matches("# Copilot Chat").count(), 2);
        assert!(output.contains("\n---\n\n"));
    }

    // =========================================================================
    // Pure file collection tests (no filesystem access)
    // =========================================================================

    #[test]
    fn collects_direct_files_first() {
        let inputs = vec![
            InputDescriptor::File(PathBuf::from("/direct/b.json")),
            InputDescriptor::Directory {
                contents: vec![
                    PathBuf::from("/dir/a.json"),
                    PathBuf::from("/dir/nested/c.json"),
                ],
            },
        ];

        let files = collect_from_descriptors(&inputs);

        assert_eq!(
            files,
            vec![
                PathBuf::from("/direct/b.json"),
                PathBuf::from("/dir/a.json"),
                PathBuf::from("/dir/nested/c.json"),
            ]
        );
    }

    #[test]
    fn deduplicates_files() {
        let inputs = vec![
            InputDescriptor::File(PathBuf::from("/a.json")),
            InputDescriptor::Directory {
                contents: vec![
                    PathBuf::from("/a.json"), // duplicate of direct file
                    PathBuf::from("/b.json"),
                ],
            },
            InputDescriptor::File(PathBuf::from("/b.json")), // duplicate from directory
        ];

        let files = collect_from_descriptors(&inputs);

        assert_eq!(
            files,
            vec![PathBuf::from("/a.json"), PathBuf::from("/b.json"),]
        );
    }

    #[test]
    fn filters_non_json_from_directories() {
        let inputs = vec![InputDescriptor::Directory {
            contents: vec![
                PathBuf::from("/dir/a.json"),
                PathBuf::from("/dir/notes.txt"),
                PathBuf::from("/dir/data.json"),
                PathBuf::from("/dir/readme.md"),
            ],
        }];

        let files = collect_from_descriptors(&inputs);

        assert_eq!(
            files,
            vec![
                PathBuf::from("/dir/a.json"),
                PathBuf::from("/dir/data.json"),
            ]
        );
    }

    #[test]
    fn direct_files_not_filtered_by_extension() {
        // Direct file inputs are trusted - user explicitly named them
        let inputs = vec![
            InputDescriptor::File(PathBuf::from("/explicit.txt")),
            InputDescriptor::File(PathBuf::from("/also.md")),
        ];

        let files = collect_from_descriptors(&inputs);

        assert_eq!(
            files,
            vec![PathBuf::from("/explicit.txt"), PathBuf::from("/also.md"),]
        );
    }

    #[test]
    fn preserves_input_order() {
        let inputs = vec![
            InputDescriptor::File(PathBuf::from("/z.json")),
            InputDescriptor::File(PathBuf::from("/a.json")),
            InputDescriptor::Directory {
                contents: vec![PathBuf::from("/dir/m.json"), PathBuf::from("/dir/b.json")],
            },
        ];

        let files = collect_from_descriptors(&inputs);

        // Order matches input order, not alphabetical
        assert_eq!(
            files,
            vec![
                PathBuf::from("/z.json"),
                PathBuf::from("/a.json"),
                PathBuf::from("/dir/m.json"),
                PathBuf::from("/dir/b.json"),
            ]
        );
    }

    #[test]
    fn show_timestamps_flag_enables_timestamps() {
        let cli = parse_args_from(args("--show-timestamps -o - x.json")).unwrap();
        assert!(matches!(
            cli.timestamps,
            renderer::TimestampDisplay::Zoned(renderer::TimestampZone::Utc)
        ));
    }

    #[test]
    fn hide_timestamps_flag_disables_timestamps() {
        let cli = parse_args_from(args("--hide-timestamps -o - x.json")).unwrap();
        assert!(matches!(cli.timestamps, renderer::TimestampDisplay::Hidden));

        // Show then hide - last wins
        let cli = parse_args_from(args("--show-timestamps --hide-timestamps -o - x.json")).unwrap();
        assert!(matches!(cli.timestamps, renderer::TimestampDisplay::Hidden));
    }

    #[test]
    fn hide_tools_flag_parsed() {
        let cli = parse_args_from(args("--hide-tools -o - x.json")).unwrap();
        assert!(matches!(cli.tools, renderer::Visibility::Hidden));
    }

    #[test]
    fn errors_on_unknown_argument() {
        let err = parse_args_from(args("--unknown-flag -o - x.json")).unwrap_err();
        assert!(matches!(err, Error::ParseArgs { .. }));
    }
}
