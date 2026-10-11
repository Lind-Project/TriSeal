// SPDX-License-Identifier: Apache-2.0

//! Console output (stdout and stderr) kept in the enclave until the keep
//! exits, so that writing to the console does not leave the enclave.
//!
//! Writes are stored in order as records of `[fd: u8][len: u32][bytes]` and
//! written to the host by `exit_group()`, or earlier if the buffer fills up,
//! one host write per run of writes to the same fd.

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

    /// Pass the buffered output to `f`, oldest first, and empty the buffer.
    /// Consecutive writes to the same fd are passed as one, so that flushing
    /// takes as few host writes as possible: their bytes are moved together
    /// in place, over the record headers between them.
    pub fn drain(&mut self, mut f: impl FnMut(u8, &[u8])) {
        let mut read = 0;
        while read < self.len {
            let fd = self.buf[read];
            let start = read;
            let mut write = start;
            while read < self.len && self.buf[read] == fd {
                let mut len = [0; 4];
                len.copy_from_slice(&self.buf[read + 1..read + HEADER]);
                let data = read + HEADER;
                let end = data + u32::from_le_bytes(len) as usize;
                // `write` never passes `data`, so this moves bytes left.
                self.buf.copy_within(data..end, write);
                write += end - data;
                read = end;
            }
            f(fd, &self.buf[start..write]);
        }
        self.len = 0;
    }
}
