use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[clap(
    author,
    version,
    about = "A fast, concurrent file and directory remover.",
    long_about = None
)]
pub struct Cli {
    /// Files or directories to remove
    #[clap(required = true, num_args = 1..)]
    pub paths: Vec<PathBuf>,

    /// Verbosity level: -v for standard, -vv for detailed
    #[clap(short = 'v', long = "verbose", action = clap::ArgAction::Count)]
    pub verbosity: u8,

    /// Do not actually remove anything, just show what would be done
    #[clap(short = 'n', long = "dry-run")]
    pub dry_run: bool,

    /// Number of threads to use (defaults to number of CPU cores)
    #[clap(short = 'j', long = "threads")]
    pub threads: Option<usize>,

    /// Continue processing even if errors occur
    #[clap(short = 'c', long = "continue-on-error")]
    pub continue_on_error: bool,
}
