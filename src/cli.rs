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
    #[clap(alias = "c")]
    Create {
        /// Name of the workspace
        ///
        /// Must entirely consist of the characters [A-Za-z0-9_-].
        #[arg(value_parser = parse_pathsafe)]
        workspace_name: String,

        /// Duration in days to extend the workspace to
        ///
        /// Must be less or equal to the DURATION given in `workspaces filesystems`.
        #[arg(short, long, value_parser = |arg: &str| -> Result<Duration, ParseIntError> {Ok(Duration::days(arg.parse()?))})]
        duration: Duration,

        /// User the workspace belongs to
        #[arg(short, long, default_value_t = get_current_username().unwrap().to_string_lossy().to_string(), value_parser = parse_pathsafe)]
        user: String,

        /// Filesystem to create the workspace in
        #[arg(short, long = "filesystem")]
        filesystem_name: Option<String>,
    },
    /// List workspaces
    #[clap(alias = "ls")]
    List,
    /// Postpone the expiry date of an already existing workspace
    #[clap(alias = "ex")]
    Extend {
        /// Name of the workspace
        #[arg(value_parser = parse_pathsafe)]
        name: String,

        /// Duration in days to extend the workspace until
        ///
        /// If this is fewer than the current days until expiry,
        /// no action will be taken.
        #[arg(short, long, value_parser = |arg: &str| -> Result<Duration, ParseIntError> {Ok(Duration::days(arg.parse()?))})]
        duration: Duration,

        /// User the workspace belongs to
        #[arg(short, long, default_value_t = get_current_username().unwrap().to_string_lossy().to_string(), value_parser = parse_pathsafe)]
        user: String,

        /// Filesystem of the workspace
        #[arg(short, long = "filesystem")]
        filesystem_name: Option<String>,
    },
    /// Expire a workspace
    Expire {
        /// Name of the workspace
        #[arg(value_parser = parse_pathsafe)]
        name: String,
        /// User the workspace belongs to
        #[arg(short, long, default_value_t = get_current_username().unwrap().to_string_lossy().to_string(), value_parser = parse_pathsafe)]
        user: String,

        /// Filesystem of the workspace
        #[arg(short, long = "filesystem")]
        filesystem_name: Option<String>,

        /// Delete this dataset on next cleanup
        ///
        /// No deletion will take place until the next time `clean` is called.
        /// Be aware that this may happen due to another user / cronjob.
        #[arg(long = "terminally")]
        delete_on_next_clean: bool,
    },
    /// List all existing filesystems
    #[clap(alias = "fi")]
    Filesystems,
    /// Clean up workspaces which not been extended in a while
    ///
    /// This will delete all workspaces marked as `deleted soon` in `workspaces list`,
    /// including other users' workspaces.
    Clean,
}

/// String contains characters which are not [A-Za-z0-9_-]
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

/// Ensures string only contains the characters [A-Za-z0-9_-]
fn parse_pathsafe(ident: &str) -> Result<String, NotPathsafeError> {
    if ident.len() > 0
        && ident
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
