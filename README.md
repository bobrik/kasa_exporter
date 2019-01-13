# kasa_exporter

[Prometheus](https://prometheus.io/) exporter for [TP-Link Kasa](https://www.kasasmart.com/us) smart home products.

This is an experiment in learning [Rust](https://www.rust-lang.org/) and [tokio](https://tokio.rs/) library.

## Supported products

### [Smart wifi plug (HS-110)](https://www.kasasmart.com/us/products/smart-plugs/kasa-smart-plug-energy-monitoring-hs110)

![HS-110](https://kasasmart.cdn.prismic.io/kasasmart/324e6a946178da38bd31dfaf6e8a2fa87b181959_hs110-product-image.png)

#### Exported metrics

All three as reported by API with `device_id` and `device_alias` labels: 

* `device_current`
* `device_power`
* `device_voltage`

## Building

[Install Rust](https://www.rust-lang.org/tools/install), then from cloned repo:

```
$ cargo build
``` 

## Usage

After [Building](#Building), run the command to get help:

```
$ ./target/debug/kasa_exporter --help
kasa_exporter 0.1.0
Ivan Babrou <hello@ivan.computer>


USAGE:
    kasa_exporter [OPTIONS] --kasa.password <kasa.password> --kasa.username <kasa.username>

FLAGS:
    -h, --help       Prints help information
    -V, --version    Prints version information

OPTIONS:
        --kasa.password <kasa.password>              Password to log into Kasa service
        --kasa.username <kasa.username>              Username to log into Kasa service
        --web.listen-address <web.listen-address>
            Address on which to expose metrics and web interface [default: :12345]
```

Note that `web.listen-address` expects `<ip>:<port>`, e.g.:

* `127.0.0.1:12345` for IPv4
* `[::1]:12345` for IPv6

Example response:

```
$ curl http://localhost:12345/
# HELP device_current Corrent reading from device
# TYPE device_current gauge
device_current{device_alias="Banana",device_id="800607035E84C0B634C36B7DF52CCEC3188C1BAB"} 0.25383
device_current{device_alias="Potato",device_id="800691A498F774D60997B91E241EE2CC18D08921"} 0.021187
# HELP device_power Power reading from device
# TYPE device_power gauge
device_power{device_alias="Banana",device_id="800607035E84C0B634C36B7DF52CCEC3188C1BAB"} 29.504412
device_power{device_alias="Potato",device_id="800691A498F774D60997B91E241EE2CC18D08921"} 0
# HELP device_voltage Voltage reading from device
# TYPE device_voltage gauge
device_voltage{device_alias="Banana",device_id="800607035E84C0B634C36B7DF52CCEC3188C1BAB"} 121.999552
device_voltage{device_alias="Potato",device_id="800691A498F774D60997B91E241EE2CC18D08921"} 121.528545
```

## License

MIT
