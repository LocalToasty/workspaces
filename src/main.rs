use chrono::{DateTime, Duration, Local};
use clap::Parser;
use rusqlite::Connection;
use std::{collections::HashMap, fs, io, process::Command};
use users::{get_current_uid, get_current_username};

const DB_PATH: &str = "/usr/local/share/workspaces.db";
const CONFIG_PATH: &str = "/usr/local/etc/workspaces.toml";

mod cli {
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
}

mod config {
    use chrono::Duration;
    use serde::{Deserialize, Deserializer};
    use std::collections::HashMap;

    #[derive(Debug, Deserialize)]
    pub struct Config {
        pub filesystems: HashMap<String, Filesystem>,
    }

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
    }

    fn from_days<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let days: i64 = Deserialize::deserialize(deserializer)?;
        Ok(Duration::days(i64::from(days)))
    }
}

fn create(
    conn: &mut Connection,
    filesystem_name: &str,
    filesystem: &config::Filesystem,
    user: &str,
    name: &str,
    duration: &Duration,
) {
    assert!(
        get_current_username().unwrap() == user || get_current_uid() == 0,
        "you are not allowed to execute this operation"
    );

    let transaction = conn.transaction().unwrap();
    transaction
        .execute(
            "INSERT INTO workspaces (filesystem, user, name, expiration_time)
            VALUES (?1, ?2, ?3, ?4)",
            (filesystem_name, user, name, Local::now() + *duration),
        )
        .unwrap();

    // create dataset
    let status = Command::new("zfs")
        .args([
            "create",
            "-p",
            &format!("{}/{}/{}", filesystem.root, user, name),
        ])
        .status()
        .unwrap();
    assert!(status.success(), "failed to create dataset property");

    // get mountpoint
    let mountpoint = zfs_get(
        "mountpoint",
        &format!("{}/{}/{}", filesystem.root, user, name),
    )
    .unwrap();

    let status = Command::new("chmod")
        .args(["750", &mountpoint])
        .status()
        .unwrap();
    assert!(status.success(), "failed to set rights on dataset");

    let status = Command::new("chown")
        .args([&format!("{}:{}", user, user), &mountpoint])
        .status()
        .unwrap();
    assert!(status.success(), "failed to change owner on dataset");
    transaction.commit().unwrap();

    println!("Created workspace at {}", mountpoint);
}

fn zfs_get(attribute: &str, volume: &str) -> io::Result<String> {
    //TODO remove unwraps
    let output = Command::new("zfs")
        .args(["get", attribute, volume])
        .output()?;
    assert!(output.status.success());
    let info_line = String::from_utf8(output.stdout).unwrap();
    let value = info_line
        .lines()
        .nth(1)
        .unwrap()
        .split_whitespace()
        .nth(2)
        .unwrap()
        .to_string();
    Ok(value)
}

#[derive(Debug)]
struct WorkspacesRow {
    filesystem_name: String,
    user: String,
    name: String,
    expiration_time: DateTime<Local>,
}

fn list(conn: &Connection, filesystems: &HashMap<String, config::Filesystem>) {
    let mut statement = conn
        .prepare("SELECT filesystem, user, name, expiration_time FROM workspaces")
        .unwrap();
    let workspace_iter = statement
        .query_map([], |row| {
            Ok(WorkspacesRow {
                filesystem_name: row.get(0)?,
                user: row.get(1)?,
                name: row.get(2)?,
                expiration_time: row.get(3)?,
            })
        })
        .unwrap();

    println!(
        "{:<16}{:<16}{:<16}{:<16}{:<8}{}",
        "NAME", "USER", "FILESYSTEM", "EXPIRY DATE", "SIZE", "MOUNTPOINT"
    );
    for workspace in workspace_iter {
        let workspace = workspace.unwrap();
        let dataset_info = Command::new("zfs")
            .args([
                "get",
                "mountpoint,logicalreferenced",
                &format!(
                    "{}/{}/{}",
                    workspace.filesystem_name, workspace.user, workspace.name
                ),
            ])
            .output()
            .unwrap();

        if !dataset_info.status.success() {
            continue;
        }

        print!(
            "{:<15}\t{:<15}\t{:<15}",
            workspace.name, workspace.user, workspace.filesystem_name
        );

        if Local::now() > workspace.expiration_time {
            print!(
                "\tdeleted in {:>2}d",
                (workspace.expiration_time
                    + filesystems[&workspace.filesystem_name].expired_retention
                    - Local::now())
                .num_days()
            );
        } else {
            print!(
                "\texpires in {:>2}d",
                (workspace.expiration_time - Local::now()).num_days()
            );
        }

        let info = String::from_utf8(dataset_info.stdout).unwrap();
        let info = info
            .lines()
            .skip(1)
            .map(|line| {
                let parts = line.split_whitespace().collect::<Vec<_>>();
                (parts[1], parts[2])
            })
            .collect::<HashMap<_, _>>();

        println!("\t{:>6}\t{}", info["logicalreferenced"], info["mountpoint"]);
    }
}

fn extend(
    conn: &Connection,
    filesystem: &config::Filesystem,
    user: &str,
    name: &str,
    duration: &Duration,
) {
    assert!(
        get_current_username().unwrap() == user || get_current_uid() == 0,
        "you are not allowed to execute this operation"
    );
    assert!(
        duration <= &filesystem.max_duration,
        "duration has to be shorter than {}",
        filesystem.max_duration
    );

    let rows_updated = conn
        .execute(
            "UPDATE workspaces
            SET expiration_time = MAX(expiration_time, ?1)
            WHERE filesystem = ?2
                AND user = ?3
                AND name = ?4",
            (Local::now() + *duration, &filesystem.root, user, name),
        )
        .unwrap();
    assert_eq!(rows_updated, 1);

    let status = Command::new("zfs")
        .args([
            "set",
            "readonly=off",
            &format!("{}/{}/{}", filesystem.root, user, name),
        ])
        .status()
        .unwrap();
    assert!(status.success(), "failed to update readonly property");
}

fn expire(
    conn: &Connection,
    filesystem_name: &str,
    filesystem: &config::Filesystem,
    user: &str,
    name: &str,
    terminally: bool,
) {
    assert!(
        get_current_username().unwrap() == user || get_current_uid() == 0,
        "you are not allowed to execute this operation"
    );

    let expiration_time = if terminally {
        // set the expiration time sufficiently far in the past
        // for it to get cleaned up soon
        Local::now() - filesystem.expired_retention
    } else {
        Local::now()
    };
    let rows_updated = conn
        .execute(
            "UPDATE workspaces
            SET expiration_time = ?1
            WHERE filesystem = ?2
                AND user = ?3
                AND name = ?4",
            (expiration_time, filesystem_name, user, name),
        )
        .unwrap();
    assert!(
        rows_updated == 1,
        "could not find a matching filesystem/user/name combination: {}/{}/{}",
        filesystem_name,
        user,
        name
    );

    let status = Command::new("zfs")
        .args([
            "set",
            "readonly=on",
            &format!("{}/{}/{}", filesystem.root, user, name),
        ])
        .status()
        .unwrap();
    assert!(status.success(), "failed to update readonly property");
}

fn filesystems(filesystems: &HashMap<String, config::Filesystem>) {
    println!("{:<15}\t{:<7}\t{:<16}", "FILESYSTEM", "FREE", "DURATION");
    filesystems.iter().for_each(|(name, info)| {
        let available = zfs_get("available", name).unwrap();
        println!(
            "{:<15}\t{:>6}\t{:>7}d",
            name,
            available,
            info.max_duration.num_days()
        );
    });
}

fn clean(conn: &mut Connection, filesystems: &HashMap<String, config::Filesystem>) {
    let transaction = conn.transaction().unwrap();
    {
        let mut statement = transaction
            .prepare(
                "SELECT filesystem, user, name, expiration_time
                    FROM workspaces
                    WHERE expiration_time < ?1",
            )
            .unwrap();
        let mut rows = statement.query([Local::now()]).unwrap();
        while let Some(row) = rows.next().unwrap() {
            let filesystem_name: String = row.get(0).unwrap();
            let user: String = row.get(1).unwrap();
            let name: String = row.get(2).unwrap();
            let expiration_time: DateTime<Local> = row.get(3).unwrap();

            let filesystem = &filesystems
                .get(&filesystem_name)
                .expect("unknown filesystem name");
            if expiration_time < Local::now() - filesystem.expired_retention {
                let status = Command::new("zfs")
                    .args(["destroy", &format!("{}/{}/{}", filesystem.root, user, name)])
                    .status()
                    .unwrap();
                assert!(status.success(), "failed to delete dataset");
                transaction
                    .execute(
                        "DELETE FROM workspaces
                            WHERE filesystem = ?1
                                AND user = ?2
                                AND name = ?3",
                        (filesystem_name, user, name),
                    )
                    .unwrap();
            } else {
                let status = Command::new("zfs")
                    .args([
                        "set",
                        "readonly=on",
                        &format!("{}/{}/{}", filesystem.root, user, name),
                    ])
                    .status()
                    .unwrap();
                assert!(status.success(), "failed to update readonly property");
            }
        }
    }
    transaction.commit().unwrap();
}

const UPDATE_DB: &[fn(&mut Connection)] = &[|conn| {
    // Creates initial database
    conn.pragma_update(None, "journal_mode", "WAL").unwrap();
    let transaction = conn.transaction().unwrap();
    transaction
        .execute(
            "CREATE TABLE workspaces (
            filesystem      TEXT     NOT NULL,
            user            TEXT     NOT NULL,
            name            TEXT     NOT NULL,
            expiration_time DATETIME NOT NULL,
            UNIQUE(filesystem, user, name)
        )",
            (),
        )
        .unwrap();
    transaction.pragma_update(None, "user_version", 1).unwrap();
    transaction.commit().unwrap();
}];
const NEWEST_DB_VERSION: usize = UPDATE_DB.len();

fn main() {
    let toml_str = fs::read_to_string(CONFIG_PATH).expect("could not find configuration file");
    let config: config::Config = toml::from_str(&toml_str).unwrap();

    let mut conn = Connection::open(DB_PATH).unwrap();
    let db_version: usize = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert!(
        db_version <= NEWEST_DB_VERSION,
        "database seems to be from a more current version of workspaces"
    );
    UPDATE_DB[db_version..].iter().for_each(|f| f(&mut conn));

    let args = cli::Args::parse();
    match args.command {
        cli::Command::Create {
            filesystem_name,
            name,
            duration,
            user,
        } => create(
            &mut conn,
            &filesystem_name,
            &config
                .filesystems
                .get(&filesystem_name)
                .expect("unknown filesystem name"),
            &user,
            &name,
            &duration,
        ),
        cli::Command::List => list(&conn, &config.filesystems),
        cli::Command::Extend {
            filesystem_name: filesystem,
            name,
            user,
            duration,
        } => extend(
            &mut conn,
            &config.filesystems[&filesystem],
            &user,
            &name,
            &duration,
        ),
        cli::Command::Expire {
            filesystem_name,
            name,
            user,
            delete_on_next_clean,
        } => expire(
            &mut conn,
            &filesystem_name,
            &config
                .filesystems
                .get(&filesystem_name)
                .expect("unknown filesystem name"),
            &user,
            &name,
            delete_on_next_clean,
        ),
        cli::Command::Filesystems => filesystems(&config.filesystems),
        cli::Command::Clean => clean(&mut conn, &config.filesystems),
    }
}
