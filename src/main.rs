use chrono::{DateTime, Duration, Local};
use clap::Parser;
use rusqlite::Connection;
use std::{
    collections::HashMap,
    fs,
    os::unix::prelude::PermissionsExt,
    process::{self, Command},
};
use users::{get_current_uid, get_current_username};

mod cli;
mod config;
mod zfs;

const DB_PATH: &str = "/usr/local/share/workspaces/workspaces.db";
const CONFIG_PATH: &str = "/usr/local/etc/workspaces.toml";

mod exit_codes {
    pub const INSUFFICIENT_PRIVILEGES: i32 = 1;
    pub const FS_DISABLED: i32 = 2;
    pub const TOO_HIGH_DURATION: i32 = 3;
    pub const UNKNOWN_VOLUME: i32 = 4;
    pub const WORKSPACE_EXISTS: i32 = 4;
}

fn create(
    conn: &mut Connection,
    filesystem_name: &str,
    filesystem: &config::Filesystem,
    user: &str,
    name: &str,
    duration: &Duration,
) {
    if get_current_username().unwrap() != user && get_current_uid() != 0 {
        eprintln!("You are not allowed to execute this operation");
        process::exit(exit_codes::INSUFFICIENT_PRIVILEGES);
    }
    if filesystem.disabled && get_current_uid() != 0 {
        eprintln!("Filesystem is disabled. Please try another filesystem.");
        process::exit(exit_codes::FS_DISABLED);
    }
    if duration > &filesystem.max_duration && get_current_uid() != 0 {
        eprintln!(
            "Duration can be at most {} days",
            filesystem.max_duration.num_days()
        );
        process::exit(exit_codes::TOO_HIGH_DURATION);
    }

    let transaction = conn.transaction().unwrap();
    match transaction.execute(
        "INSERT INTO workspaces (filesystem, user, name, expiration_time)
            VALUES (?1, ?2, ?3, ?4)",
        (filesystem_name, user, name, Local::now() + *duration),
    ) {
        Ok(_) => {}
        Err(rusqlite::Error::SqliteFailure(
            libsqlite3_sys::Error {
                code: libsqlite3_sys::ErrorCode::ConstraintViolation,
                ..
            },
            _,
        )) => {
            eprintln!(
                "This workspace already exists. You can extend it using `workspaces extend`."
            );
            process::exit(exit_codes::WORKSPACE_EXISTS);
        }
        Err(_) => unreachable!(),
    };

    let volume = to_volume_string(&filesystem.root, user, name);

    zfs::create(&volume).unwrap();

    let mountpoint = zfs::get_property(&volume, "mountpoint").unwrap();

    let mut permissions = fs::metadata(&mountpoint).unwrap().permissions();
    permissions.set_mode(0o750);
    fs::set_permissions(&mountpoint, permissions).unwrap();

    let status = Command::new("chown")
        .args([&format!("{}:{}", user, user), &mountpoint])
        .status()
        .unwrap();
    assert!(status.success(), "failed to change owner on dataset");
    transaction.commit().unwrap();

    println!("Created workspace at {}", mountpoint);
}

fn to_volume_string(root: &str, user: &str, name: &str) -> String {
    format!("{}/{}/{}", root, user, name)
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
        "{:<23}\t{:<15}\t{:<15}\t{:<14}\t{:<6}\t{}",
        "NAME", "USER", "FILESYSTEM", "EXPIRY DATE", "SIZE", "MOUNTPOINT"
    );
    for workspace in workspace_iter {
        let workspace = workspace.unwrap();
        let volume = to_volume_string(
            &filesystems[&workspace.filesystem_name].root,
            &workspace.user,
            &workspace.name,
        );
        let mountpoint = zfs::get_property(&volume, "mountpoint");
        let referenced = zfs::get_property(&volume, "referenced");
        if mountpoint.is_err() || referenced.is_err() {
            eprintln!("Failed to get info for {}", volume);
            continue;
        }

        print!(
            "{:<23}\t{:<15}\t{:<15}",
            workspace.name, workspace.user, workspace.filesystem_name
        );

        if Local::now()
            > workspace.expiration_time + filesystems[&workspace.filesystem_name].expired_retention
        {
            print!("\tdeleted   soon");
        } else if Local::now() > workspace.expiration_time {
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

        println!("\t{:>6}\t{}", referenced.unwrap(), mountpoint.unwrap());
    }
}

fn extend(
    conn: &Connection,
    filesystem_name: &str,
    filesystem: &config::Filesystem,
    user: &str,
    name: &str,
    duration: &Duration,
) {
    if get_current_username().unwrap() != user && get_current_uid() != 0 {
        eprintln!("You are not allowed to execute this operation");
        process::exit(exit_codes::INSUFFICIENT_PRIVILEGES);
    }
    if filesystem.disabled && get_current_uid() != 0 {
        eprintln!("Filesystem is disabled. Please recreate workspace on another filesystem.");
        process::exit(exit_codes::FS_DISABLED);
    }
    if duration > &filesystem.max_duration && get_current_uid() != 0 {
        eprintln!(
            "Duration can be at most {} days",
            filesystem.max_duration.num_days()
        );
        process::exit(exit_codes::TOO_HIGH_DURATION);
    }

    let rows_updated = conn
        .execute(
            "UPDATE workspaces
            SET expiration_time = MAX(expiration_time, ?1)
            WHERE filesystem = ?2
                AND user = ?3
                AND name = ?4",
            (Local::now() + *duration, filesystem_name, user, name),
        )
        .unwrap();
    match rows_updated {
        0 => {
            eprintln!(
                "Could not find a matching filesystem={}, user={}, name={}",
                filesystem_name, user, name
            );
            process::exit(exit_codes::UNKNOWN_VOLUME);
        }
        1 => {}
        _ => unreachable!(),
    };

    zfs::set_property(
        &to_volume_string(&filesystem.root, &user, &name),
        "readonly",
        "off",
    )
    .unwrap();
}

fn expire(
    conn: &Connection,
    filesystem_name: &str,
    filesystem: &config::Filesystem,
    user: &str,
    name: &str,
    delete_on_next_clean: bool,
) {
    if get_current_username().unwrap() != user && get_current_uid() != 0 {
        eprintln!("You are not allowed to execute this operation");
        process::exit(exit_codes::INSUFFICIENT_PRIVILEGES);
    }

    let expiration_time = if delete_on_next_clean {
        // set the expiration time sufficiently far in the past
        // for it to get cleaned up soon
        Local::now() - filesystem.expired_retention
    } else {
        Local::now()
    };
    let rows_updated = conn
        .execute(
            "UPDATE workspaces
            SET expiration_time = MIN(expiration_time, ?1)
            WHERE filesystem = ?2
                AND user = ?3
                AND name = ?4",
            (expiration_time, filesystem_name, user, name),
        )
        .unwrap();
    match rows_updated {
        0 => {
            eprintln!(
                "Could not find a matching filesystem={}, user={}, name={}",
                filesystem_name, user, name
            );
            process::exit(exit_codes::UNKNOWN_VOLUME);
        }
        1 => {}
        _ => unreachable!(),
    };

    zfs::set_property(
        &to_volume_string(&filesystem.root, &user, &name),
        "readonly",
        "on",
    )
    .unwrap();
}

fn filesystems(filesystems: &HashMap<String, config::Filesystem>) {
    println!(
        "{:<15}\t{:<7}\t{}\t{}",
        "FILESYSTEM", "FREE", "DURATION", "RETENTION"
    );
    filesystems.iter().for_each(|(name, info)| {
        let available = zfs::get_property(&info.root, "available").unwrap();
        print!("{:<15}\t{:>7}", name, available);
        if info.disabled {
            print!("\t{:>8}", "disabled");
        } else {
            print!("\t{:>7}d", info.max_duration.num_days());
        }
        println!("\t{:>8}d", info.expired_retention.num_days());
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
            let volume = to_volume_string(&filesystem.root, &user, &name);
            if expiration_time < Local::now() - filesystem.expired_retention {
                zfs::destroy(&volume).unwrap();
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
                zfs::set_property(&volume, "readonly", "on").unwrap();
            }
        }
    }
    transaction.commit().unwrap();
}

const UPDATE_DB: &[fn(&mut Connection)] = &[|conn| {
    // Creates initial database
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
            filesystem_name,
            name,
            user,
            duration,
        } => extend(
            &mut conn,
            &filesystem_name,
            &config.filesystems[&filesystem_name],
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
