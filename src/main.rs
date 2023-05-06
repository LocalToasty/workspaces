use chrono::{DateTime, Duration, Local};
use clap::Parser;
use rusqlite::{Connection, Result};
use std::collections::HashMap;
use std::error::Error;
use std::path::Path;
use std::process::Command;
use users::{get_current_username, get_effective_uid};

const DB_PATH: &str = "workspaces.db";

mod cli {
    use chrono::Duration;
    use clap::{Parser, Subcommand};
    use std::num::ParseIntError;
    use users::{get_current_groupname, get_current_username};

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
            #[arg(short, long)]
            name: String,

            /// Filesystem of the workspace
            #[arg(short, long)]
            filesystem: String,

            /// Duration in days to extend the workspace to
            #[arg(short, long, value_parser = |arg: &str| -> Result<Duration, ParseIntError> {Ok(Duration::days(arg.parse()?))})]
            duration: Duration,

            /// User the workspace belongs to
            #[arg(short, long, default_value_t = get_current_username().unwrap().to_string_lossy().to_string())]
            user: String,

            /// Group the workspace belongs to
            #[arg(short, long, default_value_t = get_current_groupname().unwrap().to_string_lossy().to_string())]
            group: String,
        },
        /// List workspaces
        List {},
        /// Extend the expiry date of a workspace
        Extend {
            /// Name of the workspace
            #[arg(short, long)]
            name: String,

            /// User the workspace belongs to
            #[arg(short, long, default_value_t = get_current_username().unwrap().to_string_lossy().to_string())]
            user: String,

            /// Filesystem of the workspace
            #[arg(short, long)]
            filesystem: String,

            /// Duration in days to extend the workspace to
            #[arg(short, long, value_parser = |arg: &str| -> Result<Duration, ParseIntError> {Ok(Duration::days(arg.parse()?))})]
            duration: Duration,
        },
        /// Expire a workspace
        Expire {
            /// Name of the workspace
            #[arg(short, long)]
            name: String,
            /// User the workspace belongs to
            #[arg(short, long, default_value_t = get_current_username().unwrap().to_string_lossy().to_string())]
            user: String,

            /// Filesystem of the workspace
            #[arg(short, long)]
            filesystem: String,
        },
        /// Clean up workspaces which have been expired for too long
        Clean {},
    }
}

fn create(
    filesystem: &str,
    user: &str,
    name: &str,
    group: &str,
    duration: &Duration,
) -> Result<(), Box<dyn Error>> {
    assert!(
        get_current_username().unwrap() == user || get_effective_uid() == 0,
        "you are not allowed to execute this operation"
    );

    let conn = Connection::open(Path::new(DB_PATH))?;
    conn.execute("BEGIN TRANSACTION", ())?;
    conn.execute(
        "INSERT INTO workspaces (filesystem, user, name, \"group\", expiration_time)
            VALUES (?1, ?2, ?3, ?4, ?5)",
        (filesystem, user, name, group, Local::now() + *duration),
    )?;

    // create dataset
    let status = Command::new("zfs")
        .args(["create", "-p", &format!("{}/{}/{}", filesystem, user, name)])
        .status()?;
    assert!(status.success(), "failed to create dataset property");

    // get mountpoint
    let dataset_info = Command::new("zfs")
        .args([
            "get",
            "mountpoint",
            &format!("{}/{}/{}", filesystem, user, name),
        ])
        .output()?;
    assert!(dataset_info.status.success());
    let info = String::from_utf8(dataset_info.stdout)?;
    let mountpoint = info
        .lines()
        .nth(1)
        .unwrap()
        .split_whitespace()
        .nth(2)
        .unwrap();

    let status = Command::new("chmod").args(["750", mountpoint]).status()?;
    assert!(status.success(), "failed to set rights on dataset");

    let status = Command::new("chown")
        .args([&format!("{}:{}", user, group), mountpoint])
        .status()?;
    assert!(status.success(), "failed to change owner/group on dataset");
    conn.execute("COMMIT", ())?;

    println!("Created workspace at {}", mountpoint);
    Ok(())
}

#[derive(Debug)]
struct WorkspacesRow {
    filesystem: String,
    user: String,
    name: String,
    expiration_time: DateTime<Local>,
}

fn list() -> Result<(), Box<dyn Error>> {
    let conn = Connection::open(Path::new(DB_PATH))?;
    let mut statement =
        conn.prepare("SELECT filesystem, user, name, expiration_time FROM workspaces")?;
    let workspace_iter = statement.query_map([], |row| {
        Ok(WorkspacesRow {
            filesystem: row.get(0)?,
            user: row.get(1)?,
            name: row.get(2)?,
            expiration_time: row.get(3)?,
        })
    })?;

    println!(
        "{:<16}{:<16}{:<16}{:<16}{:<8}{}",
        "NAME", "USER", "FILESYSTEM", "EXPIRY DATE", "SIZE", "MOUNTPOINT"
    );
    for workspace in workspace_iter {
        let workspace = workspace?;
        let dataset_info = Command::new("zfs")
            .args([
                "get",
                "mountpoint,logicalreferenced",
                &format!(
                    "{}/{}/{}",
                    workspace.filesystem, workspace.user, workspace.name
                ),
            ])
            .output()?;

        if !dataset_info.status.success() {
            continue;
        }

        print!(
            "{:<15}\t{:<15}\t{:<15}",
            workspace.name, workspace.user, workspace.filesystem
        );

        if Local::now() > workspace.expiration_time {
            print!(
                "\tdeleted in {:>2}d",
                (workspace.expiration_time + Duration::days(30) - Local::now()).num_days()
            );
        } else {
            print!(
                "\texpires in {:>2}d",
                (workspace.expiration_time - Local::now()).num_days()
            );
        }

        let info = String::from_utf8(dataset_info.stdout)?;
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
    Ok(())
}

fn extend(
    filesystem: &str,
    user: &str,
    name: &str,
    duration: &Duration,
) -> Result<(), Box<dyn Error>> {
    assert!(
        get_current_username().unwrap() == user || get_effective_uid() == 0,
        "you are not allowed to execute this operation"
    );

    let conn = Connection::open(Path::new(DB_PATH))?;
    let rows_updated = conn.execute(
        "UPDATE workspaces
            SET expiration_time = MAX(expiration_time, ?1)
            WHERE filesystem = ?2
                AND user = ?3
                AND name = ?4",
        (Local::now() + *duration, filesystem, user, name),
    )?;
    assert_eq!(rows_updated, 1);

    let status = Command::new("zfs")
        .args([
            "set",
            "readonly=off",
            &format!("{}/{}/{}", filesystem, user, name),
        ])
        .status()?;
    assert!(status.success(), "failed to update readonly property");

    Ok(())
}

fn expire(filesystem: &str, user: &str, name: &str) -> Result<(), Box<dyn Error>> {
    assert!(
        get_current_username().unwrap() == user || get_effective_uid() == 0,
        "you are not allowed to execute this operation"
    );

    let conn = Connection::open(Path::new(DB_PATH))?;
    let rows_updated = conn.execute(
        "UPDATE workspaces
            SET expiration_time = ?1
            WHERE filesystem = ?2
                AND user = ?3
                AND name = ?4",
        (Local::now(), filesystem, user, name),
    )?;
    assert_eq!(rows_updated, 1);

    let status = Command::new("zfs")
        .args([
            "set",
            "readonly=on",
            &format!("{}/{}/{}", filesystem, user, name),
        ])
        .status()?;
    assert!(status.success(), "failed to update readonly property");

    Ok(())
}

fn clean() -> Result<(), Box<dyn Error>> {
    let conn = Connection::open(Path::new(DB_PATH))?;
    let mut statement = conn.prepare(
        "SELECT filesystem, user, name, expiration_time
                FROM workspaces
                WHERE expiration_time < ?1",
    )?;
    let mut rows = statement.query([Local::now()])?;
    while let Some(row) = rows.next()? {
        let filesystem: String = row.get(0).unwrap();
        let user: String = row.get(1).unwrap();
        let name: String = row.get(2).unwrap();
        let expiration_time: DateTime<Local> = row.get(3).unwrap();

        if expiration_time < Local::now() - Duration::days(30) {
            let status = Command::new("zfs")
                .args([
                    "destroy",
                    &format!("{}/{}/{}", filesystem, user, name),
                ])
                .status()
                .unwrap();
            assert!(status.success(), "failed to delete dataset");
            conn.execute(
                "DELETE FROM workspaces
                    WHERE filesystem = ?1
                        AND user = ?2
                        AND name = ?3",
                (filesystem, user, name),
            )?;
        } else {
            let status = Command::new("zfs")
                .args([
                    "set",
                    "readonly=on",
                    &format!("{}/{}/{}", filesystem, user, name),
                ])
                .status()
                .unwrap();
            assert!(status.success(), "failed to update readonly property");
        }
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = cli::Args::parse();
    match args.command {
        cli::Command::Create {
            filesystem,
            name,
            duration,
            user,
            group,
        } => create(&filesystem, &user, &name, &group, &duration),
        cli::Command::List {} => list(),
        cli::Command::Extend {
            filesystem,
            name,
            user,
            duration,
        } => extend(&filesystem, &user, &name, &duration),
        cli::Command::Expire {
            filesystem,
            name,
            user,
        } => expire(&filesystem, &user, &name),
        cli::Command::Clean {} => clean(),
    }
}
