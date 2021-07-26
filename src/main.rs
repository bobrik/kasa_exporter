use std::net;

use std::sync::Arc;

mod exporter;
mod kasa;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
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

    let addr = matches
        .value_of("web.listen-address")
        .unwrap()
        .parse()
        .unwrap();

    let http_client =
        hyper::Client::builder().build::<_, hyper::Body>(hyper_tls::HttpsConnector::new());

    let client = kasa::Client::new(
        http_client,
        clap::crate_name!().to_string(),
        username,
        password,
    )
    .await
    .unwrap();

    let client = Arc::new(client);

    let service = hyper::service::make_service_fn(move |_| {
        let client = Arc::clone(&client);

        async move {
            Ok::<_, hyper::Error>(hyper::service::service_fn(move |_| {
                exporter::serve(Arc::clone(&client))
            }))
        }
    });

    hyper::Server::bind(&addr).serve(service).await?;

    Ok(())
}
