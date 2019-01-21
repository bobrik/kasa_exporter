use std::net;

use std::sync::Arc;

use futures::Future;

use clap;
use hyper;
use hyper_tls;
use tokio;

mod exporter;
mod kasa;

fn main() {
    let matches = clap::App::new(clap::crate_name!())
        .version(clap::crate_version!())
        .author(clap::crate_authors!())
        .about(clap::crate_description!())
        .arg(
            clap::Arg::with_name("web.listen-address")
                .help("Address on which to expose metrics and web interface")
                .long("web.listen-address")
                .validator(|v| {
                    v.parse::<net::SocketAddr>()
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                })
                .takes_value(true)
                .default_value("[::1]:12345"),
        )
        .arg(
            clap::Arg::with_name("kasa.username")
                .help("Username to log into Kasa service")
                .long("kasa.username")
                .takes_value(true)
                .required(true),
        )
        .arg(
            clap::Arg::with_name("kasa.password")
                .help("Password to log into Kasa service")
                .long("kasa.password")
                .takes_value(true)
                .required(true),
        )
        .get_matches();

    let username = matches.value_of("kasa.username").unwrap().to_string();
    let password = matches.value_of("kasa.password").unwrap().to_string();

    let http_client = hyper::Client::builder()
        .build::<_, hyper::Body>(hyper_tls::HttpsConnector::new(1).unwrap());

    tokio::run(
        kasa::Kasa::new(
            http_client,
            clap::crate_name!().to_string(),
            username,
            password,
        )
        .map_err(|e| eprintln!("kasa authentication error: {}", e))
        .and_then(move |client| {
            let client = Arc::new(client);

            hyper::Server::bind(
                &matches
                    .value_of("web.listen-address")
                    .unwrap()
                    .parse()
                    .unwrap(),
            )
            .serve(move || hyper::service::service_fn(exporter::service(Arc::clone(&client))))
            .map_err(|e| eprintln!("server error: {}", e))
        }),
    );
}
