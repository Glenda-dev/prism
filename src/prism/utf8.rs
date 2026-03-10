pub struct Utf8Decoder {
    buffer: alloc::vec::Vec<u8>,
}

impl Utf8Decoder {
    pub fn new() -> Self {
        Self { buffer: alloc::vec::Vec::with_capacity(4) }
    }

    /// Process a single byte and potentially return decoded strings.
    pub fn process_byte(&mut self, b: u8) -> alloc::string::String {
        self.buffer.push(b);

        let mut result = alloc::string::String::new();

        loop {
            if self.buffer.is_empty() {
                break;
            }

            match core::str::from_utf8(&self.buffer) {
                Ok(s) => {
                    result.push_str(s);
                    self.buffer.clear();
                    break;
                }
                Err(e) => {
                    let valid_up_to = e.valid_up_to();
                    let error_len = e.error_len();

                    if valid_up_to > 0 {
                        let valid_str = core::str::from_utf8(&self.buffer[..valid_up_to]).unwrap();
                        result.push_str(valid_str);
                        self.buffer.drain(0..valid_up_to);
                    } else if let Some(len) = error_len {
                        // Invalid sequence
                        self.buffer.drain(0..len);
                    } else {
                        // Incomplete sequence
                        break;
                    }
                }
            }
        }

        result
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
    }
}
