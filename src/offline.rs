use crate::{DmxPort, OpenError, WriteError};
use serde::{Deserialize, Serialize};

use std::fmt;

#[derive(Debug, Serialize, Deserialize)]
pub struct OfflineDmxPort;

/// Return an offline DMX port that can be included in a port listing.
pub fn offline() -> Box<dyn DmxPort> {
    Box::new(OfflineDmxPort)
}

#[typetag::serde]
impl DmxPort for OfflineDmxPort {
    fn open(&mut self) -> Result<(), OpenError> {
        Ok(())
    }

    fn close(&mut self) {}

    fn write(&mut self, _: &[u8]) -> Result<(), WriteError> {
        Ok(())
    }
}

impl fmt::Display for OfflineDmxPort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "offline")
    }
}
