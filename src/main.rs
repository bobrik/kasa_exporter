// use anyhow::Result;
use prometheus::Encoder;

const BROADCAST_BIND_ADDR: &str = "0.0.0.0:0";
const BROADCAST_SEND_ADDR: &str = "255.255.255.255:9999";

const BROADCAST_MESSAGE: &[u8] =
    r#"{"system":{"get_sysinfo":{}},"emeter":{"get_realtime":{}}}"#.as_bytes();

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
                    v.parse::<std::net::SocketAddr>()
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                })
                .takes_value(true)
                .default_value("[::1]:12345"),
        )
        .get_matches();

    let addr = matches
        .value_of("web.listen-address")
        .unwrap()
        .parse()
        .unwrap();

    let service = hyper::service::make_service_fn(move |_| async move {
        Ok::<_, hyper::Error>(hyper::service::service_fn(|_| serve()))
    });

    hyper::Server::bind(&addr).serve(service).await?;

    Ok(())
}

pub async fn serve() -> Result<hyper::Response<hyper::Body>, hyper::Error> {
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

    let encoder = prometheus::TextEncoder::new();

    let mut buffer = vec![];
    encoder
        .encode(&registry(responses).gather(), &mut buffer)
        .unwrap();

    let mut http_response = hyper::Response::new(hyper::Body::from(buffer));

    let content_type = match encoder.format_type().parse() {
        Ok(content_type) => content_type,
        Err(e) => {
            return {
                eprintln!("error formatting content type: {e}");

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
fn registry(responses: Vec<BroadcastResponse>) -> prometheus::Registry {
    let voltage = gauge_vec(
        "device_electric_potential_volts",
        "Voltage reading from device",
        &["device_alias", "device_id"],
    );
    let current = gauge_vec(
        "device_electric_current_amperes",
        "Current reading from device",
        &["device_alias", "device_id"],
    );
    let power = gauge_vec(
        "device_electric_power_watts",
        "Power reading from device",
        &["device_alias", "device_id"],
    );
    let energy = gauge_vec(
        "device_electric_energy_joules_total",
        "Total energy consumed",
        &["device_alias", "device_id"],
    );

    let registry = prometheus::Registry::new();

    let collectors = vec![&voltage, &current, &power, &energy];

    for metric in collectors {
        registry.register(Box::new(metric.clone())).unwrap();
    }

    for response in responses {
        let realtime = match response.emeter.get_realtime {
            Some(realtime) => realtime,
            None => continue,
        };

        let labels = &prometheus::labels! {
            "device_alias" => response.system.get_sysinfo.alias.as_str(),
            "device_id"    => response.system.get_sysinfo.device_id.as_str(),
        };

        fill_metric! { labels = labels,
            voltage => if realtime.voltage.unwrap_or_default() > 0.0 {
                realtime.voltage
            } else {
                realtime.voltage_mv.map(|mv| mv as f64 / 1000.0)
            },
            current => if realtime.current.unwrap_or_default() > 0.0 {
                realtime.current
            } else {
                realtime.current_ma.map(|ma| ma as f64 / 1000.0)
            },
            power => if realtime.power.unwrap_or_default() > 0.0 {
                realtime.power
            } else {
                realtime.power_mw.map(|w| w as f64 / 1000.0)
            },
            energy => if realtime.total.unwrap_or_default() > 0.0 {
                realtime.total.map(|kwh| kwh * 3600.0 * 1000.0)
            } else {
                realtime.total_wh.map(|wh| wh as f64 * 3600.0)
            },
        };
    }

    registry
}

/// Creates Gauge vector with given parameters.
fn gauge_vec(name: &str, help: &str, labels: &[&str]) -> prometheus::GaugeVec {
    prometheus::GaugeVec::new(prometheus::opts!(name, help), labels).unwrap()
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
