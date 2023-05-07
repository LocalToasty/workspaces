use chrono::Duration;
use clap::{Parser, Subcommand};
use std::{error::Error, fmt, num::ParseIntError};
use users::get_current_username;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Create a new workspace
    Create {
        /// Name of the workspace
        #[arg(short, long, value_parser = parse_pathsafe)]
        name: String,

        /// Duration in days to extend the workspace to
        #[arg(short, long, value_parser = |arg: &str| -> Result<Duration, ParseIntError> {Ok(Duration::days(arg.parse()?))})]
        duration: Duration,

        /// User the workspace belongs to
        #[arg(short, long, default_value_t = get_current_username().unwrap().to_string_lossy().to_string(), value_parser = parse_pathsafe)]
        user: String,

        /// Filesystem to create the workspace in
        #[arg(short, long = "filesystem")]
        filesystem_name: String,
    },
    /// List workspaces
    List,
    /// Postpone the expiry date of a workspace
    Extend {
        /// Name of the workspace
        #[arg(short, long, value_parser = parse_pathsafe)]
        name: String,

        /// Duration in days to extend the workspace to
        #[arg(short, long, value_parser = |arg: &str| -> Result<Duration, ParseIntError> {Ok(Duration::days(arg.parse()?))})]
        duration: Duration,

        /// User the workspace belongs to
        #[arg(short, long, default_value_t = get_current_username().unwrap().to_string_lossy().to_string(), value_parser = parse_pathsafe)]
        user: String,

        /// Filesystem of the workspace
        #[arg(short, long = "filesystem")]
        filesystem_name: String,
    },
    /// Expire a workspace
    Expire {
        /// Name of the workspace
        #[arg(short, long, value_parser = parse_pathsafe)]
        name: String,
        /// User the workspace belongs to
        #[arg(short, long, default_value_t = get_current_username().unwrap().to_string_lossy().to_string(), value_parser = parse_pathsafe)]
        user: String,

        /// Filesystem of the workspace
        #[arg(short, long = "filesystem")]
        filesystem_name: String,

        /// Delete this dataset on next cleanup
        #[arg(long = "terminally")]
        delete_on_next_clean: bool,
    },
    /// List all existing filesystems
    Filesystems,
    /// Clean up workspaces which not been extended in a while
    Clean,
}

#[derive(Debug)]
struct NotPathsafeError {
    str: String,
}
impl fmt::Display for NotPathsafeError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "`{}` must contain only the characters [A-Za-z0-9_-]",
            self.str
        )
    }
}
impl Error for NotPathsafeError {}

fn parse_pathsafe(ident: &str) -> Result<String, NotPathsafeError> {
    if ident
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
    {
        Ok(ident.to_string())
    } else {
        Err(NotPathsafeError {
            str: ident.to_string(),
        })
    }
}
