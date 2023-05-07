
use std::{
    io,
    process::{self, Command},
};

#[derive(Debug)]
pub enum Error {
    Command(io::Error),
    ZfsStatus(process::ExitStatus),
    AttributeParse,
}

pub fn get_property(volume: &str, property: &str) -> Result<String, Error> {
    let output = Command::new("zfs")
        .args(["get", property, volume])
        .output()
        .map_err(Error::Command)?;
    if !output.status.success() {
        return Err(Error::ZfsStatus(output.status));
    }
    let info_line = String::from_utf8(output.stdout).unwrap();
    info_line
        .lines()
        .nth(1)
        .ok_or(Error::AttributeParse)?
        .split_whitespace()
        .nth(2)
        .ok_or(Error::AttributeParse)
        .map(String::from)
}

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
