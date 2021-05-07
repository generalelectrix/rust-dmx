//! Implementation of support for the Enttec USB DMX Pro dongle.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::io::Write;
use std::time::Duration;
use std::{cmp::min, fmt};

use crate::{PortListing, PortOpener};

use super::{DmxPort, Error};
use serialport::{available_ports, new, SerialPort, SerialPortInfo, SerialPortType, UsbPortInfo};

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

/// Format a byte buffer as an enttec message into the provided output buffer.
/// Maximum valid size for payload is 600; no check is made here that the payload is within this range.
fn make_packet(message_type: u8, payload: &[u8], output: &mut Vec<u8>) {
    // Enttec messages are the size of the payload plus 5 bytes for type, length, and framing.
    let payload_size = payload.len();
    output.clear();
    output.reserve(payload_size + 5);
    let (len_lsb, len_msb) = (payload_size as u8, (payload_size >> 8) as u8);
    output.push(START_VAL);
    output.push(message_type);
    output.push(len_lsb);
    output.push(len_msb);
    output.extend_from_slice(payload);
    output.push(END_VAL);
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
    fn as_packet(&self, output: &mut Vec<u8>) {
        let payload = [
            self.break_time,
            self.mark_after_break_time,
            self.output_rate,
        ];
        make_packet(SET_PARAMETERS, &payload, output)
    }
}

#[derive(Serialize, Deserialize)]
pub struct EnttecDmxPort {
    params: EnttecParams,
    #[serde(skip)]
    port: Option<Box<dyn SerialPort>>,
    #[serde(with = "SerialPortInfoDef")]
    info: SerialPortInfo,
    #[serde(skip)]
    output_buffer: Vec<u8>,
}

impl EnttecDmxPort {
    /// Create an enttec port.
    /// The port is not opened yet.
    pub fn new(info: SerialPortInfo) -> Self {
        let params = EnttecParams::default();

        Self {
            params: params,
            port: None,
            info,
            output_buffer: Vec::new(),
        }
    }

    /// Create an enttec port and open it.
    pub fn opened(info: SerialPortInfo) -> Result<Self, Error> {
        let mut port = Self::new(info);
        port.open()?;
        Ok(port)
    }

    /// Write the current parameters out to the port.
    fn write_params(&mut self) -> Result<(), Error> {
        self.params.as_packet(&mut self.output_buffer);
        self.write_output_buffer()
    }

    /// Write the current contents of the output buffer to the port.
    fn write_output_buffer(&mut self) -> Result<(), Error> {
        match &mut self.port {
            Some(p) => p.write_all(&self.output_buffer)?,
            None => return Err(Error::PortClosed),
        }
        Ok(())
    }
}

#[typetag::serde]
impl DmxPort for EnttecDmxPort {
    /// Return the available enttec ports connected to this system.
    /// TODO: provide a mechanism to specialize this implementation depending on platform.
    fn available_ports() -> Result<PortListing, Error> {
        Ok(available_ports()?
            .into_iter()
            .filter(|info| {
                if let SerialPortType::UsbPort(usb_port_info) = &info.port_type {
                    if let Some(product) = &usb_port_info.product {
                        return product == "DMX USB PRO";
                    }
                }
                false
            })
            .map(|info| {
                let open_info = info.clone();
                let opener: Box<PortOpener> = Box::new(move || {
                    EnttecDmxPort::opened(open_info.clone())
                        .map(|p| Box::new(p) as Box<dyn DmxPort>)
                });
                (info, opener)
            })
            .collect())
    }

    /// Open the port.
    fn open(&mut self) -> Result<(), Error> {
        if self.port.is_some() {
            return Ok(());
        }

        // baud rate is not used on FTDI
        let mut port = new(&self.info.port_name, 57600)
            .timeout(Duration::from_millis(1))
            .open()?;

        self.port = Some(port);

        // send the default parameters to the port
        if let Err(e) = self.write_params() {
            self.port = None;
            return Err(e);
        }
        Ok(())
    }

    fn close(&mut self) {
        self.port = None;
    }

    fn write(&mut self, frame: &[u8]) -> Result<(), Error> {
        let size = frame.len();
        if size < MIN_UNIVERSE_SIZE {
            let mut padded_frame = Vec::with_capacity(MIN_UNIVERSE_SIZE);
            padded_frame.extend_from_slice(frame);
            padded_frame.resize(MIN_UNIVERSE_SIZE, 0);
            make_packet(SEND_DMX_PACKET, &padded_frame, &mut self.output_buffer)
        } else {
            make_packet(
                SEND_DMX_PACKET,
                &frame[0..min(size, MAX_UNIVERSE_SIZE)],
                &mut self.output_buffer,
            )
        }

        self.write_output_buffer()
    }
}

impl fmt::Display for EnttecDmxPort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.info.port_type {
            SerialPortType::UsbPort(p) => {
                if let Some(sn) = &p.serial_number {
                    return write!(f, "Enttec DMX USB PRO {}", sn);
                }
            }
            _ => (),
        }
        write!(f, "Enttec DMX USB PRO {}", self.info.port_name)
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
