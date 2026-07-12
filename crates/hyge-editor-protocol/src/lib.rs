//! Versioned wire contract for the Hyge editor.
//!
//! The protocol deliberately contains no ECS or renderer types. The Rust
//! service translates engine state into these stable JSON messages, while a
//! Qt/PySide frontend consumes them without linking to the engine ABI.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod envelope;
mod error;
mod framing;

pub use envelope::{Envelope, MessageType, ProtocolError};
pub use error::ProtocolIoError;
pub use framing::{read_envelope, read_frame, write_envelope, write_frame};

/// Current wire protocol version.
pub const PROTOCOL_VERSION: u32 = 2;
/// Protocol versions accepted for compatibility negotiation.
pub const SUPPORTED_PROTOCOL_VERSIONS: &[u32] = &[1, PROTOCOL_VERSION];

/// Maximum accepted JSON message size (16 MiB).
pub const MAX_MESSAGE_BYTES: u32 = 16 * 1024 * 1024;

#[cfg(test)]
mod tests {
    use std::io::{self, Cursor, Read};

    use super::*;

    #[test]
    fn round_trips_length_prefixed_envelope() {
        let envelope = Envelope::new(
            "42",
            MessageType::Hello,
            serde_json::json!({"session_token": "pytest"}),
        );
        let mut bytes = Vec::new();
        write_envelope(&mut bytes, &envelope).expect("write must succeed");
        let decoded = read_envelope(&mut Cursor::new(bytes)).expect("read must succeed");
        assert_eq!(decoded, envelope);
    }

    #[test]
    fn reads_partial_streams() {
        struct Chunked(Cursor<Vec<u8>>);

        impl Read for Chunked {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                let mut one = [0_u8; 1];
                let read = self.0.read(&mut one)?;
                if read == 1 {
                    buf[0] = one[0];
                }
                Ok(read)
            }
        }

        let envelope = Envelope::new("partial", MessageType::OpenProject, serde_json::json!({}));
        let mut bytes = Vec::new();
        write_envelope(&mut bytes, &envelope).expect("write must succeed");
        let decoded = read_envelope(&mut Chunked(Cursor::new(bytes))).expect("read must succeed");
        assert_eq!(decoded, envelope);
    }

    #[test]
    fn rejects_wrong_protocol_version() {
        let mut envelope = Envelope::new("1", MessageType::Hello, serde_json::json!({}));
        envelope.protocol_version = PROTOCOL_VERSION + 1;
        let error = write_envelope(&mut Vec::new(), &envelope).expect_err("version must fail");
        assert!(matches!(error, ProtocolIoError::UnsupportedVersion(3)));
    }

    #[test]
    fn rejects_empty_message_id_and_non_object_payload() {
        let empty_id = Envelope::new("", MessageType::Hello, serde_json::json!({}));
        assert!(matches!(
            write_envelope(&mut Vec::new(), &empty_id),
            Err(ProtocolIoError::InvalidEnvelope(_))
        ));

        let mut non_object = Envelope::new("1", MessageType::Hello, serde_json::json!({}));
        non_object.payload = serde_json::Value::Null;
        assert!(matches!(
            write_envelope(&mut Vec::new(), &non_object),
            Err(ProtocolIoError::InvalidEnvelope(_))
        ));
    }

    #[test]
    fn rejects_invalid_json_and_oversized_frame_before_allocation() {
        let mut invalid = Vec::new();
        invalid.extend_from_slice(&3_u32.to_be_bytes());
        invalid.extend_from_slice(b"bad");
        assert!(matches!(
            read_envelope(&mut Cursor::new(invalid)),
            Err(ProtocolIoError::Json(_))
        ));

        let oversized = MAX_MESSAGE_BYTES + 1;
        assert!(matches!(
            read_envelope(&mut Cursor::new(oversized.to_be_bytes())),
            Err(ProtocolIoError::TooLarge(value)) if value == oversized
        ));
    }

    #[test]
    fn error_response_is_machine_readable() {
        let envelope = Envelope::error("7", "invalid_request", "bad payload");
        let error = envelope.error.as_ref().expect("error exists");
        assert_eq!(error.code, "invalid_request");
        assert!(!error.recoverable);
        assert_eq!(envelope.message_type, MessageType::EngineError);
    }

    #[test]
    fn diagnostic_error_round_trips_recovery_metadata() {
        let envelope = Envelope::diagnostic_error(
            "7",
            "scene_load_failed",
            "scene could not be decoded",
            true,
            Some("main.hyge-world".into()),
            Some("open_scene".into()),
            Some("repair the scene and retry".into()),
        );
        assert_eq!(envelope.message_type, MessageType::EngineError);
        let mut bytes = Vec::new();
        write_envelope(&mut bytes, &envelope).expect("write diagnostic");
        let decoded = read_envelope(&mut Cursor::new(bytes)).expect("read diagnostic");
        let error = decoded.error.expect("diagnostic error");
        assert!(error.recoverable);
        assert_eq!(error.path.as_deref(), Some("main.hyge-world"));
        assert_eq!(error.operation.as_deref(), Some("open_scene"));
    }

    #[test]
    fn structurally_reads_unknown_versions_for_negotiation() {
        let mut envelope = Envelope::new("future", MessageType::Hello, serde_json::json!({}));
        envelope.protocol_version = PROTOCOL_VERSION + 1;
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &envelope).expect("structural frame must write");
        let decoded = read_frame(&mut Cursor::new(bytes)).expect("structural frame must read");
        assert_eq!(decoded.protocol_version, PROTOCOL_VERSION + 1);
        let mut strict_bytes = Vec::new();
        write_frame(&mut strict_bytes, &decoded).expect("structural frame must write");
        assert!(matches!(
            read_envelope(&mut Cursor::new(strict_bytes)),
            Err(ProtocolIoError::UnsupportedVersion(3))
        ));
    }
}
