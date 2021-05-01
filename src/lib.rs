use derive_more::Display;
use serial::Error as SerialError;
use std::error::Error as StdError;

mod enttec;
mod offline;

pub use enttec::EnttecDmxPort;
pub use offline::OfflineDmxPort;

/// Trait for the general notion of a DMX port.
/// This enables creation of an "offline" port to slot into place if an API requires an output.
pub trait DmxPort {
    /// Return the available ports, and closures that can open them.
    fn available_ports() -> PortListing
    where
        Self: Sized;

    /// Write a DMX frame out to the port.  If the frame is smaller than the minimum universe size,
    /// it will be padded with zeros.  If the frame is larger than the maximum universe size, the
    /// values beyond the max size will be ignored.
    fn write(&mut self, frame: &[u8]) -> Result<(), Error>;
}

pub type PortOpener = dyn Fn() -> Result<Box<dyn DmxPort>, Error>;
type PortListing = Vec<(String, Box<PortOpener>)>;

/// Gather up all of the providers and use them to get listings of all ports they have available.
/// Return them as a vector of names plus opener functions.
/// This function does not check whether or not any of the ports are in use already.
pub fn available_ports() -> PortListing {
    let mut ports = Vec::new();
    ports.extend(OfflineDmxPort::available_ports().into_iter());
    ports.extend(EnttecDmxPort::available_ports().into_iter());
    ports
}

#[derive(Debug, Display)]
pub enum Error {
    Serial(SerialError),
    IO(std::io::Error),
    InvalidNamespace(String),
}

impl From<SerialError> for Error {
    fn from(e: SerialError) -> Self {
        Error::Serial(e)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::IO(e)
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        use Error::*;
        match *self {
            Serial(ref e) => Some(e),
            IO(ref e) => Some(e),
            InvalidNamespace(_) => None,
        }
    }
}
