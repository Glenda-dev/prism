pub struct Utf8Decoder {
    buffer: alloc::vec::Vec<u8>,
}

impl Utf8Decoder {
    pub fn new() -> Self {
        Self { buffer: alloc::vec::Vec::with_capacity(4) }
    }

    /// Process a single byte and potentially return a decoded string.
    pub fn process_byte(&mut self, b: u8) -> Option<alloc::string::String> {
        self.buffer.push(b);
        debug!("Utf8Decoder: processing byte {:#x}, buffer len: {}", b, self.buffer.len());

        match core::str::from_utf8(&self.buffer) {
            Ok(s) => {
                let decoded = alloc::string::String::from(s);
                debug!("Utf8Decoder: successfully decoded \"{}\"", decoded);
                self.buffer.clear();
                Some(decoded)
            }
            Err(e) => {
                if let Some(valid_up_to) = Some(e.valid_up_to()) {
                    if valid_up_to > 0 {
                        // This shouldn't happen if we clear on success, but for robustness:
                        debug!(
                            "Utf8Decoder: partial valid data found (should not happen), clearing up to {}",
                            valid_up_to
                        );
                        self.buffer.drain(0..valid_up_to);
                    }
                }

                if e.error_len().is_some() {
                    // Invalid sequence
                    debug!("Utf8Decoder: invalid sequence detected, clearing buffer");
                    self.buffer.clear();
                    None
                } else {
                    // Incomplete sequence
                    debug!("Utf8Decoder: incomplete sequence, waiting for more bytes");
                    None
                }
            }
        }
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
    }
}
