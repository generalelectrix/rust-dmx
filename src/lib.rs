use serde::{Deserialize, Deserializer};
use serde::{Serialize, Serializer};
use serial::Error as SerialError;
use std::error::Error as StdError;
use std::fmt;

mod enttec;

pub use enttec::{EnttecDmxPort, EnttecPortProvider, ENTTEC_NAMESPACE};

/// Trait for the general notion of a DMX port.
/// This enables creation of an "offline" port to slot into place if an API requires an output.
pub trait DmxPort: fmt::Debug {
    /// Write a DMX frame out to the port.  If the frame is smaller than the minimum universe size,
    /// it will be padded with zeros.  If the frame is larger than the maximum universe size, the
    /// values beyond the max size will be ignored.
    fn write(&mut self, frame: &[u8]) -> Result<(), Error>;

    /// Return the namespace this port lives in.
    fn namespace(&self) -> &str;

    /// Return the name of this port.  Should only be used for display purposes.
    fn port_name(&self) -> &str;

    /// Return a SerializablePort to be used to try to reopen a port
    /// after deserialization of a saved show or after application restart.
    fn serializable(&self) -> SerializablePort {
        SerializablePort::new(self.namespace(), self.port_name())
    }
}

/// A source of DmxPorts based on unique string identifiers.
pub trait DmxPortProvider {
    /// Return a unique namespace for this port provider.
    fn namespace(&self) -> &str;
    /// Return a description of the available ports provided by this provider.
    fn available_ports(&self) -> Vec<String>;
    /// Attempt to open this port, and return it behind the trait object or an error.
    fn open<N: Into<String>>(&self, port: N) -> Result<Box<dyn DmxPort>, Error>;
}

pub struct OfflineDmxPort;

impl fmt::Debug for OfflineDmxPort {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.serializable().fmt(f)
    }
}

const OFFLINE_NAMESPACE: &'static str = "offline";
const OFFLINE_ID: &'static str = "offline";

impl DmxPort for OfflineDmxPort {
    fn write(&mut self, _: &[u8]) -> Result<(), Error> {
        Ok(())
    }
    fn namespace(&self) -> &str {
        OFFLINE_NAMESPACE
    }
    fn port_name(&self) -> &str {
        OFFLINE_ID
    }
}

pub struct OfflinePortProvider;

impl DmxPortProvider for OfflinePortProvider {
    /// Return a unique namespace for this port provider.
    fn namespace(&self) -> &str {
        OFFLINE_NAMESPACE
    }
    /// Return a description of the available ports provided by this provider.
    fn available_ports(&self) -> Vec<String> {
        vec![OFFLINE_ID.to_string()]
    }
    /// Attempt to open this port, and return it behind the trait object or an error.
    fn open<N: Into<String>>(&self, _: N) -> Result<Box<dyn DmxPort>, Error> {
        Ok(Box::new(OfflineDmxPort))
    }
}

/// Gather up all of the providers behind their namespace.
/// This is your one-stop-shop for port creation.
pub fn open_port<N: Into<String>>(namespace: &str, port_name: N) -> Result<Box<dyn DmxPort>, Error> {
    match namespace {
        OFFLINE_NAMESPACE => OfflinePortProvider.open(port_name),
        ENTTEC_NAMESPACE => EnttecPortProvider.open(port_name),
        _ => return Err(Error::InvalidNamespace(namespace.to_string())),
    }
}

/// Gather up all of the providers and use them to get listings of all ports they have available.
/// Return them as a vector of pairs, each of which would be suitable to feed to open_port.
/// This function does not check whether or not any of the ports are in use already.
pub fn available_ports() -> Vec<(String, String)> {
    let mut ports = Vec::new();
    fn add_ports<P: DmxPortProvider>(ports: &mut Vec<(String, String)>, provider: P) {
        let namespace = provider.namespace();
        let mut available = provider.available_ports();
        for port_id in available.drain(..) {
            ports.push((namespace.to_string(), port_id));
        }
    }
    add_ports(&mut ports, OfflinePortProvider);
    add_ports(&mut ports, EnttecPortProvider);
    ports
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
/// A serializable data structure for persisting a record of a port to disk, also providing
/// for attempted reopening of a port.  Since we serialize and deserialize directly from disk,
/// this data structure needs to own its data or Serde will fail upon deserialization as it isn't
/// quite clever enough to figure out that this reference doesn't need to live beyond the
/// deserialization step.
pub struct SerializablePort {
    namespace: String,
    id: String,
}

impl SerializablePort {
    fn new<N: Into<String>, I: Into<String>>(namespace: N, id: I) -> Self {
        SerializablePort {
            namespace: namespace.into(),
            id: id.into(),
        }
    }

    /// Try to open the port described by this serialized form.
    fn open(self) -> Result<Box<dyn DmxPort>, Error> {
        match self.namespace.as_str() {
            OFFLINE_NAMESPACE => Ok(Box::new(OfflineDmxPort {})),
            ENTTEC_NAMESPACE => Ok(Box::new(EnttecDmxPort::new(self.id)?)),
            _ => Err(Error::InvalidNamespace(self.namespace.to_string())),
        }
    }

    /// Based on the namespace and id, try to reopen this DMX port.
    /// If we don't know the namespace or the port isn't available, return an offline port.
    fn reopen(self) -> Box<dyn DmxPort> {
        self.open().unwrap_or(Box::new(OfflineDmxPort {}))
    }
}

// Helper functions to use when serializing and deserializing DmxPort trait objects contained in
// other structs.  This can be done using the serde with attribute.
pub fn serialize<S>(port: &Box<dyn DmxPort>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    port.serializable().serialize(serializer)
}

pub fn deserialize<'de, D>(deserializer: D) -> Result<Box<dyn DmxPort>, D::Error>
where
    D: Deserializer<'de>,
{
    SerializablePort::deserialize(deserializer).map(SerializablePort::reopen)
}

#[derive(Debug)]
pub enum Error {
    Serial(SerialError),
    IO(std::io::Error),
    InvalidNamespace(String),
}

/// We're ok with a loose equality comparison here.  Just delegate to description for now.
impl PartialEq for Error {
    fn eq(&self, other: &Self) -> bool {
        use Error::*;
        match (self, other) {
            (&Serial(ref e0), &Serial(ref e1)) => e0.description() == e1.description(),
            (&IO(ref e0), &IO(ref e1)) => e0.description() == e1.description(),
            (&InvalidNamespace(ref n0), &InvalidNamespace(ref n1)) => n0 == n1,
            _ => false,
        }
    }
}

impl Eq for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use Error::*;
        match *self {
            Serial(ref e) => e.fmt(f),
            IO(ref e) => e.fmt(f),
            InvalidNamespace(ref n) => write!(f, "Invalid DMX port namesapce: '{}'.", n),
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

impl StdError for Error {
    fn description(&self) -> &str {
        use Error::*;
        match *self {
            Serial(ref e) => e.description(),
            IO(ref e) => e.description(),
            InvalidNamespace(_) => "Invalid DMX port namespace.",
        }
    }

    fn cause(&self) -> Option<&dyn StdError> {
        use Error::*;
        match *self {
            Serial(ref e) => Some(e),
            IO(ref e) => Some(e),
            InvalidNamespace(_) => None,
        }
    }
}
