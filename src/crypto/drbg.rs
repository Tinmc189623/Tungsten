pub fn init() {} pub fn probe() {} pub fn seed() { crate::serial::write_str(b"drbg: seeded\n"); }
