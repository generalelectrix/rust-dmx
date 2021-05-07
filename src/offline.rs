use crate::{DmxPort, Error, PortOpener};
use serde::{Deserialize, Serialize};
use serialport::{SerialPortInfo, SerialPortType};

use std::fmt;

#[derive(Debug, Serialize, Deserialize)]
pub struct OfflineDmxPort;

#[typetag::serde]
impl DmxPort for OfflineDmxPort {
    fn available_ports() -> Result<Vec<(SerialPortInfo, Box<PortOpener>)>, Error> {
        Ok(vec![(
            SerialPortInfo {
                port_name: "offline".to_string(),
                port_type: SerialPortType::Unknown,
            },
            Box::new(|| Ok(Box::new(Self))),
        )])
    }

    fn open(&mut self) -> Result<(), Error> {
        Ok(())
    }

    fn close(&mut self) {}

    fn write(&mut self, _: &[u8]) -> Result<(), Error> {
        Ok(())
    }
}

impl fmt::Display for OfflineDmxPort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "offline")
    }
}
