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
    let document = context.to_json();

    let environment = document["environment"].as_str().unwrap_or_default();
    let debug = document["debug"].as_bool().unwrap_or(false);
    let timeout = document["timeout"].as_i64().unwrap_or(0);
    println!("env={environment} debug={debug} timeout={timeout}");

    if let Some(databases) = document["Database"].as_array() {
        if let Some(primary) = databases.first() {
            let host = primary["host"].as_str().unwrap_or_default();
            let port = primary["port"].as_i64().unwrap_or(0);
            println!("primary database -> {host}:{port}");
        }
    }

    Ok(())
}
```

Run the parser against other files with `tyco_rust::loads(&content)`; both entrypoints return a
fully-rendered `TycoContext`, so calling `to_json()` yields the same structure as the Python
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
# Global configuration with type annotations
str environment: production
bool debug: false
int timeout: 30

# Database configuration struct
Database:
 *str name:           # Primary key field (*)
  str host:
  int port:
  str connection_string:
  # Instances
  - primary, localhost,    5432, "postgresql://localhost:5432/myapp"
  - replica, replica-host, 5432, "postgresql://replica-host:5432/myapp"

# Server configuration struct  
Server:
 *str name:           # Primary key for referencing
  int port:
  str host:
  ?str description:   # Nullable field (?) - can be null
  # Server instances
  - web1,    8080, web1.example.com,    description: "Primary web server"
  - api1,    3000, api1.example.com,    description: null
  - worker1, 9000, worker1.example.com, description: "Worker number 1"

# Feature flags array
str[] features: [auth, analytics, caching]
```
