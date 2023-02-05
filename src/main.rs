use std::{net::UdpSocket, sync::atomic::AtomicU64, time::Duration};

use axum::{
    http::{HeaderMap, HeaderValue},
    response::IntoResponse,
    routing::get,
    Router, Server,
};
use clap::Parser;
use prometheus_client::{
    encoding::{text::encode, EncodeLabelSet},
    metrics::{counter::Counter, family::Family, gauge::Gauge},
    registry::Registry,
};
use serde_derive::Deserialize;
use serde_json::from_slice;
use tplink_shome_protocol::{decrypt, encrypt};

const BROADCAST_BIND_ADDR: &str = "0.0.0.0:0";
const BROADCAST_SEND_ADDR: &str = "255.255.255.255:9999";

const BROADCAST_MESSAGE: &[u8] =
    r#"{"system":{"get_sysinfo":{}},"emeter":{"get_realtime":{}}}"#.as_bytes();

const BROADCAST_WAIT_TIME: Duration = Duration::from_millis(500);

const BROADCAST_RESPONSE_BUFFER_SIZE: usize = 4096;

const DEFAULT_PROMETHEUS_BIND_ADDR: &str = "[::1]:12345";

const PROMETHEUS_CONTENT_TYPE: &str = "application/openmetrics-text; version=1.0.0; charset=utf-8";

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Address on which to expose metrics and web interface.
    #[arg(long = "web.listen-address", default_value = DEFAULT_PROMETHEUS_BIND_ADDR)]
    listen_address: String,
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args = Args::parse();

    let addr = args
        .listen_address
        .parse()
        .expect("error parsing listen address");

    eprintln!("listening on {}", args.listen_address);

    let app = Router::new().route("/metrics", get(metrics));

    Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .expect("error running server");
}

async fn metrics() -> impl IntoResponse {
    let socket = UdpSocket::bind(BROADCAST_BIND_ADDR).unwrap();
    socket.set_read_timeout(Some(BROADCAST_WAIT_TIME)).unwrap();
    socket.set_broadcast(true).unwrap();

    let msg = encrypt(BROADCAST_MESSAGE);

    socket
        .send_to(&msg, BROADCAST_SEND_ADDR)
        .expect("error broadcasting");

    let mut buf = [0u8; BROADCAST_RESPONSE_BUFFER_SIZE];
    let mut responses = vec![];

    while let Ok((n, _)) = socket.recv_from(&mut buf) {
        responses.push(from_slice(&decrypt(&buf[0..n])).expect("ugh"));
    }

    let registry = into_registry(responses);

    let mut buffer = String::new();
    encode(&mut buffer, &registry).expect("error encoding prometheus data");

    let mut headers = HeaderMap::new();
    headers.insert(
        "content-type",
        HeaderValue::from_static(PROMETHEUS_CONTENT_TYPE),
    );

    (headers, buffer)
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
struct MetricLabels {
    device_alias: String,
    device_id: String,
}

type GaugeMetric = Family<MetricLabels, Gauge<f64, AtomicU64>>;
type CounterMetric = Family<MetricLabels, Counter<f64, AtomicU64>>;

fn into_registry(responses: Vec<BroadcastResponse>) -> Registry {
    let mut registry = Registry::default();

    let voltage = GaugeMetric::default();

    registry.register(
        "device_electric_potential_volts",
        "Voltage reading from device",
        voltage.clone(),
    );

    let current = GaugeMetric::default();

    registry.register(
        "device_electric_current_amperes",
        "Current reading from device",
        current.clone(),
    );

    let power = GaugeMetric::default();

    registry.register(
        "device_electric_power_watts",
        "Power reading from device",
        power.clone(),
    );

    let energy = CounterMetric::default();

    registry.register(
        "device_electric_energy_joules_total",
        "Voltage reading from device",
        energy.clone(),
    );

    for response in responses {
        let realtime = match response.emeter.get_realtime {
            Some(realtime) => realtime,
            None => continue,
        };

        let labels = MetricLabels {
            device_alias: response.system.get_sysinfo.alias.clone(),
            device_id: response.system.get_sysinfo.device_id.clone(),
        };

        voltage
            .get_or_create(&labels)
            .set(if realtime.voltage.unwrap_or_default() > 0.0 {
                realtime.voltage.unwrap()
            } else {
                realtime.voltage_mv.map(|mv| mv as f64 / 1000.0).unwrap()
            });

        current
            .get_or_create(&labels)
            .set(if realtime.current.unwrap_or_default() > 0.0 {
                realtime.current.unwrap()
            } else {
                realtime.current_ma.map(|ma| ma as f64 / 1000.0).unwrap()
            });

        power
            .get_or_create(&labels)
            .set(if realtime.power.unwrap_or_default() > 0.0 {
                realtime.power.unwrap()
            } else {
                realtime.power_mw.map(|w| w as f64 / 1000.0).unwrap()
            });

        energy
            .get_or_create(&labels)
            .inc_by(if realtime.total.unwrap_or_default() > 0.0 {
                realtime.total.map(|kwh| kwh * 3600.0 * 1000.0).unwrap()
            } else {
                realtime.total_wh.map(|wh| wh as f64 * 3600.0).unwrap()
            });
    }

    registry
}

#[derive(Deserialize, Debug)]
struct BroadcastResponse {
    system: SystemResponse,
    emeter: EmeterResponse,
}

#[derive(Deserialize, Debug)]
struct SystemResponse {
    get_sysinfo: GetSysinfoResponse,
}

#[derive(Deserialize, Debug)]
struct GetSysinfoResponse {
    alias: String,
    #[serde(rename = "deviceId")]
    device_id: String,
}

#[derive(Deserialize, Debug)]
struct EmeterResponse {
    get_realtime: Option<GetRealtimeResponse>,
}

#[derive(Deserialize, Debug)]
struct GetRealtimeResponse {
    // v1 hardware returns f64 values in base units
    current: Option<f64>,
    voltage: Option<f64>,
    power: Option<f64>,
    total: Option<f64>,

    // v2 hardware returns u64 values in named units
    voltage_mv: Option<u64>,
    current_ma: Option<u64>,
    power_mw: Option<u64>,
    total_wh: Option<u64>,
}
