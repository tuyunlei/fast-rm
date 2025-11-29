use colored::*;
use std::path::Path;
use std::sync::Arc;

use crate::progress::RemoveProgress;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verbosity {
    Simple,
    Standard,
    Detailed,
}

impl Verbosity {
    pub fn from_count(count: u8) -> Self {
        match count {
            0 => Self::Simple,
            1 => Self::Standard,
            _ => Self::Detailed,
        }
    }

    pub fn is_verbose(&self) -> bool {
        matches!(self, Self::Standard | Self::Detailed)
    }
}

#[derive(Debug, Clone)]
pub struct RemoveConfig {
    pub verbosity: Verbosity,
    pub dry_run: bool,
    pub continue_on_error: bool,
    pub progress: Option<Arc<RemoveProgress>>, 
}

impl RemoveConfig {
    pub fn from_cli(cli: &crate::cli::Cli, progress: Option<Arc<RemoveProgress>>) -> Self {
        Self {
            verbosity: Verbosity::from_count(cli.verbosity),
            dry_run: cli.dry_run,
            continue_on_error: cli.continue_on_error,
            progress,
        }
    }

    pub fn log_action(&self, action: &str, action_dry: &str, path: &Path, color: colored::Color) {
        if self.verbosity.is_verbose() || self.dry_run {
            let msg = if self.dry_run { action_dry } else { action };
            println!("  {}{:?}", msg.color(color), path);
        }
    }

    pub fn log_check(&self, path: &Path) {
        if self.verbosity.is_verbose() {
            let msg = if self.dry_run { "Would check " } else { "Checking " };
            println!("  {}{:?}", msg.dimmed(), path);
        }
    }
}

