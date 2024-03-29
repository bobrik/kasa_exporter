# kasa_exporter

[Prometheus](https://prometheus.io/) exporter for [TP-Link Kasa](https://www.kasasmart.com/us) smart home products.

The metrics are exported for all devices find on a local network.
## Supported products

There might be others that have the same API. The following ones have been tested.

### [Smart wifi plug (HS110)](https://www.kasasmart.com/us/products/smart-plugs/kasa-smart-plug-energy-monitoring-hs110)

![HS110](https://images.prismic.io/kasasmart/324e6a946178da38bd31dfaf6e8a2fa87b181959_hs110-product-image.png?auto=compress,format)

### [Smart wifi plug (KP115)](https://www.kasasmart.com/us/products/smart-plugs/kasa-smart-plug-slim-energy-monitoring-kp115)

![KP115](https://images.prismic.io/kasasmart/01ca42d1-a4b0-42c6-a134-b1831477e5e7_KP115_Set-up+Images.png?auto=compress,format)

#### Exported metrics

All three as reported by API with `device_id` and `device_alias` labels:

* `device_electric_current_amperes`
* `device_electric_potential_volts`
* `device_electric_power_watts`
* `device_electric_energy_joules_total`

## Building

[Install Rust](https://www.rust-lang.org/tools/install), then from cloned repo:

```
$ cargo build --release
```

## Usage

After [Building](#Building), run the command to get help:

```
$ ./target/release/kasa_exporter --help
Prometheus exporter for TP-Link kasa devices

Usage: kasa_exporter [OPTIONS]

Options:
      --web.listen-address <LISTEN_ADDRESS>
          Address on which to expose metrics and web interface [default: [::1]:12345]
  -h, --help
          Print help
  -V, --version
          Print version
```

Note that `web.listen-address` expects `<ip>:<port>`, e.g.:

* `127.0.0.1:12345` for IPv4
* `[::1]:12345` for IPv6

Example response:

```
$ curl http://localhost:12345/
# HELP device_electric_current_amperes Corrent reading from device
# TYPE device_electric_current_amperes gauge
device_electric_current_amperes{device_alias="Banana",device_id="800607035E84C0B634C36B7DF52CCEC3188C1BAB"} 0.256972
device_electric_current_amperes{device_alias="Potato",device_id="800691A498F774D60997B91E241EE2CC18D08921"} 0.031424
# HELP device_electric_potential_volts Voltage reading from device
# TYPE device_electric_potential_volts gauge
device_electric_potential_volts{device_alias="Banana",device_id="800607035E84C0B634C36B7DF52CCEC3188C1BAB"} 123.16094
device_electric_potential_volts{device_alias="Potato",device_id="800691A498F774D60997B91E241EE2CC18D08921"} 123.130631
# HELP device_electric_power_watts Power reading from device
# TYPE device_electric_power_watts gauge
device_electric_power_watts{device_alias="Banana",device_id="800607035E84C0B634C36B7DF52CCEC3188C1BAB"} 30.071476
device_electric_power_watts{device_alias="Potato",device_id="800691A498F774D60997B91E241EE2CC18D08921"} 0.750854
```

## License

MIT
