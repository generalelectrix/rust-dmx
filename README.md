# rust-dmx

This library aims to provide a generic trait for a DMX port.
It currently supports:

- Enttec DMX USB Pro (and compatible FTDI-based units such as made by DMXKing).
- ArtNet
- an offline placeholder

## Usage

Use the `available_ports` function to get a listing of all available ports.
The port must be opened before use.

```rust
use rust_dmx::{available_ports, DmxPort};

let port = available_ports()?[0];
port.open()?;
port.write(&[0, 1, 2, 3][..])?;
```

Ports can be serialized/deserialized, maintaining their identity. They will
need to be re-opened after deserialization.
