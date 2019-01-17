// TODO: fix after error_chain supports Rust 2018:
// * https://github.com/rust-lang-nursery/error-chain/issues/250
use error_chain::error_chain;
use error_chain::error_chain_processing;
use error_chain::impl_error_chain_kind;
use error_chain::impl_error_chain_processed;
use error_chain::impl_extract_backtrace;

error_chain! {
    foreign_links {
        Hyper(hyper::error::Error);
    }
}
