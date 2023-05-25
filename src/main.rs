use chrono::{DateTime, Duration, Local};
use clap::Parser;
use cli::FilesystemsColumns;
use prettytable::{
    color,
    format::{Alignment, FormatBuilder},
    Attr, Cell, Row, Table,
};
use rusqlite::Connection;
use std::{
    collections::HashMap,
    fs,
    os::unix::prelude::PermissionsExt,
    path::PathBuf,
    process::{self, Command},
};
use users::{get_current_uid, get_current_username};

mod cli;
mod config;
mod zfs;

/// Path to store the workspace database in
const DB_PATH: &str = "/usr/local/lib/workspaces/workspaces.db";
/// Path of the configuration file
const CONFIG_PATH: &str = "/etc/workspaces/workspaces.toml";

mod exit_codes {
    /// The user tried executing an action they have no rights to do,
    /// i.e. expiring another user's workspace
    pub const INSUFFICIENT_PRIVILEGES: i32 = 1;
    /// The user tried creating / extending a workspace on a disabled filesystem
    pub const FS_DISABLED: i32 = 2;
    /// The user tried creating / extending a workspace with too long a duration
    pub const TOO_HIGH_DURATION: i32 = 3;
    /// The workspace specified by a user does not exist
    pub const UNKNOWN_WORKSPACE: i32 = 4;
    /// The user tried to create a workspace that already exists
    pub const WORKSPACE_EXISTS: i32 = 5;
    /// No filesystem given and no default specified in configuration file
    pub const NO_FILESYSTEM_SPECIFIED: i32 = 6;
}

/// Creates a new workspace
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

/// Renames an existing workspace
fn rename(
    conn: &mut Connection,
    filesystem_name: &str,
    filesystem: &config::Filesystem,
    user: &str,
    src_name: &str,
    dest_name: &str,
) {
    if get_current_username().unwrap() != user && get_current_uid() != 0 {
        eprintln!("You are not allowed to execute this operation");
        process::exit(exit_codes::INSUFFICIENT_PRIVILEGES);
    }
    if filesystem.disabled && get_current_uid() != 0 {
        eprintln!("Filesystem is disabled. Please try another filesystem.");
        process::exit(exit_codes::FS_DISABLED);
    }

    let transaction = conn.transaction().unwrap();
    match transaction.execute(
        "UPDATE workspaces
                SET name = ?1
                WHERE filesystem = ?2
                    AND user = ?3
                    AND name = ?4",
        (dest_name, filesystem_name, user, src_name),
    ) {
        Ok(_) => {}
        Err(rusqlite::Error::SqliteFailure(
            libsqlite3_sys::Error {
                code: libsqlite3_sys::ErrorCode::ConstraintViolation,
                ..
            },
            _,
        )) => {
            eprintln!("The target workspace already exists");
            process::exit(exit_codes::WORKSPACE_EXISTS);
        }
        Err(_) => unreachable!(),
    }

    let src_volume = to_volume_string(&filesystem.root, user, src_name);
    let dest_volume = to_volume_string(&filesystem.root, user, dest_name);
    zfs::rename(&src_volume, &dest_volume).unwrap();
    transaction.commit().unwrap();
}

#[derive(Debug)]
struct WorkspacesRow {
    filesystem_name: String,
    user: String,
    name: String,
    expiration_time: DateTime<Local>,
}

fn list(
    conn: &Connection,
    filesystems: &HashMap<String, config::Filesystem>,
    filter_users: &Option<Vec<String>>,
    filter_filesystems: &Option<Vec<String>>,
    output: &Option<Vec<cli::WorkspacesColumns>>,
) {
    use cli::WorkspacesColumns;
    // the default columns
    let output = output.clone().unwrap_or(vec![
        WorkspacesColumns::Name,
        WorkspacesColumns::User,
        WorkspacesColumns::Fs,
        WorkspacesColumns::Size,
        WorkspacesColumns::Expiry,
        WorkspacesColumns::Mountpoint,
    ]);

    let mut table = Table::new();
    table.set_format(FormatBuilder::new().padding(0, 2).build());

    // bold title row
    table.set_titles(Row::new(
        output
            .iter()
            .map(|h| Cell::new(&h.to_string()).with_style(Attr::Bold))
            .collect(),
    ));

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

    for workspace in workspace_iter {
        let workspace = workspace.unwrap();
        if !filter_users
            .as_ref()
            .map_or(true, |us| us.contains(&workspace.user))
            || !filter_filesystems
                .as_ref()
                .map_or(true, |fs| fs.contains(&workspace.filesystem_name))
        {
            continue;
        }
        let volume = to_volume_string(
            &filesystems
                .get(&workspace.filesystem_name)
                .expect("found workspace in database without corresponding config entry")
                .root,
            &workspace.user,
            &workspace.name,
        );
        let referenced = zfs::get_property::<usize>(&volume, "referenced");
        let mountpoint = zfs::get_property::<PathBuf>(&volume, "mountpoint");
        if mountpoint.is_err() || referenced.is_err() {
            eprintln!("Failed to get info for {}", volume);
            continue;
        }
        table.add_row(Row::new(
            output
                .iter()
                .map(|column| match column {
                    WorkspacesColumns::Name => Cell::new(&workspace.name),
                    WorkspacesColumns::User => Cell::new(&workspace.user),
                    WorkspacesColumns::Fs => Cell::new(&workspace.filesystem_name),
                    WorkspacesColumns::Expiry => {
                        if Local::now()
                            > workspace.expiration_time
                                + filesystems[&workspace.filesystem_name].expired_retention
                        {
                            Cell::new("deleted soon")
                                .with_style(Attr::Bold)
                                .with_style(Attr::ForegroundColor(color::RED))
                        } else if Local::now() > workspace.expiration_time {
                            Cell::new_align(
                                &format!(
                                    "deleted in {:>2}d",
                                    (workspace.expiration_time
                                        + filesystems[&workspace.filesystem_name]
                                            .expired_retention
                                        - Local::now())
                                    .num_days()
                                ),
                                Alignment::RIGHT,
                            )
                            .with_style(Attr::Bold)
                            .with_style(Attr::ForegroundColor(color::RED))
                        } else if workspace.expiration_time - Local::now() < Duration::days(30) {
                            Cell::new_align(
                                &format!(
                                    "expires in {:>2}d",
                                    (workspace.expiration_time - Local::now()).num_days()
                                ),
                                Alignment::RIGHT,
                            )
                            .with_style(Attr::ForegroundColor(color::YELLOW))
                        } else {
                            Cell::new_align(
                                &format!(
                                    "expires in {:>2}d",
                                    (workspace.expiration_time - Local::now()).num_days()
                                ),
                                Alignment::RIGHT,
                            )
                        }
                    }
                    WorkspacesColumns::Size => Cell::new_align(
                        &format!("{}G", referenced.as_ref().unwrap() / (1 << 30)),
                        Alignment::RIGHT,
                    ),
                    WorkspacesColumns::Mountpoint => {
                        Cell::new(mountpoint.as_ref().unwrap().to_str().unwrap())
                    }
                })
                .collect(),
        ));
    }

    table.printstd();
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
            process::exit(exit_codes::UNKNOWN_WORKSPACE);
        }
        1 => {}
        _ => unreachable!(),
    };

    zfs::set_property(
        &to_volume_string(&filesystem.root, user, name),
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
            process::exit(exit_codes::UNKNOWN_WORKSPACE);
        }
        1 => {}
        _ => unreachable!(),
    };

    zfs::set_property(
        &to_volume_string(&filesystem.root, user, name),
        "readonly",
        "on",
    )
    .unwrap();
}

fn filesystems(
    filesystems: &HashMap<String, config::Filesystem>,
    output: Option<Vec<cli::FilesystemsColumns>>,
) {
    // the default columns
    let output = output.unwrap_or(vec![
        FilesystemsColumns::Name,
        FilesystemsColumns::Used,
        FilesystemsColumns::Free,
        FilesystemsColumns::Total,
        FilesystemsColumns::Duration,
        FilesystemsColumns::Retention,
    ]);

    let mut table = Table::new();
    table.set_format(FormatBuilder::new().padding(0, 2).build());

    // bold title row
    table.set_titles(Row::new(
        output
            .iter()
            .map(|h| Cell::new(&h.to_string()).with_style(Attr::Bold))
            .collect(),
    ));

    for (name, info) in filesystems {
        let used = zfs::get_property::<usize>(&info.root, "used").unwrap();
        let available = zfs::get_property::<usize>(&info.root, "available").unwrap();
        let total = used + available;
        table.add_row(Row::new(
            output
                .iter()
                .map(|column| match column {
                    FilesystemsColumns::Name => Cell::new(name),
                    FilesystemsColumns::Used => {
                        Cell::new_align(&format!("{}G", used / (1 << 30)), Alignment::RIGHT)
                    }
                    FilesystemsColumns::Free => {
                        Cell::new_align(&format!("{}G", available / (1 << 30)), Alignment::RIGHT)
                    }
                    FilesystemsColumns::Total => {
                        Cell::new_align(&format!("{}G", total / (1 << 30)), Alignment::RIGHT)
                    }
                    FilesystemsColumns::Duration => match info.disabled {
                        true => Cell::new("disabled"),
                        false => {
                            Cell::new(&format!("{}d", info.max_duration.num_days())).style_spec("r")
                        }
                    },
                    FilesystemsColumns::Retention => {
                        Cell::new(&format!("{}d", info.expired_retention.num_days()))
                            .style_spec("r")
                    }
                })
                .map(|c| {
                    // color if almost full
                    if used as f64 > total as f64 * 0.9 {
                        c.with_style(Attr::ForegroundColor(color::RED))
                    } else if used as f64 > total as f64 * 0.75 {
                        c.with_style(Attr::ForegroundColor(color::YELLOW))
                    } else {
                        c
                    }
                })
                .map(|c| {
                    // dim if disabled
                    if info.disabled {
                        c.with_style(Attr::Dim)
                    } else {
                        c
                    }
                })
                .collect(),
        ));
    }

    table.printstd();
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
                if zfs::destroy(&volume).is_err() {
                    continue;
                }
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

//TODO make result
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
    // read config
    let toml_str = fs::read_to_string(CONFIG_PATH).expect("could not find configuration file");
    let config: config::Config =
        toml::from_str(&toml_str).expect("error parsing configuration file");

    let args = cli::Args::parse();

    // make sure database schema is current
    let mut conn = Connection::open(DB_PATH).unwrap();
    let db_version: usize = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert!(
        db_version <= NEWEST_DB_VERSION,
        "database seems to be from a more current version of workspaces"
    );
    // iteratively apply necessary database updates
    UPDATE_DB[db_version..].iter().for_each(|f| f(&mut conn));

    match args.command {
        cli::Command::Create {
            filesystem_name,
            workspace_name: name,
            duration,
            user,
        } => {
            let filesystem_name = filesystem_or_default_or_exit(
                &filesystem_name,
                &config.filesystems,
                &config.default_filesystem,
            );
            create(
                &mut conn,
                &filesystem_name,
                &config.filesystems[&filesystem_name],
                &user,
                &name,
                &duration,
            )
        }
        cli::Command::List {
            filter_users,
            filter_filesystems,
            output,
        } => list(
            &conn,
            &config.filesystems,
            &filter_users,
            &filter_filesystems,
            &output,
        ),
        cli::Command::Rename {
            src_workspace_name,
            dest_workspace_name,
            user,
            filesystem_name,
        } => {
            let filesystem_name = filesystem_or_default_or_exit(
                &filesystem_name,
                &config.filesystems,
                &config.default_filesystem,
            );
            rename(
                &mut conn,
                &filesystem_name,
                &config.filesystems[&filesystem_name],
                &user,
                &src_workspace_name,
                &dest_workspace_name,
            )
        }
        cli::Command::Extend {
            filesystem_name,
            name,
            user,
            duration,
        } => {
            let filesystem_name = filesystem_or_default_or_exit(
                &filesystem_name,
                &config.filesystems,
                &config.default_filesystem,
            );
            extend(
                &conn,
                &filesystem_name,
                &config.filesystems[&filesystem_name],
                &user,
                &name,
                &duration,
            )
        }
        cli::Command::Expire {
            filesystem_name,
            name,
            user,
            delete_on_next_clean,
        } => {
            let filesystem_name = filesystem_or_default_or_exit(
                &filesystem_name,
                &config.filesystems,
                &config.default_filesystem,
            );
            expire(
                &conn,
                &filesystem_name,
                &config.filesystems[&filesystem_name],
                &user,
                &name,
                delete_on_next_clean,
            )
        }
        cli::Command::Filesystems { output } => filesystems(&config.filesystems, output),
        cli::Command::Clean => clean(&mut conn, &config.filesystems),
    }
}

/// Horrible stateful filesystem name validation function
///
/// Returns with this order of preference:
/// - the given filesystem name if it exists
/// - the default filesystem, if specified in the config
/// - the only filesystem if there is only one
///
/// Otherwise, it terminates the program
fn filesystem_or_default_or_exit(
    filesystem_name: &Option<String>,
    filesystems: &HashMap<String, config::Filesystem>,
    default: &Option<String>,
) -> String {
    let filesystem_name: String = if let Some(name) = filesystem_name {
        name.clone()
    } else if let Some(name) = default {
        name.clone()
    } else if filesystems.len() == 1 {
        filesystems.keys().next().unwrap().clone()
    } else {
        eprintln!("Please specify a filesystem with `-f <FILESYSTEM>`");
        process::exit(exit_codes::NO_FILESYSTEM_SPECIFIED);
    };

    if filesystems.contains_key(&filesystem_name) {
        filesystem_name
    } else {
        eprint!("Invalid filesystem name. Please use one of the following:");
        for name in filesystems.keys() {
            eprint!(" {}", name);
        }
        eprintln!();
        process::exit(exit_codes::UNKNOWN_WORKSPACE);
    }
}
