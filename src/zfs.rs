use std::{
    io,
    process::{self, Command},
    str::FromStr,
};

#[derive(Debug)]
pub enum Error {
    /// An error occurring while running a command
    Command(io::Error),
    /// The ZFS invocation completed, but returned a non-zero code
    ZfsStatus(process::ExitStatus),
    /// Error while parsing ZFS's output
    PropertyParse(Box<dyn std::error::Error>),
}

/// Creates a new ZFS volume
pub fn create(volume: &str) -> Result<(), Error> {
    let status = Command::new("zfs")
        .args(["create", "-p", &volume])
        .status()
        .map_err(Error::Command)?;
    match status.success() {
        true => Ok(()),
        false => Err(Error::ZfsStatus(status)),
    }
}

/// Destroys a ZFS volume
pub fn destroy(volume: &str) -> Result<(), Error> {
    let status = Command::new("zfs")
        .args(["destroy", &volume])
        .status()
        .map_err(Error::Command)?;
    match status.success() {
        true => Ok(()),
        false => Err(Error::ZfsStatus(status)),
    }
}

/// Renames a ZFS volume
pub fn rename(src_volume: &str, dest_volume: &str) -> Result<(), Error> {
    let status = Command::new("zfs")
        .args(["rename", src_volume, dest_volume])
        .status()
        .map_err(Error::Command)?;
    match status.success() {
        true => Ok(()),
        false => Err(Error::ZfsStatus(status)),
    }
}

/// Retrieves a ZFS property
pub fn get_property<F: FromStr>(volume: &str, property: &str) -> Result<F, Error>
where
    <F as FromStr>::Err: std::error::Error + 'static,
{
    let output = Command::new("zfs")
        .args([
            "get", "-Hp", // make zfs output easily parsable
            "-o", "value", // output only desired value
            property, volume,
        ])
        .output()
        .map_err(Error::Command)?;
    if !output.status.success() {
        return Err(Error::ZfsStatus(output.status));
    }
    let mut info_line = String::from_utf8(output.stdout).unwrap();
    info_line.pop(); // remove trailing newline
    info_line.parse().map_err(|e| Error::PropertyParse(Box::new(e)))
}

/// Sets a ZFS property
pub fn set_property(volume: &str, property: &str, value: &str) -> Result<(), Error> {
    let status: process::ExitStatus = Command::new("zfs")
        .args(["set", &format!("{}={}", property, value), volume])
        .status()
        .map_err(Error::Command)?;

    match status.success() {
        true => Ok(()),
        false => Err(Error::ZfsStatus(status)),
    }
}
