use std::{
    collections::HashMap,
    net::{SocketAddr, UdpSocket},
    sync::{atomic::AtomicU64, Arc, Mutex},
    time::{Duration, Instant},
};

use axum::{
    extract::State,
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
use serde_json::{from_slice, from_str};
use tplink_shome_protocol::{decrypt, encrypt};

const BROADCAST_BIND_ADDR: &str = "0.0.0.0:0";
const BROADCAST_SEND_ADDR: &str = "255.255.255.255:9999";

const BROADCAST_MESSAGE: &[u8] =
    r#"{"system":{"get_sysinfo":{}},"emeter":{"get_realtime":{}}}"#.as_bytes();

const BROADCAST_WAIT_TIME: Duration = Duration::from_millis(500);

const BROADCAST_RESPONSE_BUFFER_SIZE: usize = 4096;

const DEFAULT_PROMETHEUS_BIND_ADDR: &str = "[::1]:12345";

const PROMETHEUS_CONTENT_TYPE: &str = "application/openmetrics-text; version=1.0.0; charset=utf-8";

const FORGET_TIMEOUT: Duration = Duration::from_secs(60 * 30);

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

    let app = Router::new()
        .route("/metrics", get(metrics))
        .with_state(AppState::default());

    Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .expect("error running server");
}

#[derive(Default, Clone)]
struct AppState {
    endpoints: Arc<Mutex<HashMap<SocketAddr, Instant>>>,
}

async fn metrics(State(state): State<AppState>) -> impl IntoResponse {
    let now = Instant::now();

    let mut broadcast_responses = broadcast();

    let mut endpoints = state.endpoints.lock().expect("error locking endpoints");

    let mut combined_responses = vec![];

    let mut remove = vec![];

    for (endpoint, last_seen) in endpoints.iter() {
        if let Some(response) = broadcast_responses.remove(endpoint) {
            combined_responses.push(response);
        } else if let Some(response) = check_one(endpoint) {
            combined_responses.push(response);
        } else if now.duration_since(*last_seen) > FORGET_TIMEOUT {
            remove.push(*endpoint);
        }
    }

    for endpoint in remove {
        endpoints.remove(&endpoint);
    }

    for (addr, response) in broadcast_responses {
        endpoints.insert(addr, Instant::now());
        combined_responses.push(response);
    }

    let registry = into_registry(combined_responses);

    let mut buffer = String::new();
    encode(&mut buffer, &registry).expect("error encoding prometheus data");

    let mut headers = HeaderMap::new();
    headers.insert(
        "content-type",
        HeaderValue::from_static(PROMETHEUS_CONTENT_TYPE),
    );

    (headers, buffer)
}

fn broadcast() -> HashMap<SocketAddr, BroadcastResponse> {
    let socket = UdpSocket::bind(BROADCAST_BIND_ADDR).unwrap();
    socket.set_read_timeout(Some(BROADCAST_WAIT_TIME)).unwrap();
    socket.set_broadcast(true).unwrap();

    let msg = encrypt(BROADCAST_MESSAGE);

    socket
        .send_to(&msg, BROADCAST_SEND_ADDR)
        .expect("error broadcasting");

    let mut buf = [0u8; BROADCAST_RESPONSE_BUFFER_SIZE];
    let mut responses = HashMap::default();

    while let Ok((n, addr)) = socket.recv_from(&mut buf) {
        responses.insert(addr, from_slice(&decrypt(&buf[0..n])).expect("ugh"));
    }

    responses
}

fn check_one(endpoint: &SocketAddr) -> Option<BroadcastResponse> {
    let stream = std::net::TcpStream::connect(endpoint).ok()?;
    tplink_shome_protocol::send_message(&stream, &String::from_utf8_lossy(BROADCAST_MESSAGE))
        .ok()?;
    tplink_shome_protocol::receive_message(&stream)
        .ok()
        .map(|message| from_str(&message).expect("ugh"))
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
