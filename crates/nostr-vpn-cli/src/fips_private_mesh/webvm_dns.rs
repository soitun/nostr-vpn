const WEBVM_IRIS_LOCALHOST_SUFFIX: &str = ".iris.localhost";
const WEBVM_LOCAL_DNS_TTL_SECS: u32 = 60;

fn webvm_dns_query_is_fips(query: &[u8]) -> bool {
    webvm_dns_query_names(query).is_some_and(|names| {
        names.iter().any(|name| {
            name.trim_end_matches('.')
                .to_ascii_lowercase()
                .ends_with(".fips")
        })
    })
}

fn webvm_iris_localhost_dns_response(query: &[u8]) -> Option<Vec<u8>> {
    use hickory_proto::op::{Message, MessageType, OpCode, ResponseCode};
    use hickory_proto::rr::rdata::A;
    use hickory_proto::rr::{RData, Record, RecordType};
    use hickory_proto::serialize::binary::{BinEncodable as _, BinEncoder};

    let request = Message::from_vec(query).ok()?;
    let mut response = Message::new(request.id, MessageType::Response, OpCode::Query);
    response.metadata.recursion_desired = request.recursion_desired;
    response.metadata.recursion_available = false;
    response.metadata.authoritative = true;
    response.metadata.response_code = ResponseCode::NoError;

    let mut matched = false;
    for query in &request.queries {
        response.add_query(query.clone());
        if !webvm_dns_name_is_iris_localhost(&query.name().to_utf8()) {
            continue;
        }
        matched = true;
        let data = match query.query_type() {
            RecordType::A => Some(RData::A(A(std::net::Ipv4Addr::LOCALHOST))),
            _ => None,
        };
        if let Some(data) = data {
            response.add_answer(Record::from_rdata(
                query.name().clone(),
                WEBVM_LOCAL_DNS_TTL_SECS,
                data,
            ));
        }
    }
    if !matched {
        return None;
    }

    let mut bytes = Vec::with_capacity(512);
    let mut encoder = BinEncoder::new(&mut bytes);
    response.emit(&mut encoder).ok()?;
    Some(bytes)
}

fn webvm_dns_query_names(query: &[u8]) -> Option<Vec<String>> {
    let message = hickory_proto::op::Message::from_vec(query).ok()?;
    Some(
        message
            .queries
            .iter()
            .map(|query| query.name().to_utf8())
            .collect(),
    )
}

fn webvm_dns_name_is_iris_localhost(name: &str) -> bool {
    let name = name.trim_end_matches('.').to_ascii_lowercase();
    name.len() > WEBVM_IRIS_LOCALHOST_SUFFIX.len() && name.ends_with(WEBVM_IRIS_LOCALHOST_SUFFIX)
}

#[cfg(test)]
mod webvm_dns_tests {
    use super::*;
    use hickory_proto::op::{Message, MessageType, OpCode, Query, ResponseCode};
    use hickory_proto::rr::rdata::A;
    use hickory_proto::rr::{Name, RData, RecordType};
    use hickory_proto::serialize::binary::{BinEncodable as _, BinEncoder};

    fn query(name: &str, record_type: RecordType) -> Vec<u8> {
        let mut message = Message::new(42, MessageType::Query, OpCode::Query);
        message.add_query(Query::query(
            Name::from_ascii(name).expect("DNS name"),
            record_type,
        ));
        let mut bytes = Vec::new();
        message
            .emit(&mut BinEncoder::new(&mut bytes))
            .expect("DNS query");
        bytes
    }

    fn iris_response(name: &str, record_type: RecordType) -> Message {
        Message::from_vec(
            &webvm_iris_localhost_dns_response(&query(name, record_type))
                .expect("local Iris DNS response"),
        )
        .expect("decode DNS response")
    }

    #[test]
    fn webvm_dns_only_classifies_fips_suffix() {
        assert!(webvm_dns_query_is_fips(&query(
            "npub1example.fips.",
            RecordType::AAAA,
        )));
        assert!(!webvm_dns_query_is_fips(&query(
            "example.com.",
            RecordType::AAAA,
        )));
    }

    #[test]
    fn iris_localhost_a_resolves_to_guest_loopback() {
        let response = iris_response("nhash123.iris.localhost.", RecordType::A);
        assert_eq!(response.metadata.response_code, ResponseCode::NoError);
        assert!(response.metadata.authoritative);
        assert_eq!(response.answers.len(), 1);
        match &response.answers[0].data {
            RData::A(A(ip)) => assert_eq!(*ip, std::net::Ipv4Addr::LOCALHOST),
            data => panic!("unexpected Iris A response: {data:?}"),
        }
    }

    #[test]
    fn nested_iris_localhost_aaaa_returns_no_ipv6_loopback_answer() {
        let response = iris_response("site.npub1example.iris.localhost.", RecordType::AAAA);
        assert_eq!(response.metadata.response_code, ResponseCode::NoError);
        assert!(response.metadata.authoritative);
        assert!(response.answers.is_empty());
    }

    #[test]
    fn public_dns_is_not_claimed_as_iris_localhost() {
        assert!(webvm_iris_localhost_dns_response(&query("example.com.", RecordType::A)).is_none());
        assert!(
            webvm_iris_localhost_dns_response(&query("iris.localhost.", RecordType::A)).is_none()
        );
        assert!(
            webvm_iris_localhost_dns_response(&query("npub1example.fips.", RecordType::AAAA))
                .is_none()
        );
    }
}
