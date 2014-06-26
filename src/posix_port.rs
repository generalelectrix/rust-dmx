extern crate native;
extern crate libc;

use native::io::file::FileDesc;

use self::libc::types::os::arch::c95::{c_int,c_char};


#[link(name = "ioctrl")]
extern {
	fn open_port_file(path: *c_char) -> c_int;
	fn ioctrl_tiocexcl(fd: c_int) ->  c_int;
	fn tcgetattr(fildes: c_int, termios_p: *mut TermiosPtr) -> c_int;
	fn new_termios() -> *mut TermiosPtr;
	fn free_termios(to_free: *mut TermiosPtr);
	fn clone_termios(to_clone: *mut TermiosPtr) -> *mut TermiosPtr;
	fn tcsetattr_tcsanow(fd: c_int, options: *mut TermiosPtr) -> c_int;
	fn tcflush_io(fd: c_int) -> c_int;
	fn tcdrain(fd: c_int) -> c_int;
	fn ioctrl_tiocmgetandset(fd: c_int) -> c_int;
	fn set_options_enttec(options: *mut TermiosPtr);
}

// phantom class to hold a pointer to a Termios
pub enum TermiosPtr {}

// a Termios holds a pointer to the C struct
// Must never instantiate this except by using Termios::new() and others
// This is handled externally to this module by the private visibility of the field
pub struct Termios {
	target: *mut TermiosPtr
}

impl Termios {
	pub fn new() -> Termios {
		unsafe { Termios{target: new_termios()} }
	}

	pub fn set_as_enttec(&mut self) {
		unsafe { set_options_enttec(self.target); }
	}
}

// clone a Termios by calling the C function to allocate a new one and copy
impl Clone for Termios {
	fn clone(&self) -> Termios {
		unsafe { Termios{target: clone_termios(self.target)} }
	}
}

// go into C and call free
impl Drop for Termios {
	fn drop(&mut self) {
		unsafe { free_termios(self.target); }
	}
}



// "safe" interface to C functions

// open a port file using the C interface
pub fn open_file(path: &str) -> Option<FileDesc> {

	let fd = unsafe {open_port_file(path.to_c_str().unwrap()) };

	if fd >= 0 {
		Some(FileDesc::new(fd, true))
	}
	else {
		None
	}

}

// set the file to have exclusive access, check result for success
pub fn set_exclusive(file: &FileDesc) -> bool {
	let result = unsafe { ioctrl_tiocexcl(file.fd()) };

	if result == 0 {
		true
	}
	else {
		false
	}
}

// try to get the port options
pub fn get_port_options(file: &FileDesc) -> Option<Termios> {
	let options = Termios::new();
	let result = unsafe { tcgetattr(file.fd(), options.target) };
	// get the termios from the port

	// return options if successful
	if result == 0 {
		Some(options)
	}
	else {
		None
	}

}

// try and set the port options
pub fn set_port_options(file: &FileDesc, options: &Termios) -> bool {
	let result = unsafe { tcsetattr_tcsanow(file.fd(), options.target) };
	if result == 0 {
		true
	}
	else {
		false
	}
}

// flush the port; could return success or fail, but it wont fail if port is open
pub fn flush_port(file: &FileDesc) {
	unsafe { tcflush_io(file.fd()); }
}

// wait until the port has finished sending
pub fn drain_port(file: &FileDesc) {
	unsafe { tcdrain(file.fd()); }
}

// set rs485 for sending
// is this necessary?
pub fn set_rs485_for_sending(file: &FileDesc) {
	unsafe {ioctrl_tiocmgetandset(file.fd()); }
}