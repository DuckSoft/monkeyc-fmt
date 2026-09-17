# monkeyc-fmt

`monkeyc-fmt` is a syntax-preserving formatter for [Garmin Monkey C](https://developer.garmin.com/connect-iq/monkey-c/). It parses source with Tree-sitter, changes whitespace between syntax tokens, and verifies that the formatted result has an equivalent syntax tree before returning it.

Comments, literal contents, parentheses, and punctuation are preserved. The Garmin Connect IQ SDK is not required.

## Installation

Building requires the stable Rust toolchain and a native C compiler/toolchain for the Tree-sitter parser.

Install from crates.io:

```console
cargo install --locked monkeyc-fmt
```

Alternatively, clone and install from source (requires Git):

```console
git clone https://github.com/DuckSoft/monkeyc-fmt.git
cd monkeyc-fmt
cargo install --locked --path .
```

To build without installing:

```console
cargo build --locked --release
./target/release/monkeyc-fmt --help
```

## Usage

With no file, or with `-`, `monkeyc-fmt` reads standard input and writes formatted source to standard output:

```console
$ printf 'var x=1;' | monkeyc-fmt
var x = 1;
```

A single file is also formatted to standard output without changing the file:

```console
monkeyc-fmt source/App.mc
```

Format one or more files in place with `--write` (or `-w`):

```console
monkeyc-fmt --write source/App.mc source/View.mc
```

Check files without changing them, for example in CI:

```console
monkeyc-fmt --check source/*.mc
```

`--check` prints `<path>: needs formatting` to standard error for each file that would change.

### Options

| Option | Meaning |
| --- | --- |
| `--check` | Check formatting without changing files. Accepts multiple files. |
| `-w`, `--write` | Format files in place using atomic replacement. Accepts multiple files and does not write formatted source to standard output. |
| `--line-width <WIDTH>` | Soft target for line width. Default: `100`; minimum: `20`. Unsplittable comments and literals can exceed it. |
| `--indent-width <WIDTH>` | Spaces per indentation level. Default: `4`; accepted range: `1`–`16`. |
| `-h`, `--help` | Print help. |
| `-V`, `--version` | Print the version. |

Plain standard-output mode accepts exactly one input. Multiple inputs require `--check` or `--write`; standard input cannot be combined with other inputs, and `--write` rejects standard input.

### Exit status

| Status | Meaning |
| --- | --- |
| `0` | Success; with `--check`, every input was already formatted. |
| `1` | `--check` found input that needs formatting. |
| `2` | Invalid arguments, or an input, syntax, formatting, or write error. |

## Formatting and safety

- Only whitespace between syntax tokens is changed; token text and syntax are retained.
- The result is parsed again and compared with the original syntax tree. Unsafe output is refused.
- Invalid syntax and invalid UTF-8 are rejected rather than partially formatted.
- Output uses LF between tokens and one final LF for nonempty input. Empty or whitespace-only input produces empty output. Line endings inside comments and literals are untouched.
- Line width is a soft target: literals, comments, and other unsplittable tokens are not rewritten merely to meet it.
- Syntax nested beyond 256 levels is rejected to avoid stack exhaustion.
- In-place writes preserve file permissions, replace files atomically, reject symbolic links and non-regular files, and refuse to overwrite a file that changed while it was being formatted.

## Development

The checked-in toolchain configuration uses stable Rust with `rustfmt` and Clippy. Run the same checks as CI:

```console
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked --release
```

CI runs these checks on Linux, macOS, and Windows.

## License

This project is released into the public domain under [The Unlicense](LICENSE).
