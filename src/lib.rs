extern crate serial;
#[macro_use] extern crate serde_derive;
extern crate serde;

use serde::ser::{Serializer, Serialize};
use serde::de::{self, Deserializer, Deserialize};

pub use serial::Error;

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

    /// Return a tuple of (namespace, port_identifier) to be used to try to reopen a port
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

#[derive(Serialize, Deserialize)]
/// A serializable data structure for persisting a record of a port to disk, also providing
/// for attempted reopening of a port.
pub struct SerializablePort {
    namespace: String,
    id: String,
}

impl SerializablePort {
    fn new<N: Into<String>, I: Into<String>>(namespace: N, id: I) -> Self {
        SerializablePort { namespace: namespace.into(), id: id.into() }
    }
    /// Based on the namespace and id, try to reopen this DMX port.
    /// If we don't know the namespace or the port isn't available, return an offline port.
    fn reopen(self) -> Box<DmxPort> {
        fn offline() -> Box<DmxPort> {
            Box::new(OfflineDmxPort{})
        }
        match self.namespace.as_ref() {
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
pub fn serialize<S>(port: &DmxPort, serializer: S) -> Result<S::Ok, S::Error>
    where S: Serializer
{
    port.serializable().serialize(serializer)
}

pub fn deserialize<'de, D>(deserializer: D) -> Result<Box<DmxPort>, D::Error>
    where D: Deserializer<'de>
{
    SerializablePort::deserialize(deserializer).map(SerializablePort::reopen)
}