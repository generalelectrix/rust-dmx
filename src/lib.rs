extern crate serial;
#[macro_use] extern crate serde_derive;
extern crate serde;

use serde::ser::{Serializer, Serialize};
use serde::de::{self, Deserializer, Deserialize};
use serial::Error as SerialError;
use std::fmt;

mod enttec;

pub use enttec::{available_enttec_ports, EnttecDmxPort, ENTTEC_NAMESPACE};

/// Trait for the general notion of a DMX port.
/// This enables creation of an "offline" port to slot into place if an API requires an output.
pub trait DmxPort {
    /// Write a DMX frame out to the port.  If the frame is smaller than the minimum universe size,
    /// it will be padded with zeros.  If the frame is larger than the maximum universe size, the
    /// values beyond the max size will be ignored.
    fn write(&mut self, frame: &[u8]) -> Result<(), Error>;

    /// Return the name of this port.  Should only be used for display purposes.
    fn port_name(&self) -> &str;

    /// Return a SerializablePort to be used to try to reopen a port
    /// after deserialization of a saved show or after application restart.
    fn serializable(&self) -> SerializablePort;
}

pub struct OfflineDmxPort {}

const OFFLINE_NAMESPACE: &'static str = "offline";
const OFFLINE_ID: &'static str = "offline";

impl DmxPort for OfflineDmxPort {
    fn write(&mut self, _: &[u8]) -> Result<(), Error> {
        Ok(())
    }
    fn port_name(&self) -> &str {
        OFFLINE_ID
    }
    fn serializable(&self) -> SerializablePort {
        SerializablePort::new(OFFLINE_NAMESPACE, OFFLINE_ID)
    }
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
/// A serializable data structure for persisting a record of a port to disk, also providing
/// for attempted reopening of a port.
pub struct SerializablePort<'a> {
    namespace: &'a str,
    id: &'a str,
}

impl<'a> SerializablePort<'a> {
    fn new(namespace: &'a str, id: &'a str) -> Self {
        SerializablePort { namespace: namespace, id: id }
    }
    /// Based on the namespace and id, try to reopen this DMX port.
    /// If we don't know the namespace or the port isn't available, return an offline port.
    fn reopen(self) -> Box<DmxPort> {
        fn offline() -> Box<DmxPort> {
            Box::new(OfflineDmxPort{})
        }
        match self.namespace {
            OFFLINE_NAMESPACE => offline(),
            ENTTEC_NAMESPACE =>
                match EnttecDmxPort::new(self.id) {
                    Ok(port) => Box::new(port),
                    Err(_) => offline()
                },
            _ => offline()
        }
    }
}

// Helper functions to use when serializing and deserializing DmxPort trait objects contained in
// other structs.  This can be done using the serde with attribute.
pub fn serialize<S>(port: &Box<DmxPort>, serializer: S) -> Result<S::Ok, S::Error>
    where S: Serializer
{
    port.serializable().serialize(serializer)
}

pub fn deserialize<'de, D>(deserializer: D) -> Result<Box<DmxPort>, D::Error>
    where D: Deserializer<'de>
{
    SerializablePort::deserialize(deserializer).map(SerializablePort::reopen)
}

#[derive(Debug)]
pub enum Error {
    Serial(SerialError),
    IO(std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use Error::*;
        match *self {
            Serial(ref e) => e.fmt(f),
            IO(ref e) => e.fmt(f),
        }
    }
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

impl std::error::Error for Error {
    fn description(&self) -> &str {
        use Error::*;
        match *self {
            Serial(ref e) => e.description(),
            IO(ref e) => e.description(),
        }
    }

    fn cause(&self) -> Option<&std::error::Error> {
        use Error::*;
        match *self {
            Serial(ref e) => Some(e),
            IO(ref e) => Some(e),
        }
    }
}