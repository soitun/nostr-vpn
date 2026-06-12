use super::*;

const BINARY_VERSION_QUERY_TIMEOUT: Duration = Duration::from_secs(2);
const BINARY_VERSION_QUERY_POLL_INTERVAL: Duration = Duration::from_millis(25);

include!("service_management/commands.rs");
include!("service_management/linux.rs");
include!("service_management/windows.rs");
include!("service_management/helpers.rs");
