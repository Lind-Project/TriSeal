// SPDX-License-Identifier: Apache-2.0

//! Console output (stdout and stderr) kept until the keep exits, so that
//! writing to the console does not leave the enclave.
//!
//! Writes are stored in order as records of `[fd: u8][len: u32][bytes]`.
//! At first they are kept in the enclave and written to the host by
//! `exit_group()`, or earlier if the buffer fills up, one host write per run
//! of writes to the same fd. Once the host has given the keep memory for the
//! console ([`Console::share`]), records go there instead, after a `u64`
//! count of the bytes used, and the host writes them out itself when the
//! keep exits: not even `exit_group()` has to write the console then.

use spin::Mutex;

const SIZE: usize = 1 << 20;
const HEADER: usize = 5;

/// The buffered console output.
pub struct Console {
    buf: [u8; SIZE],
    len: usize,
    /// Memory shared with the host, and its size: `(0, 0)` until shared.
    shared: (usize, usize),
    /// Bytes of records in the shared memory. The count there is written
    /// from this one, never read back.
    shared_len: usize,
}

/// The console buffer of the keep.
pub static CONSOLE: Mutex<Console> = Mutex::new(Console {
    buf: [0; SIZE],
    len: 0,
    shared: (0, 0),
    shared_len: 0,
});

impl Console {
    /// Whether the console is in memory shared with the host.
    pub fn is_shared(&self) -> bool {
        self.shared.0 != 0
    }

    /// Keep the console in `size` bytes of host memory at `addr` (checked to
    /// be outside the enclave) from now on, moving what is buffered there.
    pub fn share(&mut self, addr: usize, size: usize) {
        if self.is_shared() || size < 8 + self.len {
            return;
        }
        self.shared = (addr, size);
        let len = self.len;
        // Safety: the memory is outside the enclave and at least `8 + len`
        // bytes long.
        unsafe { core::ptr::copy_nonoverlapping(self.buf.as_ptr(), (addr + 8) as *mut u8, len) };
        self.shared_len = len;
        self.len = 0;
        self.store_shared_len();
    }

    /// The `len` bytes of shared memory after the records written so far.
    fn shared_records(&mut self, len: usize) -> &mut [u8] {
        let (addr, _) = self.shared;
        // Safety: the memory is outside the enclave, mapped for the life of
        // the keep and only written by this console.
        unsafe { core::slice::from_raw_parts_mut((addr + 8 + self.shared_len) as *mut u8, len) }
    }

    fn store_shared_len(&mut self) {
        let (addr, _) = self.shared;
        // Safety: as above.
        unsafe { (addr as *mut u64).write_volatile(self.shared_len as u64) };
    }

    /// Append a write of `data` to `fd`. Returns false if it does not fit.
    pub fn push(&mut self, fd: u8, data: &[u8]) -> bool {
        let need = HEADER + data.len();
        if self.is_shared() {
            if 8 + self.shared_len + need > self.shared.1 {
                return false;
            }
            let record = self.shared_records(need);
            record[0] = fd;
            record[1..HEADER].copy_from_slice(&(data.len() as u32).to_le_bytes());
            record[HEADER..].copy_from_slice(data);
            self.shared_len += need;
            self.store_shared_len();
            return true;
        }
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
        if self.is_shared() {
            // Rare: the shared memory is full. Write it out in order.
            let len = self.shared_len;
            self.shared_len = 0;
            let records: &[u8] = self.shared_records(len);
            let mut at = 0;
            while at + HEADER <= records.len() {
                let mut n = [0; 4];
                n.copy_from_slice(&records[at + 1..at + HEADER]);
                let end = (at + HEADER + u32::from_le_bytes(n) as usize).min(records.len());
                f(records[at], &records[at + HEADER..end]);
                at = end;
            }
            self.store_shared_len();
            return;
        }
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
