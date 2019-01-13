use std::net;

use std::sync::Arc;
use std::sync::Mutex;

use futures::future::Future;
use futures::stream::iter_ok;
use futures::stream::Stream;

use tokio;

use hyper::service::service_fn;
use hyper::Body;
use hyper::Request;
use hyper::Response;
use hyper::Server;

use serde;
use serde_derive;
use serde_json;

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
        kasa::Kasa::new(clap::crate_name!().to_string(), username, password).and_then(move |c| {
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
                .and_then(|devices| {
                    iter_ok::<_, ()>(devices.result.unwrap().device_list.into_iter())
                        .and_then(move |device| {
                            inner_client
                                .lock()
                                .unwrap()
                                .emeter(&device.device_id)
                                .map(|emeter| (device, emeter))
                        })
                        .collect()
                })
                .and_then(
                    |emeters: Vec<(kasa::DeviceListEntry, kasa::EmeterResult)>| {
                        let encoder = TextEncoder::new();

                        let mut buffer = vec![];
                        encoder
                            .encode(&registry(emeters).gather(), &mut buffer)
                            .unwrap();

                        let mut res = Response::new(Body::from(buffer));

                        res.headers_mut().insert(
                            hyper::header::CONTENT_TYPE,
                            encoder.format_type().parse().unwrap(),
                        );

                        Ok(res)
                    },
                )
                .or_else(|_| Ok(Response::new(Body::empty())))
        })
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

    for metric in vec![&voltage, &current, &power] {
        registry.register(Box::new(metric.clone())).unwrap();
    }

    for (device, emeter) in emeters {
        let realtime = emeter.get_realtime.unwrap();

        let labels = &prometheus::labels! {
                "device_alias" => device.alias.as_str(),
                "device_id"    => device.device_id.as_str(),
        };

        voltage.with(labels).set(realtime.voltage.unwrap());
        current.with(labels).set(realtime.current.unwrap());
        power.with(labels).set(realtime.power.unwrap());
    }

    registry
}

fn gauge_vec(name: &str, help: &str, labels: &[&str]) -> prometheus::GaugeVec {
    GaugeVec::new(prometheus::opts!(name, help), labels).unwrap()
}
