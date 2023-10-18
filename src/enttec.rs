//! Implementation of support for the Enttec USB DMX Pro dongle.
use log::debug;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::time::Duration;
use std::{cmp::min, fmt};
use thiserror::Error;

use crate::{OpenError, PortListing, WriteError};

use super::DmxPort;
use serialport::{SerialPort, SerialPortInfo, SerialPortType, UsbPortInfo};

// Some constants used for enttec message framing.
const START_VAL: u8 = 0x7E;
const END_VAL: u8 = 0xE7;

// Universe size constraints.
const MIN_UNIVERSE_SIZE: usize = 24;
const MAX_UNIVERSE_SIZE: usize = 512;

// Port action flags.
const SET_PARAMETERS: u8 = 4;
//const RECEIVE_DMX_PACKET: u8 = 5;
const SEND_DMX_PACKET: u8 = 6;

/// Format a byte buffer as an enttec message into the provided writer.
/// Maximum valid size for payload is 600; no check is made here that the payload is within this range.
fn write_packet<W: Write>(
    message_type: u8,
    payload: &[u8],
    add_payload_pad_byte: bool,
    mut w: W,
) -> Result<(), WriteError> {
    // Enttec messages are the size of the payload plus 5 bytes for type, length, and framing.
    let payload_size = payload.len() + add_payload_pad_byte as usize;
    let (len_lsb, len_msb) = (payload_size as u8, (payload_size >> 8) as u8);
    let header = [START_VAL, message_type, len_lsb, len_msb];
    let mut write_all = |buf| -> Result<(), EnttecWriteError> {
        w.write_all(buf)?;
        Ok(())
    };
    write_all(&header)?;
    if add_payload_pad_byte {
        write_all(&[0][..])?;
    }
    write_all(payload)?;
    write_all(&[END_VAL][..])?;
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EnttecParams {
    /// DMX output break time in 10.67 microsecond units. Valid range is 9 to 127.
    break_time: u8,
    /// DMX output Mark After Break time in 10.67 microsecond units. Valid range is 1 to 127.
    mark_after_break_time: u8,
    /// DMX output rate in packets per second. Valid range is 1 to 40, or 0 for fastest rate
    /// possible (this will make the most difference when the output universe size is smallest).
    output_rate: u8,
}

impl Default for EnttecParams {
    /// Default parameters for the enttec port.
    /// In summary: minimum break and mark times, fastest fixed framerate.
    fn default() -> Self {
        EnttecParams {
            break_time: 9,
            mark_after_break_time: 1,
            output_rate: 40,
        }
    }
}

impl EnttecParams {
    fn write_into<W: Write>(&self, w: W) -> Result<(), WriteError> {
        let payload = [
            0, // user size lsb?
            0, // user size msb?
            self.break_time,
            self.mark_after_break_time,
            self.output_rate,
        ];
        write_packet(SET_PARAMETERS, &payload, false, w)
    }
}

#[derive(Serialize, Deserialize)]
pub struct EnttecDmxPort {
    params: EnttecParams,
    #[serde(skip)]
    port: Option<Box<dyn SerialPort>>,
    #[serde(with = "SerialPortInfoDef")]
    info: SerialPortInfo,
}

impl EnttecDmxPort {
    /// Create an enttec port.
    /// The port is not opened yet.
    pub fn new(info: SerialPortInfo) -> Self {
        let params = EnttecParams::default();

        Self {
            params,
            port: None,
            info,
        }
    }

    /// Create an enttec port and open it.
    pub fn opened(info: SerialPortInfo) -> anyhow::Result<Self> {
        let mut port = Self::new(info);
        port.open()?;
        Ok(port)
    }

    /// Write the current parameters out to the port.
    fn write_params(&mut self) -> Result<(), WriteError> {
        self.params
            .write_into(self.port.as_mut().ok_or(WriteError::Disconnected)?)?;
        Ok(())
    }
}

#[typetag::serde]
impl DmxPort for EnttecDmxPort {
    /// Return the available enttec ports connected to this system.
    /// TODO: provide a mechanism to specialize this implementation depending on platform.
    fn available_ports() -> anyhow::Result<PortListing> {
        Ok(serialport::available_ports()?
            .into_iter()
            .filter(is_enttec)
            .map(|info| Box::new(EnttecDmxPort::new(info)) as Box<dyn DmxPort>)
            .collect())
    }

    /// Open the port.
    fn open(&mut self) -> Result<(), OpenError> {
        if self.port.is_some() {
            return Ok(());
        }

        // baud rate is not used on FTDI
        let port = match serialport::new(&self.info.port_name, 57600)
            .timeout(Duration::from_millis(1))
            .open()
        {
            Ok(port) => port,
            Err(err) => {
                if let serialport::ErrorKind::Io(std::io::ErrorKind::NotFound) = err.kind() {
                    return Err(OpenError::NotConnected);
                } else {
                    return Err(OpenError::Other(err.into()));
                }
            }
        };

        self.port = Some(port);

        // send the default parameters to the port
        if let Err(e) = self.write_params() {
            self.port = None;
            return Err(OpenError::Other(e.into()));
        }
        Ok(())
    }

    fn close(&mut self) {
        self.port = None;
    }

    fn write(&mut self, frame: &[u8]) -> Result<(), WriteError> {
        // If the port isn't open, try opening it.
        // Quick profiling shows that a disconnected port only takes about
        // 100us to poll and fail, so this is acceptable to do inside an
        // application's render loop.
        if self.port.is_none() {
            if let Err(err) = self.open() {
                debug!("Failed to reopen DMX port {}: {:#?}.", self, err);
                return Err(WriteError::Disconnected);
            }
        }
        let port = self.port.as_mut().ok_or(WriteError::Disconnected)?;
        let size = frame.len();
        let write_result = if size < MIN_UNIVERSE_SIZE {
            let mut padded_frame = Vec::with_capacity(MIN_UNIVERSE_SIZE);
            padded_frame.extend_from_slice(frame);
            padded_frame.resize(MIN_UNIVERSE_SIZE, 0);
            write_packet(SEND_DMX_PACKET, &padded_frame, true, port)
        } else {
            write_packet(
                SEND_DMX_PACKET,
                &frame[0..min(size, MAX_UNIVERSE_SIZE)],
                true,
                port,
            )
        };
        if let Err(WriteError::Disconnected) = write_result {
            self.port = None;
        }
        write_result
    }
}

impl fmt::Display for EnttecDmxPort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let SerialPortType::UsbPort(p) = &self.info.port_type {
            if let Some(sn) = &p.serial_number {
                return write!(f, "Enttec DMX USB PRO {}", sn);
            }
        }
        write!(f, "Enttec DMX USB PRO {}", self.info.port_name)
    }
}

#[cfg(unix)]
fn is_enttec(info: &SerialPortInfo) -> bool {
    let SerialPortType::UsbPort(details) = &info.port_type else {
        return false;
    };
    let Some(product) = &details.product else {
        return false;
    };
    product == "DMX USB PRO" && info.port_name.contains("tty")
}

#[cfg(windows)]
fn is_enttec(info: &SerialPortInfo) -> bool {
    let SerialPortType::UsbPort(details) = &info.port_type else {
        return false;
    };
    let Some(manufacturer) = &details.manufacturer else {
        return false;
    };
    manufacturer == "FTDI"
}

#[derive(Error, Debug)]
#[error(transparent)]
pub struct EnttecWriteError(#[from] std::io::Error);

impl From<EnttecWriteError> for WriteError {
    fn from(value: EnttecWriteError) -> Self {
        if value.0.kind() == std::io::ErrorKind::BrokenPipe {
            Self::Disconnected
        } else {
            Self::Other(value.0.into())
        }
    }
}

// Derive serde for serial port info.

#[derive(Serialize, Deserialize)]
#[serde(remote = "SerialPortInfo")]
struct SerialPortInfoDef {
    pub port_name: String,
    #[serde(with = "SerialPortTypeDef")]
    pub port_type: SerialPortType,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "SerialPortType")]
pub enum SerialPortTypeDef {
    #[serde(with = "UsbPortInfoDef")]
    UsbPort(UsbPortInfo),
    PciPort,
    BluetoothPort,
    Unknown,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "UsbPortInfo")]
pub struct UsbPortInfoDef {
    pub vid: u16,
    pub pid: u16,
    pub serial_number: Option<String>,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
}

#[cfg(test)]
mod test {
    use std::{thread::sleep, time::Duration};

    use super::*;
    use std::error::Error;

    #[test]
    fn test() -> Result<(), Box<dyn Error>> {
        let mut port = EnttecDmxPort::available_ports()?.pop().unwrap();
        println!("{}", port);
        port.open()?;
        for val in 0..255 {
            port.write(&[val][..])?;
            sleep(Duration::from_millis(25));
        }
        port.write(&[0][..])?;
        Ok(())
    }
}
