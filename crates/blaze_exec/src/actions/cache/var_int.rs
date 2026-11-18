//! Port of `lib/util/VarInt.java` (protobuf-style variable-length integers) and the subset of
//! `java.nio.ByteBuffer` the action cache decodes with.
//!
//! PORT: `VarInt` belongs to `lib/util`; it lives here until `blaze_util` ports it.

use std::fmt;
use std::io::{self, Write};

/// Port of `java.nio.BufferUnderflowException`: a read past the end of a [`ByteBuffer`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BufferUnderflowException;

impl fmt::Display for BufferUnderflowException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("java.nio.BufferUnderflowException")
    }
}

impl std::error::Error for BufferUnderflowException {}

impl From<BufferUnderflowException> for io::Error {
    fn from(e: BufferUnderflowException) -> io::Error {
        io::Error::new(io::ErrorKind::UnexpectedEof, e)
    }
}

/// Whether `e` wraps a [`BufferUnderflowException`] (Java `catch (BufferUnderflowException e)`).
pub fn is_buffer_underflow(e: &io::Error) -> bool {
    e.get_ref()
        .is_some_and(|inner| inner.downcast_ref::<BufferUnderflowException>().is_some())
}

/// The read side of `java.nio.ByteBuffer.wrap(byte[])`: a cursor over a byte slice.
#[derive(Debug)]
pub struct ByteBuffer<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> ByteBuffer<'a> {
    /// `ByteBuffer.wrap(bytes)`.
    pub fn wrap(bytes: &'a [u8]) -> ByteBuffer<'a> {
        ByteBuffer { bytes, position: 0 }
    }

    /// `get()`: the next byte (signed, as Java's `byte`).
    pub fn get(&mut self) -> Result<i8, BufferUnderflowException> {
        match self.bytes.get(self.position) {
            Some(b) => {
                self.position += 1;
                Ok(*b as i8)
            }
            None => Err(BufferUnderflowException),
        }
    }

    /// `get(byte[] dst)`: fills `dst` or fails without consuming anything.
    pub fn get_bytes(&mut self, dst: &mut [u8]) -> Result<(), BufferUnderflowException> {
        if dst.len() > self.remaining() {
            return Err(BufferUnderflowException);
        }
        dst.copy_from_slice(&self.bytes[self.position..self.position + dst.len()]);
        self.position += dst.len();
        Ok(())
    }

    /// `asIntBuffer().get()`: the next big-endian 32-bit int.
    pub fn get_int(&mut self) -> Result<i32, BufferUnderflowException> {
        let mut buf = [0u8; 4];
        self.get_bytes(&mut buf)?;
        Ok(i32::from_be_bytes(buf))
    }

    /// `remaining()`.
    pub fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }
}

/// Port of `VarInt`.
// Java: com/google/devtools/build/lib/util/VarInt.java:24 VarInt
pub struct VarInt;

impl VarInt {
    /// Maximum encoded size of 32-bit positive integers (in bytes).
    // Java: com/google/devtools/build/lib/util/VarInt.java:29 VarInt.MAX_VARINT_SIZE
    pub const MAX_VARINT_SIZE: i32 = 5;

    /// Maximum encoded size of 64-bit longs, and negative 32-bit ints (in bytes).
    // Java: com/google/devtools/build/lib/util/VarInt.java:34 VarInt.MAX_VARLONG_SIZE
    pub const MAX_VARLONG_SIZE: i32 = 10;

    /// Returns the encoding size in bytes of its input value.
    // Java: com/google/devtools/build/lib/util/VarInt.java:42 VarInt.varIntSize
    pub fn var_int_size(i: i32) -> i32 {
        let mut i = i as u32;
        let mut result = 0;
        loop {
            result += 1;
            i >>= 7;
            if i == 0 {
                break;
            }
        }
        result
    }

    /// Reads a varint from `src`, advances its position, and returns the value.
    // Java: com/google/devtools/build/lib/util/VarInt.java:60 VarInt.getVarInt
    pub fn get_var_int(src: &mut ByteBuffer<'_>) -> Result<i32, BufferUnderflowException> {
        let mut tmp: i32 = src.get()? as i32;
        if tmp >= 0 {
            return Ok(tmp);
        }
        let mut result: i32 = tmp & 0x7f;
        tmp = src.get()? as i32;
        if tmp >= 0 {
            result |= tmp << 7;
        } else {
            result |= (tmp & 0x7f) << 7;
            tmp = src.get()? as i32;
            if tmp >= 0 {
                result |= tmp << 14;
            } else {
                result |= (tmp & 0x7f) << 14;
                tmp = src.get()? as i32;
                if tmp >= 0 {
                    result |= tmp << 21;
                } else {
                    result |= (tmp & 0x7f) << 21;
                    tmp = src.get()? as i32;
                    result |= tmp.wrapping_shl(28);
                    while tmp < 0 {
                        // We get into this loop only in the case of overflow.
                        // By doing this, we can call getVarInt() instead of
                        // getVarLong() when we only need an int.
                        tmp = src.get()? as i32;
                    }
                }
            }
        }
        Ok(result)
    }

    /// Encodes an integer in a variable-length encoding, 7 bits per byte, into `sink` at
    /// `offset`; returns the offset just past the encoded value.
    // Java: com/google/devtools/build/lib/util/VarInt.java:100 VarInt.putVarInt
    pub fn put_var_int_into(v: i32, sink: &mut [u8], mut offset: usize) -> usize {
        let mut v = v as u32;
        loop {
            // Encode next 7 bits + terminator bit
            let bits = v & 0x7F;
            v >>= 7;
            let b = (bits + if v != 0 { 0x80 } else { 0 }) as u8;
            sink[offset] = b;
            offset += 1;
            if v == 0 {
                break;
            }
        }
        offset
    }

    /// Encodes an integer in a variable-length encoding, 7 bits per byte, to `output_stream`.
    // Java: com/google/devtools/build/lib/util/VarInt.java:118 VarInt.putVarInt
    pub fn put_var_int(v: i32, output_stream: &mut dyn Write) -> io::Result<()> {
        let mut bytes = vec![0u8; VarInt::var_int_size(v) as usize];
        VarInt::put_var_int_into(v, &mut bytes, 0);
        output_stream.write_all(&bytes)
    }

    /// Returns the encoding size in bytes of its input value.
    // Java: com/google/devtools/build/lib/util/VarInt.java:130 VarInt.varLongSize
    pub fn var_long_size(v: i64) -> i32 {
        let mut v = v as u64;
        let mut result = 0;
        loop {
            result += 1;
            v >>= 7;
            if v == 0 {
                break;
            }
        }
        result
    }

    /// Reads an up to 64 bit long varint from `src`, advances its position, and returns the
    /// value.
    // Java: com/google/devtools/build/lib/util/VarInt.java:149 VarInt.getVarLong
    pub fn get_var_long(src: &mut ByteBuffer<'_>) -> Result<i64, BufferUnderflowException> {
        let mut tmp: i64 = src.get()? as i64;
        if tmp >= 0 {
            return Ok(tmp);
        }
        let mut result: i64 = tmp & 0x7f;
        // Each further byte adds 7 bits; the Java code unrolls this nesting.
        let mut shift = 7;
        while shift <= 56 {
            tmp = src.get()? as i64;
            if tmp >= 0 {
                result |= tmp << shift;
                return Ok(result);
            }
            result |= (tmp & 0x7f) << shift;
            shift += 7;
        }
        result |= (src.get()? as i64).wrapping_shl(63);
        Ok(result)
    }

    /// Encodes a long integer in a variable-length encoding, 7 bits per byte, into `sink`.
    // Java: com/google/devtools/build/lib/util/VarInt.java:205 VarInt.putVarLong
    pub fn put_var_long_into(v: i64, sink: &mut Vec<u8>) {
        let mut v = v as u64;
        loop {
            let bits = (v as u32) & 0x7f;
            v >>= 7;
            if v == 0 {
                sink.push(bits as u8);
                return;
            }
            sink.push((bits | 0x80) as u8);
        }
    }

    /// Encodes a long integer in a variable-length encoding, 7 bits per byte, to
    /// `output_stream`.
    // Java: com/google/devtools/build/lib/util/VarInt.java:217 VarInt.putVarLong
    pub fn put_var_long(v: i64, output_stream: &mut dyn Write) -> io::Result<()> {
        let mut bytes = Vec::with_capacity(VarInt::var_long_size(v) as usize);
        VarInt::put_var_long_into(v, &mut bytes);
        output_stream.write_all(&bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_ints() {
        for v in [0, 1, 127, 128, 300, i32::MAX, -1, i32::MIN] {
            let mut out = Vec::new();
            VarInt::put_var_int(v, &mut out).unwrap();
            assert_eq!(out.len() as i32, VarInt::var_int_size(v));
            let mut buf = ByteBuffer::wrap(&out);
            assert_eq!(VarInt::get_var_int(&mut buf).unwrap(), v);
            assert_eq!(buf.remaining(), 0);
        }
    }

    #[test]
    fn round_trip_longs() {
        for v in [0i64, 1, 127, 128, 1 << 40, i64::MAX, -1, i64::MIN] {
            let mut out = Vec::new();
            VarInt::put_var_long(v, &mut out).unwrap();
            assert_eq!(out.len() as i32, VarInt::var_long_size(v));
            let mut buf = ByteBuffer::wrap(&out);
            assert_eq!(VarInt::get_var_long(&mut buf).unwrap(), v);
            assert_eq!(buf.remaining(), 0);
        }
    }
}
