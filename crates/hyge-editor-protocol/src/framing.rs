//! Length-prefixed JSON stream framing.

use std::io::{Read, Write};

use crate::{Envelope, ProtocolIoError, MAX_MESSAGE_BYTES};

/// Writes one length-prefixed envelope to a stream.
pub fn write_envelope<W: Write>(
    writer: &mut W,
    envelope: &Envelope,
) -> Result<(), ProtocolIoError> {
    envelope.validate()?;
    let body = serde_json::to_vec(envelope)?;
    let length = u32::try_from(body.len()).map_err(|_| ProtocolIoError::TooLarge(u32::MAX))?;
    if length == 0 || length > MAX_MESSAGE_BYTES {
        return Err(ProtocolIoError::TooLarge(length));
    }
    writer.write_all(&length.to_be_bytes())?;
    writer.write_all(&body)?;
    writer.flush()?;
    Ok(())
}

/// Writes one structurally valid envelope without requiring the current
/// protocol version. This is used for compatibility errors during handshake.
pub fn write_frame<W: Write>(writer: &mut W, envelope: &Envelope) -> Result<(), ProtocolIoError> {
    envelope.validate_structure()?;
    write_body(writer, envelope)
}

/// Reads one length-prefixed envelope from a stream.
pub fn read_envelope<R: Read>(reader: &mut R) -> Result<Envelope, ProtocolIoError> {
    let envelope = read_frame(reader)?;
    envelope.validate()?;
    Ok(envelope)
}

/// Reads one structurally valid envelope without requiring the current
/// protocol version.
pub fn read_frame<R: Read>(reader: &mut R) -> Result<Envelope, ProtocolIoError> {
    let mut header = [0_u8; 4];
    reader.read_exact(&mut header)?;
    let length = u32::from_be_bytes(header);
    if length == 0 {
        return Err(ProtocolIoError::InvalidEnvelope(
            "message body must not be empty",
        ));
    }
    if length > MAX_MESSAGE_BYTES {
        return Err(ProtocolIoError::TooLarge(length));
    }
    let mut body = vec![0_u8; length as usize];
    reader.read_exact(&mut body)?;
    let envelope: Envelope = serde_json::from_slice(&body)?;
    envelope.validate_structure()?;
    Ok(envelope)
}

fn write_body<W: Write>(writer: &mut W, envelope: &Envelope) -> Result<(), ProtocolIoError> {
    let body = serde_json::to_vec(envelope)?;
    let length = u32::try_from(body.len()).map_err(|_| ProtocolIoError::TooLarge(u32::MAX))?;
    if length == 0 || length > MAX_MESSAGE_BYTES {
        return Err(ProtocolIoError::TooLarge(length));
    }
    writer.write_all(&length.to_be_bytes())?;
    writer.write_all(&body)?;
    writer.flush()?;
    Ok(())
}
