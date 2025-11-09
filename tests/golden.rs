use std::{fs, path::PathBuf};

use serde_json::Value;
use tyco_rust::TycoParser;

fn shared_suite_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/shared")
}

#[test]
fn canonical_suite() {
    let inputs_dir = shared_suite_root().join("inputs");
    let expected_dir = shared_suite_root().join("expected");

    let mut entries = fs::read_dir(&inputs_dir)
        .expect("missing shared inputs")
        .filter_map(|entry| {
            entry.ok().and_then(|e| {
                let path = e.path();
                if path.extension().and_then(|ext| ext.to_str()) == Some("tyco") {
                    Some(path)
                } else {
                    None
                }
            })
        })
        .collect::<Vec<_>>();

    entries.sort();

    for input in entries {
        let name = input
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();
        let expected_path = expected_dir.join(format!("{name}.json"));
        if !expected_path.exists() {
            println!("SKIP {name}");
            continue;
        }

        let mut parser = TycoParser::new();
        let context = parser
            .parse_file(&input)
            .unwrap_or_else(|e| panic!("Failed to parse {name}: {e}"));
        let actual_json = context.to_json();
        let expected_json: Value = serde_json::from_str(
            &fs::read_to_string(&expected_path)
                .unwrap_or_else(|e| panic!("Cannot read expected {name}: {e}")),
        )
        .unwrap_or_else(|e| panic!("Invalid JSON for {name}: {e}"));

        assert_eq!(expected_json, actual_json, "Mismatch for {name}");
    }
}
