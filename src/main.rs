use clap::Parser;

const BROADCAST_BIND_ADDR: &str = "0.0.0.0:0";
const BROADCAST_SEND_ADDR: &str = "255.255.255.255:9999";

const BROADCAST_MESSAGE: &[u8] =
    r#"{"system":{"get_sysinfo":{}},"emeter":{"get_realtime":{}}}"#.as_bytes();

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Address on which to expose metrics and web interface.
    #[arg(long = "web.listen-address", default_value = "[::1]:12345")]
    listen_address: String,
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args = Args::parse();

    let addr = args
        .listen_address
        .parse()
        .expect("error parsing listen address");

    let app = axum::routing::Router::new().route("/metrics", axum::routing::get(metrics));

    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .expect("error running server");
}

async fn metrics() -> impl axum::response::IntoResponse {
    let socket = std::net::UdpSocket::bind(BROADCAST_BIND_ADDR).unwrap();
    socket
        .set_read_timeout(Some(std::time::Duration::from_millis(500)))
        .unwrap();
    socket.set_broadcast(true).unwrap();

    let msg = tplink_shome_protocol::encrypt(BROADCAST_MESSAGE);

    socket
        .send_to(&msg, BROADCAST_SEND_ADDR)
        .expect("error broadcasting");

    let mut buf = [0u8; 4096];
    let mut responses = vec![];

    while let Ok((n, _)) = socket.recv_from(&mut buf) {
        responses.push(
            serde_json::from_slice(&tplink_shome_protocol::decrypt(&buf[0..n])).expect("ugh"),
        );
    }

    let registry = into_registry(responses);

    let mut buffer = String::new();
    prometheus_client::encoding::text::encode(&mut buffer, &registry)
        .expect("error encoding prometheus data");

    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        "content-type",
        axum::http::HeaderValue::from_static(
            "application/openmetrics-text; version=1.0.0; charset=utf-8",
        ),
    );

    (headers, buffer)
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelSet)]
struct MetricLabels {
    device_alias: String,
    device_id: String,
}

fn into_registry(responses: Vec<BroadcastResponse>) -> prometheus_client::registry::Registry {
    let mut registry = prometheus_client::registry::Registry::default();

    let voltage = prometheus_client::metrics::family::Family::<
        MetricLabels,
        prometheus_client::metrics::gauge::Gauge<f64, std::sync::atomic::AtomicU64>,
    >::default();

    registry.register(
        "device_electric_potential_volts",
        "Voltage reading from device",
        voltage.clone(),
    );

    let current = prometheus_client::metrics::family::Family::<
        MetricLabels,
        prometheus_client::metrics::gauge::Gauge<f64, std::sync::atomic::AtomicU64>,
    >::default();

    registry.register(
        "device_electric_current_amperes",
        "Current reading from device",
        current.clone(),
    );

    let power = prometheus_client::metrics::family::Family::<
        MetricLabels,
        prometheus_client::metrics::gauge::Gauge<f64, std::sync::atomic::AtomicU64>,
    >::default();

    registry.register(
        "device_electric_power_watts",
        "Power reading from device",
        power.clone(),
    );

    let energy = prometheus_client::metrics::family::Family::<
        MetricLabels,
        prometheus_client::metrics::counter::Counter<f64, std::sync::atomic::AtomicU64>,
    >::default();

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

#[derive(serde_derive::Deserialize, Debug)]
struct BroadcastResponse {
    system: SystemResponse,
    emeter: EmeterResponse,
}

#[derive(serde_derive::Deserialize, Debug)]
struct SystemResponse {
    get_sysinfo: GetSysinfoResponse,
}

#[derive(serde_derive::Deserialize, Debug)]
struct GetSysinfoResponse {
    alias: String,
    #[serde(rename = "deviceId")]
    device_id: String,
}

#[derive(serde_derive::Deserialize, Debug)]
struct EmeterResponse {
    get_realtime: Option<GetRealtimeResponse>,
}

#[derive(serde_derive::Deserialize, Debug)]
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
