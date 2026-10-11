// SPDX-License-Identifier: Apache-2.0

//! Console output (stdout and stderr) kept in the enclave until the keep
//! exits, so that writing to the console does not leave the enclave.
//!
//! Writes are stored in order as records of `[fd: u8][len: u32][bytes]` and
//! written to the host by `exit_group()`, or earlier if the buffer fills up.

use spin::Mutex;

const SIZE: usize = 1 << 20;
const HEADER: usize = 5;

/// The buffered console output.
pub struct Console {
    buf: [u8; SIZE],
    len: usize,
}

/// The console buffer of the keep.
pub static CONSOLE: Mutex<Console> = Mutex::new(Console {
    buf: [0; SIZE],
    len: 0,
});

impl Console {
    /// Append a write of `data` to `fd`. Returns false if it does not fit.
    pub fn push(&mut self, fd: u8, data: &[u8]) -> bool {
        let need = HEADER + data.len();
        if self.len + need > SIZE {
            return false;
        }
        let at = self.len;
        self.buf[at] = fd;
        self.buf[at + 1..at + HEADER].copy_from_slice(&(data.len() as u32).to_le_bytes());
        self.buf[at + HEADER..at + need].copy_from_slice(data);
        self.len += need;
        true
    }

    /// Pass every buffered write to `f`, oldest first, and empty the buffer.
    pub fn drain(&mut self, mut f: impl FnMut(u8, &[u8])) {
        let mut at = 0;
        while at < self.len {
            let fd = self.buf[at];
            let mut len = [0; 4];
            len.copy_from_slice(&self.buf[at + 1..at + HEADER]);
            let end = at + HEADER + u32::from_le_bytes(len) as usize;
            f(fd, &self.buf[at + HEADER..end]);
            at = end;
        }
        self.len = 0;
    }
}
