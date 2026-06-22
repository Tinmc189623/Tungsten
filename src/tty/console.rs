pub fn init() {} pub fn probe() {} pub fn init_tty() { crate::serial::write_str(b"tty: console init\n"); }
