# Tyco Rust

Rust implementation of the Tyco configuration language parser. This crate mirrors the Python
reference implementation and stays in sync with the shared `tyco-test-suite`.

## Quick Start

Every binding bundles the canonical sample configuration under `tyco/example.tyco`
([view on GitHub](https://github.com/typedconfig/tyco-rust/blob/main/tyco/example.tyco)).
Load it to explore globals, structs, and references exactly like the Python README:

```rust
use tyco_rust::{load, TycoError};

fn main() -> Result<(), TycoError> {
    let context = load("tyco/example.tyco")?;
    let document = context.to_object();

    let timezone = document["timezone"].as_str().unwrap_or_default();
    println!("timezone={timezone}");

    if let Some(apps) = document["Application"].as_array() {
        if let Some(primary) = apps.first() {
            let service = primary["service"].as_str().unwrap_or_default();
            let command = primary["command"].as_str().unwrap_or_default();
            println!("primary service -> {service} ({command})");
        }
    }

    if let Some(hosts) = document["Host"].as_array() {
        if hosts.len() > 1 {
            let backup = &hosts[1];
            let hostname = backup["hostname"].as_str().unwrap_or_default();
            let cores = backup["cores"].as_i64().unwrap_or(0);
            println!("host {hostname} cores={cores}");
        }
    }

    Ok(())
}
```

Run the parser against other files with `tyco_rust::loads(&content)`; both entrypoints return a
fully-rendered `TycoContext`, so calling `to_object()` (or the legacy `to_json()`) yields the same structure as the Python
example.

## Testing

```
cargo test
```

The test suite replays the fixtures from `../tyco-test-suite`, ensuring behaviour stays aligned with
the other language bindings.

## Example Tyco File

```
tyco/example.tyco
```

```tyco
str timezone: UTC  # this is a global config setting

Application:       # schema defined first, followed by instance creation
  str service:
  str profile:
  str command: start_app {service}.{profile} -p {port.number}
  Host host:
  Port port: Port(http_web)  # reference to Port instance defined below
  - service: webserver, profile: primary, host: Host(prod-01-us)
  - service: webserver, profile: backup,  host: Host(prod-02-us)
  - service: database,  profile: mysql,   host: Host(prod-02-us), port: Port(http_mysql)

Host:
 *str hostname:  # star character (*) used as reference primary key
  int cores:
  bool hyperthreaded: true
  str os: Debian
  - prod-01-us, cores: 64, hyperthreaded: false
  - prod-02-us, cores: 32, os: Fedora

Port:
 *str name:
  int number:
  - http_web,   80  # can skip field keys when obvious
  - http_mysql, 3306
```
