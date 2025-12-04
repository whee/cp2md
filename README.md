# cp2md

Copyright (C) 2025 Brian Hetro <whee@smaertness.net>

Convert GitHub Copilot chat exports to Markdown.

## Highlights

- Converts GitHub Copilot chat exports (single files or whole directories) to Markdown
- Optional inclusion of tool invocations and timestamps
- CLI-friendly: `cp2md [OPTIONS] --output <OUTPUT> <INPUT>...`

## Installation

Download a prebuilt binary from [GitHub Releases](https://github.com/whee/cp2md/releases), or build from source:

```bash
cargo build --release
./target/release/cp2md --help
```

## Usage

```bash
cp2md [OPTIONS] --output <OUTPUT> <INPUT>...
```

### Arguments

- `<INPUT>...` - One or more JSON files or directories containing Copilot chat exports

### Options

- `-o, --output <OUTPUT>` - Output directory (or file with `--concat`, or `-` for stdout) (required)
- `--concat` - Combine all inputs into a single output file
- `-v, --verbose` - Include tool invocations in output
- `--show-timestamps` - Include timestamps in conversation metadata
- `--no-model` - Hide model ID from output
- `--heading-offset <N>` - Shift heading levels by N (0-5, default: 0)
- `-q, --quiet` - Suppress progress messages
- `-n, --dry-run` - Show what would be processed without writing
- `-f, --force` - Overwrite existing output files
- `-h, --help` - Print help
- `-V, --version` - Print version

### Examples

Convert a single chat export:

```bash
cp2md chat.json -o output/
```

Convert all JSON files in a directory:

```bash
cp2md ~/copilot-exports/ -o markdown/
```

Include tool invocations (searches, file reads, etc.):

```bash
cp2md chat.json -o output/ --verbose
```

Preview what would be converted without writing:

```bash
cp2md ~/copilot-exports/ -o markdown/ --dry-run
```

Combine multiple chats into a single file:

```bash
cp2md chat1.json chat2.json -o combined.md --concat
```

Output to stdout (useful for piping):

```bash
cp2md chat.json -o - | less
```

## Finding Copilot Exports

Export chat history using the VS Code command palette: `Copilot: Export Chat...`

## Output Format

Each input file `foo.json` produces `foo.md` in the output directory. The Markdown includes:

- Model identifier for each user message
- Timestamps (with `--show-timestamps`)
- User prompts and assistant responses
- Tool invocations (with `--verbose`)
- File modification summaries

Example output:

```markdown
# Copilot Chat

## User

*claude-sonnet-4*

How do I reverse a string in Python?

## Assistant

You can reverse a string using slicing: `[::-1]`
```

## License

This program is free software: you can redistribute it and/or modify it under
the terms of version 3 of the GNU General Public License as published by the
Free Software Foundation.

See [LICENSE](LICENSE) for the full text.
