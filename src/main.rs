use std::ffi::OsStr;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{ArgAction, ArgGroup, Parser};
use monkeyc_fmt::{Options, format};
use tempfile::NamedTempFile;

const EXIT_CHANGES_NEEDED: u8 = 1;
const EXIT_ERROR: u8 = 2;

/// Format Garmin Monkey C source code.
#[derive(Debug, Parser)]
#[command(
    version,
    about,
    long_about = "Format Garmin Monkey C source code.\n\nWith no FILE, or when FILE is '-', input is read from standard input. Formatted output is written to standard output unless --check or --write is used. Only one input may be used when writing to standard output.",
    after_help = "Exit status:\n  0  success (and, with --check, all inputs were already formatted)\n  1  --check found input that needs formatting\n  2  invalid arguments or an input, formatting, or write error\n\nExamples:\n  monkeyc-fmt source/App.mc\n  monkeyc-fmt --write source/App.mc source/View.mc\n  monkeyc-fmt --check source/*.mc\n\nFormatting changes whitespace only. Comments, literals, and syntax are preserved and verified before output. Line endings outside tokens become LF. Width is a soft target; unsplittable tokens may exceed it. No Garmin SDK is required.",
    group = ArgGroup::new("mode").args(["check", "write"]).multiple(false)
)]
struct Cli {
    /// Check formatting without changing files
    #[arg(long, action = ArgAction::SetTrue)]
    check: bool,

    /// Format files in place using atomic replacement
    #[arg(short, long, action = ArgAction::SetTrue)]
    write: bool,

    /// Soft target for formatted line width
    #[arg(long, default_value_t = 100, value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(20..))]
    line_width: usize,

    /// Number of spaces in each indentation level
    #[arg(long, default_value_t = 4, value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..=16))]
    indent_width: usize,

    /// Input files; use '-' for standard input
    #[arg(value_name = "FILE")]
    files: Vec<PathBuf>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        RunOutcome::Success => ExitCode::SUCCESS,
        RunOutcome::ChangesNeeded => ExitCode::from(EXIT_CHANGES_NEEDED),
        RunOutcome::Error => ExitCode::from(EXIT_ERROR),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RunOutcome {
    Success,
    ChangesNeeded,
    Error,
}

fn run(mut cli: Cli) -> RunOutcome {
    if cli.files.is_empty() {
        cli.files.push(PathBuf::from("-"));
    }

    let stdin_count = cli
        .files
        .iter()
        .filter(|path| path.as_os_str() == OsStr::new("-"))
        .count();

    if cli.write && stdin_count != 0 {
        diagnostic("--write cannot be used with standard input");
        return RunOutcome::Error;
    }
    if stdin_count > 1 || (stdin_count == 1 && cli.files.len() > 1) {
        diagnostic("standard input ('-') must be the only input");
        return RunOutcome::Error;
    }
    if !cli.check && !cli.write && cli.files.len() != 1 {
        diagnostic("multiple inputs require --check or --write");
        return RunOutcome::Error;
    }

    let options = Options {
        line_width: cli.line_width,
        indent_width: cli.indent_width,
    };
    let mut had_error = false;
    let mut needs_formatting = false;

    for path in &cli.files {
        let label = path_label(path);
        let original_metadata = if cli.write {
            match writable_file_metadata(path) {
                Ok(metadata) => Some(metadata),
                Err(error) => {
                    diagnostic(&format!("{label}: {error}"));
                    had_error = true;
                    continue;
                }
            }
        } else {
            None
        };
        let source = match read_source(path) {
            Ok(source) => source,
            Err(error) => {
                diagnostic(&format!("{label}: {error}"));
                had_error = true;
                continue;
            }
        };
        let formatted = match format(&source, &options) {
            Ok(formatted) => formatted,
            Err(error) => {
                diagnostic(&format!("{label}: {error}"));
                had_error = true;
                continue;
            }
        };

        if cli.check {
            if formatted != source {
                diagnostic(&format!("{label}: needs formatting"));
                needs_formatting = true;
            }
        } else if let Some(metadata) = original_metadata.as_ref() {
            if formatted != source {
                if let Err(error) = replace_file(path, formatted.as_bytes(), metadata) {
                    diagnostic(&format!("{label}: {error}"));
                    had_error = true;
                }
            }
        } else if let Err(error) = write_stdout(formatted.as_bytes()) {
            if error.kind() != io::ErrorKind::BrokenPipe {
                diagnostic(&format!("standard output: {error}"));
                had_error = true;
            }
        }
    }

    if had_error {
        RunOutcome::Error
    } else if needs_formatting {
        RunOutcome::ChangesNeeded
    } else {
        RunOutcome::Success
    }
}

fn path_label(path: &Path) -> String {
    if path.as_os_str() == OsStr::new("-") {
        "standard input".to_owned()
    } else {
        path.display().to_string()
    }
}

fn read_source(path: &Path) -> Result<String, String> {
    let bytes = if path.as_os_str() == OsStr::new("-") {
        let mut bytes = Vec::new();
        io::stdin()
            .read_to_end(&mut bytes)
            .map_err(|error| format!("could not read: {error}"))?;
        bytes
    } else {
        fs::read(path).map_err(|error| format!("could not read: {error}"))?
    };

    String::from_utf8(bytes).map_err(|error| {
        format!(
            "input is not valid UTF-8 (invalid byte at offset {})",
            error.utf8_error().valid_up_to()
        )
    })
}

fn writable_file_metadata(path: &Path) -> Result<fs::Metadata, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("could not inspect before writing: {error}"))?;
    if metadata.file_type().is_symlink() {
        return Err("refusing to overwrite a symbolic link".to_owned());
    }
    if !metadata.file_type().is_file() {
        return Err("refusing to overwrite a non-regular file".to_owned());
    }
    Ok(metadata)
}

fn replace_file(
    path: &Path,
    contents: &[u8],
    original_metadata: &fs::Metadata,
) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = NamedTempFile::new_in(parent)
        .map_err(|error| format!("could not create temporary file: {error}"))?;
    temporary
        .as_file()
        .set_permissions(original_metadata.permissions())
        .map_err(|error| format!("could not preserve permissions: {error}"))?;
    temporary
        .write_all(contents)
        .map_err(|error| format!("could not write temporary file: {error}"))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| format!("could not sync temporary file: {error}"))?;

    // Refuse to replace a path that changed after it was read.
    let current = writable_file_metadata(path)
        .map_err(|error| format!("file changed while formatting: {error}"))?;
    if !same_file_version(original_metadata, &current) {
        return Err("file changed while formatting; refusing to overwrite it".to_owned());
    }

    temporary
        .persist(path)
        .map_err(|error| format!("could not atomically replace file: {}", error.error))?;
    Ok(())
}

#[cfg(unix)]
fn same_file_version(original: &fs::Metadata, current: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;

    original.dev() == current.dev()
        && original.ino() == current.ino()
        && original.len() == current.len()
        && original.mtime() == current.mtime()
        && original.mtime_nsec() == current.mtime_nsec()
}

#[cfg(not(unix))]
fn same_file_version(original: &fs::Metadata, current: &fs::Metadata) -> bool {
    original.len() == current.len() && original.modified().ok() == current.modified().ok()
}

fn write_stdout(contents: &[u8]) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    stdout.write_all(contents)
}

fn diagnostic(message: &str) {
    let _ = writeln!(io::stderr().lock(), "monkeyc-fmt: {message}");
}
