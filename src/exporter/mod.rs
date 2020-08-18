//! # Exporter
//! Prometheus exporter service for Kasa to be used in Hyper Server.

use std::result::Result;

use std::sync::Arc;

use prometheus::Encoder;

use super::kasa;

/// Returns a future that implements a response for an exporter request.
pub async fn serve<T>(
    client: Arc<kasa::Client<T>>,
) -> Result<hyper::Response<hyper::Body>, hyper::Error>
where
    T: hyper::client::connect::Connect + std::clone::Clone + std::marker::Send + Sync + 'static,
{
    let devices = match client.get_device_list().await {
        Ok(devices) => devices,
        Err(e) => {
            return {
                eprintln!("error from kasa api: {}", e.to_string());
                let mut http_response = hyper::Response::new(hyper::Body::empty());
                *http_response.status_mut() = hyper::StatusCode::INTERNAL_SERVER_ERROR;
                Ok(http_response)
            }
        }
    };

    let emeters: Vec<(kasa::DeviceListEntry, kasa::EmeterResult)> = match devices.result {
        Some(devices) => {
            let mut results = Vec::new();
            for device in devices.device_list {
                match client.emeter(&device.device_id).await {
                    Ok(emeter) => results.push((device, emeter)),
                    Err(e) => eprintln!(
                        "error reading device {} ({}): {}",
                        device.alias,
                        device.device_id,
                        e.to_string()
                    ),
                };
            }
            results
        }
        None => vec![],
    };

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
                *http_response.status_mut() = hyper::StatusCode::INTERNAL_SERVER_ERROR;

                Ok(http_response)
            };
        }
    };

    http_response
        .headers_mut()
        .insert(hyper::header::CONTENT_TYPE, content_type);

    Ok(http_response)
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
    let energy = gauge_vec(
        "device_electric_energy_joules_total",
        "Total energy consumed",
        &["device_alias", &"device_id"],
    );

    let registry = prometheus::Registry::new();

    let collectors = vec![&voltage, &current, &power, &energy];

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
            energy  => realtime.total.map(|kwh| kwh * 3600.0 * 1000.0),
        };
    }

    registry
}

/// Creates Gauge vector with given parameters.
fn gauge_vec(name: &str, help: &str, labels: &[&str]) -> prometheus::GaugeVec {
    prometheus::GaugeVec::new(prometheus::opts!(name, help), labels).unwrap()
}
