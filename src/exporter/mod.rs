//! # Exporter
//! Prometheus exporter service for Kasa to be used in Hyper Server.

use std::sync::Arc;

use error_chain::ChainedError;

use futures::future;
use futures::future::Future;
use futures::stream;
use futures::stream::Stream;

use hyper;

use prometheus;
use prometheus::Encoder;

use super::kasa;

/// Implements an exporter for a given client.
///
/// # Examples
///
/// ```
/// use std::net;
/// use std::sync::Arc;
/// use futures::future;
/// use futures::future::Future;
/// use futures::stream;
/// use futures::stream::Stream;
/// use tokio;
/// use hyper;
///
/// fn main() {
///     let http_client = hyper::Client::builder()
///         .build::<_, hyper::Body>(hyper_tls::HttpsConnector::new(1).unwrap());
///
///     tokio::run(
///         kasa_exporter::kasa::Kasa::new(
///             http_client,
///             "ebpf_exporter".to_string(),
///             "foo@bar.com".to_string(),
///             "123".to_string(),
///         )
///         .map_err(|e| eprintln!("kasa authentication error: {}", e))
///         .and_then(move |client| {
///             let client = Arc::new(client);
///
///             hyper::Server::bind(&"[::1]:12345".parse().unwrap())
///                 .serve(move || hyper::service::service_fn(kasa_exporter::exporter::service(Arc::clone(&client))))
///                 .map_err(|e| eprintln!("server error: {}", e))
///         }),
///     );
/// }
/// ```
pub fn service<T>(
    client: Arc<kasa::Kasa<T>>,
) -> impl Fn(
    hyper::Request<hyper::Body>,
) -> Box<Future<Item = hyper::Response<hyper::Body>, Error = hyper::Error> + Send>
where
    T: hyper::client::connect::Connect + Sync + 'static,
{
    move |_| Box::new(serve(Arc::clone(&client)))
}

/// Returns a future that implements a response for an exporter request.
fn serve<T>(
    client: Arc<kasa::Kasa<T>>,
) -> impl Future<Item = hyper::Response<hyper::Body>, Error = hyper::Error>
where
    T: hyper::client::connect::Connect + Sync + 'static,
{
    client
        .get_device_list()
        .and_then(|devices| match devices.result {
            Some(devices) => future::Either::A(
                stream::iter_ok(devices.device_list.into_iter())
                    .and_then(move |device| {
                        client
                            .emeter(&device.device_id)
                            .map(|emeter| (device, emeter))
                    })
                    .collect(),
            ),
            None => future::Either::B(future::ok(vec![])),
        })
        .and_then(
            |emeters: Vec<(kasa::DeviceListEntry, kasa::EmeterResult)>| {
                let encoder = prometheus::TextEncoder::new();

                let mut buffer = vec![];
                encoder
                    .encode(&registry(emeters).gather(), &mut buffer)
                    .unwrap();

                let mut http_response = hyper::Response::new(hyper::Body::from(buffer));

                let content_type = match encoder.format_type().parse() {
                    Ok(content_type) => content_type,
                    Err(e) => {
                        return {
                            eprintln!("error formatting content type: {}", e);

                            let mut http_response = hyper::Response::new(hyper::Body::empty());
                            *http_response.status_mut() = http::StatusCode::INTERNAL_SERVER_ERROR;

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
            Ok(hyper::Response::new(hyper::Body::empty()))
        })
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
fn registry(emeters: Vec<(kasa::DeviceListEntry, kasa::EmeterResult)>) -> prometheus::Registry {
    let voltage = gauge_vec(
        "device_electric_potential_volts",
        "Voltage reading from device",
        &["device_alias", &"device_id"],
    );
    let current = gauge_vec(
        "device_electric_current_amperes",
        "Corrent reading from device",
        &["device_alias", &"device_id"],
    );
    let power = gauge_vec(
        "device_electric_power_watts",
        "Power reading from device",
        &["device_alias", &"device_id"],
    );

    let registry = prometheus::Registry::new();

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
    prometheus::GaugeVec::new(prometheus::opts!(name, help), labels).unwrap()
}
