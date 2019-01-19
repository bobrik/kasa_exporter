use std::net;

use std::sync::Arc;
use std::sync::Mutex;

use error_chain::ChainedError;

use futures::future;
use futures::future::Future;
use futures::stream;
use futures::stream::Stream;

use tokio;

use hyper::service::service_fn;
use hyper::Body;
use hyper::Request;
use hyper::Response;
use hyper::Server;

use http;

use prometheus;
use prometheus::Encoder;
use prometheus::GaugeVec;
use prometheus::Registry;
use prometheus::TextEncoder;

use clap;

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
                .default_value(":12345"),
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

    tokio::run(
        kasa::Kasa::new(clap::crate_name!().to_string(), username, password)
            .map_err(|e| eprintln!("kasa authentication error: {}", e))
            .and_then(move |c| {
                let client = Arc::new(Mutex::new(c));

                Server::bind(
                    &matches
                        .value_of("web.listen-address")
                        .unwrap()
                        .parse()
                        .unwrap(),
                )
                .serve(move || service_fn(service(client.clone())))
                .map_err(|e| eprintln!("server error: {}", e))
            }),
    );
}

fn service(
    client: Arc<Mutex<kasa::Kasa>>,
) -> impl Fn(Request<Body>) -> Box<Future<Item = Response<Body>, Error = hyper::Error> + Send> {
    move |_| {
        Box::new({
            // This is ugly
            let inner_client = Arc::clone(&client);

            client
                .lock()
                .unwrap()
                .get_device_list()
                .and_then(|devices| match devices.result {
                    Some(devices) => future::Either::A(
                        stream::iter_ok(devices.device_list.into_iter())
                            .and_then(move |device| {
                                inner_client
                                    .lock()
                                    .unwrap()
                                    .emeter(&device.device_id)
                                    .map(|emeter| (device, emeter))
                            })
                            .collect(),
                    ),
                    None => future::Either::B(future::ok(vec![])),
                })
                .and_then(
                    |emeters: Vec<(kasa::DeviceListEntry, kasa::EmeterResult)>| {
                        let encoder = TextEncoder::new();

                        let mut buffer = vec![];
                        encoder
                            .encode(&registry(emeters).gather(), &mut buffer)
                            .unwrap();

                        let mut http_response = Response::new(Body::from(buffer));

                        let content_type = match encoder.format_type().parse() {
                            Ok(content_type) => content_type,
                            Err(e) => {
                                return {
                                    eprintln!("error formatting content type: {}", e);

                                    let mut http_response = Response::new(Body::empty());
                                    *http_response.status_mut() =
                                        http::StatusCode::INTERNAL_SERVER_ERROR;

                                    Ok(http_response)
                                };
                            }
                        };

                        http_response
                            .headers_mut()
                            .insert(hyper::header::CONTENT_TYPE, content_type);

                        Ok(http_response)
                    },
                )
                .or_else(|e| {
                    eprintln!("error from kasa api: {}", e.display_chain().to_string());
                    Ok(Response::new(Body::empty()))
                })
        })
    }
}

macro_rules! fill_metric {
    ( labels = $labels:expr, $($metric:expr => $value:expr,)+ ) => {
        $(
            if let Some(value) = $value {
                $metric.with($labels).set(value);
            }
        )+
    }
}
fn registry(emeters: Vec<(kasa::DeviceListEntry, kasa::EmeterResult)>) -> Registry {
    let voltage = gauge_vec(
        "device_voltage",
        "Voltage reading from device",
        &["device_alias", &"device_id"],
    );
    let current = gauge_vec(
        "device_current",
        "Corrent reading from device",
        &["device_alias", &"device_id"],
    );
    let power = gauge_vec(
        "device_power",
        "Power reading from device",
        &["device_alias", &"device_id"],
    );

    let registry = Registry::new();

    let collectors = vec![&voltage, &current, &power];

    for metric in collectors {
        registry.register(Box::new(metric.clone())).unwrap();
    }

    for (device, emeter) in emeters {
        let realtime = match emeter.get_realtime {
            Some(realtime) => realtime,
            None => continue,
        };

        let labels = &prometheus::labels! {
                "device_alias" => device.alias.as_str(),
                "device_id"    => device.device_id.as_str(),
        };

        fill_metric! { labels = labels,
            voltage => realtime.voltage,
            current => realtime.current,
            power   => realtime.power,
        };
    }

    registry
}

fn gauge_vec(name: &str, help: &str, labels: &[&str]) -> prometheus::GaugeVec {
    GaugeVec::new(prometheus::opts!(name, help), labels).unwrap()
}
