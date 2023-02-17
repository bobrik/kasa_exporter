use std::{
    collections::HashMap,
    io::{Error, ErrorKind, Result},
    net::SocketAddr,
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
use futures::future::join_all;
use prometheus_client::{
    encoding::{text::encode, EncodeLabelSet},
    metrics::{counter::Counter, family::Family, gauge::Gauge},
    registry::Registry,
};
use serde_derive::Deserialize;
use serde_json::from_slice;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UdpSocket,
};
use tokio::{net::TcpStream, time::timeout};
use tplink_shome_protocol::{decrypt, encrypt};

const BROADCAST_BIND_ADDR: &str = "0.0.0.0:0";
const BROADCAST_SEND_ADDR: &str = "255.255.255.255:9999";

const BROADCAST_RESPONSE_BUFFER_SIZE: usize = 4096;

const REQUEST: &[u8] = r#"{"system":{"get_sysinfo":{}},"emeter":{"get_realtime":{}}}"#.as_bytes();

const RESPONSE_WAIT_TIME: Duration = Duration::from_millis(500);

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

    let mut responses = broadcast().await.unwrap_or_else(|e| {
        eprintln!("error getting broadcast responses: {e}");
        Default::default()
    });

    let mut combined = vec![];

    let endpoints = state
        .endpoints
        .lock()
        .expect("error locking endpoints")
        .clone();

    let mut rechecks = vec![];

    for (endpoint, last_seen) in endpoints.iter() {
        if let Some(response) = responses.remove(endpoint) {
            combined.push(response);
        } else {
            rechecks.push(async move {
                (
                    endpoint,
                    last_seen,
                    match timeout(RESPONSE_WAIT_TIME, check_one(endpoint)).await {
                        Ok(Ok(response)) => Some(response),
                        Ok(Err(e)) => {
                            eprintln!("error checking {endpoint}: {e}");
                            None
                        }
                        Err(e) => {
                            eprintln!("timed out error checking {endpoint}: {e}");
                            None
                        }
                    },
                )
            });
        }
    }

    let rechecks = join_all(rechecks).await;

    let mut remove = vec![];

    for (endpoint, last_seen, response) in rechecks {
        if let Some(response) = response {
            combined.push(response);
        } else if now.duration_since(*last_seen) > FORGET_TIMEOUT {
            eprintln!(
                "removed {endpoint} after not seeing it for {}s",
                FORGET_TIMEOUT.as_secs()
            );
            remove.push(endpoint);
        }
    }

    let mut endpoints = state.endpoints.lock().expect("error locking endpoints");

    for endpoint in remove {
        endpoints.remove(endpoint);
    }

    for (endpoint, response) in responses {
        eprintln!(
            "discovered {} at {endpoint}",
            response.system.get_sysinfo.alias
        );
        endpoints.insert(endpoint, Instant::now());
        combined.push(response);
    }

    let registry = into_registry(combined);

    let mut buffer = String::new();
    encode(&mut buffer, &registry).expect("error encoding prometheus data");

    let mut headers = HeaderMap::new();
    headers.insert(
        "content-type",
        HeaderValue::from_static(PROMETHEUS_CONTENT_TYPE),
    );

    (headers, buffer)
}

async fn broadcast() -> Result<HashMap<SocketAddr, Response>> {
    let socket = UdpSocket::bind(BROADCAST_BIND_ADDR).await?;
    socket.set_broadcast(true)?;

    let buf = encrypt(REQUEST);
    socket.send_to(&buf, BROADCAST_SEND_ADDR).await?;

    let mut buf = [0u8; BROADCAST_RESPONSE_BUFFER_SIZE];
    let mut responses = HashMap::default();

    while let Ok(Ok((n, addr))) = timeout(RESPONSE_WAIT_TIME, socket.recv_from(&mut buf)).await {
        let response: Response =
            from_slice(&decrypt(&buf[0..n])).map_err(|_| Error::from(ErrorKind::InvalidData))?;

        if response.emeter.get_realtime.is_some() {
            responses.insert(addr, response);
        }
    }

    Ok(responses)
}

async fn check_one(endpoint: &SocketAddr) -> Result<Response> {
    let mut stream = TcpStream::connect(endpoint).await?;

    let buf = encrypt(REQUEST);
    stream.write_all(&(buf.len() as u32).to_be_bytes()).await?;

    stream.write_all(&buf).await?;

    let mut buf = [0; 4];
    stream.read_exact(&mut buf).await?;

    let mut buf: Vec<u8> = vec![0; u32::from_be_bytes(buf) as usize];
    stream.read_exact(&mut buf).await?;

    from_slice(&decrypt(&buf)).map_err(|_| Error::from(ErrorKind::InvalidData))
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
struct MetricLabels {
    device_alias: String,
    device_id: String,
}

type GaugeMetric = Family<MetricLabels, Gauge<f64, AtomicU64>>;
type CounterMetric = Family<MetricLabels, Counter<f64, AtomicU64>>;

fn into_registry(responses: Vec<Response>) -> Registry {
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
        "device_electric_energy_joules",
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

        voltage.get_or_create(&labels).set(realtime.voltage());
        current.get_or_create(&labels).set(realtime.current());
        power.get_or_create(&labels).set(realtime.power());
        energy.get_or_create(&labels).inc_by(realtime.energy());
    }

    registry
}

#[derive(Deserialize, Debug)]
struct Response {
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

impl GetRealtimeResponse {
    fn voltage(&self) -> f64 {
        self.voltage
            .unwrap_or_else(|| self.voltage_mv.unwrap_or_default() as f64 / 1000.0)
    }

    fn current(&self) -> f64 {
        self.current
            .unwrap_or_else(|| self.current_ma.unwrap_or_default() as f64 / 1000.0)
    }

    fn power(&self) -> f64 {
        self.power
            .unwrap_or_else(|| self.power_mw.unwrap_or_default() as f64 / 1000.0)
    }

    fn energy(&self) -> f64 {
        self.total
            .unwrap_or_else(|| self.total_wh.unwrap_or_default() as f64 / 1000.0)
            * 1000.0
            * 3600.0
    }
}
