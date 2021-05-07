# rust-dmx

This library aims to provide a generic trait for a DMX port.
The library only currently supports the Enttec USB DMX Pro (the original, not
the 2-universe MkII). It also provides an offline port placeholder.

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
