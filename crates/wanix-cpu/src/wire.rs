//! The [`CpuJobSpec`] request framing the caller sends on the control stream.
//!
//! Before the acceptor delivers any [`crate::CpuEvent`], the caller sends the job
//! spec so the acceptor knows what to run. The framing is the same tiny,
//! self-contained, length-prefixed style as [`crate::CpuEvent`] — a sequence of
//! length-prefixed UTF-8 strings: `kind`, `program`, `cwd`, then the argv and env
//! lists each prefixed by their element count. Every length is bounded on decode
//! against [`MAX_FIELD_LEN`] / [`MAX_LIST_LEN`] so an untrusted caller cannot
//! force an unbounded allocation.

use std::io::{Read, Write};

use crate::spec::CpuJobSpec;

/// Upper bound on a single encoded string field (a path, arg, or env line).
pub const MAX_FIELD_LEN: usize = 64 * 1024;
/// Upper bound on the number of elements in the argv or env list.
pub const MAX_LIST_LEN: usize = 4096;

/// Writes `spec` as a length-prefixed request frame to `writer`.
///
/// # Errors
///
/// Returns an I/O error when a field is too long for the wire or the write fails.
pub fn write_spec<W: Write>(writer: &mut W, spec: &CpuJobSpec) -> std::io::Result<()> {
    write_field(writer, &spec.kind)?;
    write_field(writer, &spec.program)?;
    write_field(writer, spec.cwd.as_str())?;
    write_list(writer, &spec.args)?;
    write_list(writer, &spec.env)?;
    writer.flush()
}

/// Reads a length-prefixed [`CpuJobSpec`] request frame from `reader`.
///
/// # Errors
///
/// Returns an I/O error on a transport failure, a truncated frame, a field or
/// list length over its ceiling, invalid UTF-8, or a `cwd` that is not a valid
/// normalized path.
pub fn read_spec<R: Read>(reader: &mut R) -> std::io::Result<CpuJobSpec> {
    let kind = read_field(reader)?;
    let program = read_field(reader)?;
    let cwd = read_field(reader)?;
    let args = read_list(reader)?;
    let env = read_list(reader)?;
    let spec = CpuJobSpec::new(kind, program)
        .and_then(|spec| spec.with_cwd(cwd))
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err.to_string()))?;
    Ok(spec.with_args(args).with_env(env))
}

/// Writes one length-prefixed UTF-8 field.
fn write_field<W: Write>(writer: &mut W, value: &str) -> std::io::Result<()> {
    let len = u32::try_from(value.len()).map_err(|_| too_long())?;
    if value.len() > MAX_FIELD_LEN {
        return Err(too_long());
    }
    writer.write_all(&len.to_le_bytes())?;
    writer.write_all(value.as_bytes())
}

/// Reads one length-prefixed UTF-8 field, enforcing the field ceiling.
fn read_field<R: Read>(reader: &mut R) -> std::io::Result<String> {
    let mut len_bytes = [0_u8; 4];
    reader.read_exact(&mut len_bytes)?;
    let len = u32::from_le_bytes(len_bytes) as usize;
    if len > MAX_FIELD_LEN {
        return Err(too_long());
    }
    let mut bytes = vec![0_u8; len];
    reader.read_exact(&mut bytes)?;
    String::from_utf8(bytes)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err.to_string()))
}

/// Writes a count-prefixed list of length-prefixed UTF-8 fields.
fn write_list<W: Write>(writer: &mut W, values: &[String]) -> std::io::Result<()> {
    let count = u32::try_from(values.len()).map_err(|_| too_long())?;
    if values.len() > MAX_LIST_LEN {
        return Err(too_long());
    }
    writer.write_all(&count.to_le_bytes())?;
    for value in values {
        write_field(writer, value)?;
    }
    Ok(())
}

/// Reads a count-prefixed list of fields, enforcing the list ceiling.
fn read_list<R: Read>(reader: &mut R) -> std::io::Result<Vec<String>> {
    let mut count_bytes = [0_u8; 4];
    reader.read_exact(&mut count_bytes)?;
    let count = u32::from_le_bytes(count_bytes) as usize;
    if count > MAX_LIST_LEN {
        return Err(too_long());
    }
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(read_field(reader)?);
    }
    Ok(values)
}

/// An "encoded element exceeds its ceiling" error.
fn too_long() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "cpu spec field or list exceeded its wire ceiling",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_round_trips_through_the_wire() {
        let spec = CpuJobSpec::new("qjs", "build.js")
            .unwrap()
            .with_args(vec!["--flag".to_owned(), "value".to_owned()])
            .with_env(vec!["K=v".to_owned()])
            .with_cwd("work")
            .unwrap();
        let mut buf = Vec::new();
        write_spec(&mut buf, &spec).unwrap();
        let mut cursor = std::io::Cursor::new(buf);
        assert_eq!(read_spec(&mut cursor).unwrap(), spec);
    }

    #[test]
    fn an_empty_argv_and_env_round_trip() {
        let spec = CpuJobSpec::new("noop", ".").unwrap();
        let mut buf = Vec::new();
        write_spec(&mut buf, &spec).unwrap();
        let mut cursor = std::io::Cursor::new(buf);
        assert_eq!(read_spec(&mut cursor).unwrap(), spec);
    }

    #[test]
    fn an_oversize_list_count_is_rejected_without_allocating() {
        // kind="x", program="y", cwd=".", then an argv count past the ceiling.
        let mut buf = Vec::new();
        write_field(&mut buf, "x").unwrap();
        write_field(&mut buf, "y").unwrap();
        write_field(&mut buf, ".").unwrap();
        let count = u32::try_from(MAX_LIST_LEN + 1).unwrap();
        buf.extend_from_slice(&count.to_le_bytes());
        let mut cursor = std::io::Cursor::new(buf);
        assert!(read_spec(&mut cursor).is_err());
    }
}
