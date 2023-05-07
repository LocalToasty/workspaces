use chrono::Duration;
use serde::{Deserialize, Deserializer};
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub filesystems: HashMap<String, Filesystem>,
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
    Ok(Duration::days(i64::from(days)))
}
