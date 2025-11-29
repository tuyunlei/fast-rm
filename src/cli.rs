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
    /// Deprecated: use --scan-threads and --delete-threads for fine-grained control
    #[clap(short = 'j', long = "threads")]
    pub threads: Option<usize>,

    /// Number of threads for scanning (defaults to number of CPU cores)
    /// Takes precedence over --threads if both are specified
    #[clap(long = "scan-threads")]
    pub scan_threads: Option<usize>,

    /// Number of threads for deletion (defaults to number of CPU cores)
    /// Takes precedence over --threads if both are specified
    #[clap(long = "delete-threads")]
    pub delete_threads: Option<usize>,

    /// Continue processing even if errors occur
    #[clap(short = 'c', long = "continue-on-error")]
    pub continue_on_error: bool,
}

impl Cli {
    /// Get the number of scanner threads to use
    /// Priority: --scan-threads > --threads > CPU cores
    pub fn get_scan_threads(&self) -> usize {
        self.scan_threads
            .or(self.threads)
            .unwrap_or_else(num_cpus::get)
    }

    /// Get the number of deleter threads to use
    /// Priority: --delete-threads > --threads > CPU cores
    pub fn get_delete_threads(&self) -> usize {
        self.delete_threads
            .or(self.threads)
            .unwrap_or_else(num_cpus::get)
    }
}
