//! Per-profile loopback TCP broker protocol, registry, server, and client.

pub mod client;
pub mod protocol;
pub mod registry;
pub mod server;

pub use client::{BROKER_RESPONSE_LIMIT_BYTES, BrokerClient, BrokerLauncher};
pub use protocol::{
    BROKER_DESCRIPTOR_SCHEMA, BROKER_REGISTRATION_SCHEMA, BrokerDescriptor, BrokerEndpoint,
    BrokerRegistration, BrokerRequest, CommandSemantics, DEFAULT_COMMAND_TIMEOUT_SECONDS,
    DETERMINISTIC_WRITE_ERRORS, MAX_COMMAND_TIMEOUT_SECONDS, MAX_FRAME_BYTES,
    MIN_COMMAND_TIMEOUT_SECONDS, command_timeout_seconds, constant_time_eq, read_bounded_line,
};
pub use registry::{
    broker_status, descriptor_is_live, list_brokers, register_broker, registration_path,
    stop_all_brokers, stop_broker, unregister_broker,
};
pub use server::{BrokerHandler, BrokerServer, BrokerServerConfig, FnBrokerHandler};
