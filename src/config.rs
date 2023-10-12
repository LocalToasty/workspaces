use chrono::Duration;
use serde::{Deserialize, Deserializer};
use std::collections::HashMap;
use std::path::PathBuf;

/// Path of the configuration file
pub const CONFIG_PATH: &str = "/etc/workspaces/workspaces.toml";

#[derive(Debug, Deserialize)]
pub struct Config {
    /// Default filesystem to use in CLI
    pub default_filesystem: Option<String>,
    /// Workspaces database location
    #[serde(default = "default_db_path")]
    pub db_path: PathBuf,
    /// Workspace filesystem definitions
    #[serde(default)]
    pub filesystems: HashMap<String, Filesystem>,
}

fn default_db_path() -> PathBuf {
    // The >=v0.3 default location.  If such a file exist, we are going to take this one
    let path = PathBuf::from("/usr/local/lib/workspaces/workspaces.db");
    if path.exists() {
        return path;
    }
    // v0.2 database location.  We'll take this one if it exists
    let path = PathBuf::from("/usr/local/share/workspaces/workspaces.db");
    if path.exists() {
        eprintln!(
            "DEPRECATION WARNING: the workspaces default database location has been moved from \
            `/usr/local/share/workspaces/workspaces.db` \
            to `/usr/local/lib/workspaces/workspaces.db`.  \
            Please either move your database to the new location, or manually specify it in `{}`",
            CONFIG_PATH
        );
        return path;
    }

    PathBuf::from("/usr/local/lib/workspaces/workspaces.db")
}

/// A filesystem workpsaces can be created in
#[derive(Debug, Deserialize)]
pub struct Filesystem {
    /// ZFS filesystem / volume which will act as the root for the datasets
    pub root: String,
    /// Maximum number of days a workspace may exist
    #[serde(deserialize_with = "from_days")]
    pub max_duration: Duration,
    /// Days after which an expired dataset will be removed
    #[serde(deserialize_with = "from_days")]
    pub expired_retention: Duration,
    /// Whether datasets can be created / extended
    #[serde(default)]
    pub disabled: bool,
}

fn from_days<'de, D>(deserializer: D) -> Result<Duration, D::Error>
where
    D: Deserializer<'de>,
{
    let days: i64 = Deserialize::deserialize(deserializer)?;
    Ok(Duration::days(days))
}
