use serde_json::Value;

#[allow(dead_code)]
pub fn redact(v: &mut Value) {
    if let Value::Object(map) = v {
        for k in [
            "generated_at",
            "cxpak_version",
            "timestamp",
            "baseline_date",
        ] {
            if map.contains_key(k) {
                map.insert(k.into(), Value::String("[REDACTED]".into()));
            }
        }
        for vv in map.values_mut() {
            redact(vv);
        }
    } else if let Value::Array(arr) = v {
        for vv in arr {
            redact(vv);
        }
    }
}
