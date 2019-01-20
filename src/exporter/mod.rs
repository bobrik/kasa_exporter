//! # Exporter
//! Prometheus exporter service for Kasa to be used in Hyper Server.

use error_chain::ChainedError;

use std::sync::{Arc, Mutex};

use futures::future;
use futures::future::Future;
use futures::stream;
use futures::stream::Stream;

use hyper::{Body, Request, Response};

use prometheus::{Encoder, GaugeVec, Registry, TextEncoder};

use super::kasa;

/// Implements an exporter for a given client.
///
/// # Examples
///
/// ```
/// use std::net;
/// use std::sync::{Arc, Mutex};
/// use futures::future;
/// use futures::future::Future;
/// use futures::stream;
/// use futures::stream::Stream;
/// use tokio;
/// use hyper;
///
/// fn main() {
///     tokio::run(
///         kasa_exporter::kasa::Kasa::new(
///             "ebpf_exporter".to_string(),
///             "foo@bar.com".to_string(),
///             "123".to_string(),
///         )
///         .map_err(|e| eprintln!("kasa authentication error: {}", e))
///         .and_then(move |c| {
///             let client = Arc::new(Mutex::new(c));
///
///             hyper::Server::bind(&"[::1]:12345".parse().unwrap())
///                 .serve(move || hyper::service::service_fn(kasa_exporter::exporter::service(client.clone())))
///                 .map_err(|e| eprintln!("server error: {}", e))
///         }),
///     );
/// }
/// ```
pub fn service(
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

/// Populates data for a metric from a given emeter measurement.
macro_rules! fill_metric {
    ( labels = $labels:expr, $($metric:expr => $value:expr,)+ ) => {
        $(
            if let Some(value) = $value {
                $metric.with($labels).set(value);
            }
        )+
    }
}

/// Creates a throw away registry to populate data for a request.
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

/// Creates Gauge vector with given parameters.
fn gauge_vec(name: &str, help: &str, labels: &[&str]) -> prometheus::GaugeVec {
    GaugeVec::new(prometheus::opts!(name, help), labels).unwrap()
}
