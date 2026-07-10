use hickory_proto::op::{Message, MessageType, OpCode, ResponseCode};
use hickory_proto::serialize::binary::{BinEncodable as _, BinEncoder};
use thiserror::Error;

pub const EXIT_DNS_FIPS_SERVICE_PORT: u16 = 7_370;
pub const EXIT_DNS_MAX_PACKET_BYTES: usize = 4_096;

const EXIT_DNS_MAGIC: &[u8; 4] = b"NDNS";
const EXIT_DNS_VERSION: u8 = 1;
const EXIT_DNS_REQUEST: u8 = 1;
const EXIT_DNS_RESPONSE: u8 = 2;
const EXIT_DNS_HEADER_BYTES: usize = 18;
const DNS_HEADER_BYTES: usize = 12;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExitDnsRequest {
    pub nonce: u64,
    pub transaction_id: u16,
    pub query: Vec<u8>,
}

impl ExitDnsRequest {
    pub fn new(nonce: u64, query: Vec<u8>) -> Result<Self, ExitDnsCodecError> {
        let transaction_id = validate_dns_packet(&query, MessageType::Query)?;
        Ok(Self {
            nonce,
            transaction_id,
            query,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExitDnsResponse {
    pub nonce: u64,
    pub transaction_id: u16,
    pub response: Vec<u8>,
}

impl ExitDnsResponse {
    pub fn new(
        nonce: u64,
        transaction_id: u16,
        response: Vec<u8>,
    ) -> Result<Self, ExitDnsCodecError> {
        let actual = validate_dns_packet(&response, MessageType::Response)?;
        if actual != transaction_id {
            return Err(ExitDnsCodecError::TransactionMismatch {
                expected: transaction_id,
                actual,
            });
        }
        Ok(Self {
            nonce,
            transaction_id,
            response,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitDnsMessage {
    Request(ExitDnsRequest),
    Response(ExitDnsResponse),
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ExitDnsCodecError {
    #[error("exit DNS frame is shorter than its fixed header")]
    Truncated,
    #[error("exit DNS frame magic is invalid")]
    InvalidMagic,
    #[error("unsupported exit DNS frame version {0}")]
    UnsupportedVersion(u8),
    #[error("unsupported exit DNS frame type {0}")]
    UnsupportedType(u8),
    #[error("exit DNS frame payload length does not match its header")]
    InvalidLength,
    #[error("DNS packet length {len} is outside {min}..={max} bytes")]
    DnsPacketLength { len: usize, min: usize, max: usize },
    #[error("DNS packet is malformed")]
    InvalidDnsPacket,
    #[error("DNS packet has the wrong message type")]
    InvalidDnsMessageType,
    #[error("DNS transaction mismatch: expected {expected}, got {actual}")]
    TransactionMismatch { expected: u16, actual: u16 },
}

pub fn encode_exit_dns_message(message: &ExitDnsMessage) -> Result<Vec<u8>, ExitDnsCodecError> {
    let (kind, nonce, transaction_id, payload, expected_type) = match message {
        ExitDnsMessage::Request(request) => (
            EXIT_DNS_REQUEST,
            request.nonce,
            request.transaction_id,
            request.query.as_slice(),
            MessageType::Query,
        ),
        ExitDnsMessage::Response(response) => (
            EXIT_DNS_RESPONSE,
            response.nonce,
            response.transaction_id,
            response.response.as_slice(),
            MessageType::Response,
        ),
    };
    let actual = validate_dns_packet(payload, expected_type)?;
    if actual != transaction_id {
        return Err(ExitDnsCodecError::TransactionMismatch {
            expected: transaction_id,
            actual,
        });
    }

    let payload_len =
        u16::try_from(payload.len()).map_err(|_| ExitDnsCodecError::DnsPacketLength {
            len: payload.len(),
            min: DNS_HEADER_BYTES,
            max: EXIT_DNS_MAX_PACKET_BYTES,
        })?;
    let mut bytes = Vec::with_capacity(EXIT_DNS_HEADER_BYTES + payload.len());
    bytes.extend_from_slice(EXIT_DNS_MAGIC);
    bytes.push(EXIT_DNS_VERSION);
    bytes.push(kind);
    bytes.extend_from_slice(&nonce.to_be_bytes());
    bytes.extend_from_slice(&transaction_id.to_be_bytes());
    bytes.extend_from_slice(&payload_len.to_be_bytes());
    bytes.extend_from_slice(payload);
    Ok(bytes)
}

pub fn decode_exit_dns_message(bytes: &[u8]) -> Result<ExitDnsMessage, ExitDnsCodecError> {
    if bytes.len() < EXIT_DNS_HEADER_BYTES {
        return Err(ExitDnsCodecError::Truncated);
    }
    if &bytes[..4] != EXIT_DNS_MAGIC {
        return Err(ExitDnsCodecError::InvalidMagic);
    }
    if bytes[4] != EXIT_DNS_VERSION {
        return Err(ExitDnsCodecError::UnsupportedVersion(bytes[4]));
    }

    let kind = bytes[5];
    let nonce = u64::from_be_bytes(bytes[6..14].try_into().expect("fixed nonce range"));
    let transaction_id =
        u16::from_be_bytes(bytes[14..16].try_into().expect("fixed transaction range"));
    let payload_len =
        u16::from_be_bytes(bytes[16..18].try_into().expect("fixed length range")) as usize;
    if bytes.len() != EXIT_DNS_HEADER_BYTES + payload_len {
        return Err(ExitDnsCodecError::InvalidLength);
    }
    let payload = &bytes[EXIT_DNS_HEADER_BYTES..];

    match kind {
        EXIT_DNS_REQUEST => {
            let request = ExitDnsRequest::new(nonce, payload.to_vec())?;
            if request.transaction_id != transaction_id {
                return Err(ExitDnsCodecError::TransactionMismatch {
                    expected: transaction_id,
                    actual: request.transaction_id,
                });
            }
            Ok(ExitDnsMessage::Request(request))
        }
        EXIT_DNS_RESPONSE => Ok(ExitDnsMessage::Response(ExitDnsResponse::new(
            nonce,
            transaction_id,
            payload.to_vec(),
        )?)),
        other => Err(ExitDnsCodecError::UnsupportedType(other)),
    }
}

pub fn build_exit_dns_refused_response(query: &[u8]) -> Option<Vec<u8>> {
    build_dns_error_response(query, ResponseCode::Refused)
}

pub fn build_exit_dns_servfail_response(query: &[u8]) -> Option<Vec<u8>> {
    build_dns_error_response(query, ResponseCode::ServFail)
}

fn validate_dns_packet(
    packet: &[u8],
    expected_type: MessageType,
) -> Result<u16, ExitDnsCodecError> {
    if !(DNS_HEADER_BYTES..=EXIT_DNS_MAX_PACKET_BYTES).contains(&packet.len()) {
        return Err(ExitDnsCodecError::DnsPacketLength {
            len: packet.len(),
            min: DNS_HEADER_BYTES,
            max: EXIT_DNS_MAX_PACKET_BYTES,
        });
    }
    let message = Message::from_vec(packet).map_err(|_| ExitDnsCodecError::InvalidDnsPacket)?;
    if message.metadata.message_type != expected_type {
        return Err(ExitDnsCodecError::InvalidDnsMessageType);
    }
    Ok(message.id)
}

fn build_dns_error_response(query: &[u8], code: ResponseCode) -> Option<Vec<u8>> {
    let request = Message::from_vec(query).ok()?;
    if request.metadata.message_type != MessageType::Query {
        return None;
    }
    let mut response = Message::new(request.id, MessageType::Response, OpCode::Query);
    response.metadata.recursion_desired = request.recursion_desired;
    response.metadata.recursion_available = false;
    response.metadata.authoritative = false;
    response.metadata.response_code = code;
    for query in request.queries {
        response.add_query(query);
    }

    let mut bytes = Vec::with_capacity(query.len().max(DNS_HEADER_BYTES));
    response.emit(&mut BinEncoder::new(&mut bytes)).ok()?;
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hickory_proto::op::{Query, ResponseCode};
    use hickory_proto::rr::{Name, RecordType};

    fn query(id: u16) -> Vec<u8> {
        let mut message = Message::new(id, MessageType::Query, OpCode::Query);
        message.add_query(Query::query(
            Name::from_ascii("example.com.").expect("DNS name"),
            RecordType::A,
        ));
        let mut bytes = Vec::new();
        message
            .emit(&mut BinEncoder::new(&mut bytes))
            .expect("DNS query");
        bytes
    }

    #[test]
    fn versioned_request_and_response_round_trip_with_correlation() {
        let request = ExitDnsMessage::Request(ExitDnsRequest::new(99, query(42)).expect("request"));
        assert_eq!(
            decode_exit_dns_message(&encode_exit_dns_message(&request).expect("encode request"))
                .expect("decode request"),
            request
        );

        let response_packet = build_exit_dns_refused_response(&query(42)).expect("response");
        let response = ExitDnsMessage::Response(
            ExitDnsResponse::new(99, 42, response_packet).expect("correlated response"),
        );
        assert_eq!(
            decode_exit_dns_message(&encode_exit_dns_message(&response).expect("encode response"))
                .expect("decode response"),
            response
        );
    }

    #[test]
    fn codec_rejects_wrong_version_length_and_transaction() {
        let request = ExitDnsMessage::Request(ExitDnsRequest::new(7, query(42)).expect("request"));
        let mut bytes = encode_exit_dns_message(&request).expect("frame");
        bytes[4] = EXIT_DNS_VERSION + 1;
        assert!(matches!(
            decode_exit_dns_message(&bytes),
            Err(ExitDnsCodecError::UnsupportedVersion(_))
        ));

        let mut bytes = encode_exit_dns_message(&request).expect("frame");
        bytes[17] = bytes[17].saturating_add(1);
        assert_eq!(
            decode_exit_dns_message(&bytes),
            Err(ExitDnsCodecError::InvalidLength)
        );

        let mut bytes = encode_exit_dns_message(&request).expect("frame");
        bytes[15] ^= 1;
        assert!(matches!(
            decode_exit_dns_message(&bytes),
            Err(ExitDnsCodecError::TransactionMismatch { .. })
        ));

        let response = build_exit_dns_servfail_response(&query(42)).expect("SERVFAIL");
        assert!(matches!(
            ExitDnsResponse::new(7, 43, response),
            Err(ExitDnsCodecError::TransactionMismatch { .. })
        ));
    }

    #[test]
    fn codec_rejects_oversized_dns_payload() {
        let mut oversized = query(42);
        oversized.resize(EXIT_DNS_MAX_PACKET_BYTES + 1, 0);
        assert!(matches!(
            ExitDnsRequest::new(1, oversized),
            Err(ExitDnsCodecError::DnsPacketLength { .. })
        ));
    }

    #[test]
    fn explicit_dns_errors_preserve_transaction_and_question() {
        for (packet, expected) in [
            (
                build_exit_dns_refused_response(&query(31337)).expect("REFUSED"),
                ResponseCode::Refused,
            ),
            (
                build_exit_dns_servfail_response(&query(31337)).expect("SERVFAIL"),
                ResponseCode::ServFail,
            ),
        ] {
            let response = Message::from_vec(&packet).expect("DNS response");
            assert_eq!(response.id, 31337);
            assert_eq!(response.metadata.response_code, expected);
            assert_eq!(response.queries.len(), 1);
        }
    }
}
