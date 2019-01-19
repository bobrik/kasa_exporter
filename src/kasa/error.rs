// TODO: fix after error_chain supports Rust 2018:
// * https://github.com/rust-lang-nursery/error-chain/issues/250
use error_chain::error_chain;
use error_chain::error_chain_processing;
use error_chain::impl_error_chain_kind;
use error_chain::impl_error_chain_processed;
use error_chain::impl_extract_backtrace;

error_chain! {
    errors {
        EmptyAuthResponse(code: i32, message: String) {
            description("empty auth response")
            display("empty auth response: code={} message={}", code, message)
        }
        EmptyPassthroughResponse {
            description("empty passthrough response")
            display("empty passthrough response")
        }
        EmptyEmeterResponse {
            description("empty emeter response")
            display("empty emeter response")
        }
        PassthtoughParams {
            description("error initializing passthrough params")
            display("error initializing passthrough params")
        }
        Deserialization(data: String) {
            description("deserialization error")
            display("error deserializing: {}", data)
        }
        Serialization(debug: String) {
            description("serialization error")
            display("error serializing: {}", debug)
        }
    }

    foreign_links {
        Hyper(hyper::error::Error);
        HyperTLS(hyper_tls::Error);
        Serde(serde_json::error::Error);
        InvalidUri(http::uri::InvalidUri);
    }
}
