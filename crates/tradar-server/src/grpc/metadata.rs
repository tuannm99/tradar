use super::*;

/// A JSON object of `name: value` strings into request metadata. Names are
/// lower-cased (gRPC metadata keys are case-insensitive, HTTP/2 wants them
/// lower); `-bin` keys carry binary values and aren't supported.
pub fn parse_metadata(value: Option<&serde_json::Value>) -> anyhow::Result<MetadataMap> {
    let mut map = MetadataMap::new();
    let Some(value) = value.filter(|v| !v.is_null()) else {
        return Ok(map);
    };
    let object = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("`metadata` must be an object {{name: value}}"))?;
    for (name, value) in object {
        let name = name.trim().to_ascii_lowercase();
        if name.ends_with("-bin") {
            anyhow::bail!("metadata `{name}`: `-bin` (binary) keys are not supported yet");
        }
        let text = match value {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        let key = MetadataKey::from_bytes(name.as_bytes())
            .map_err(|e| anyhow::anyhow!("metadata key `{name}` is invalid: {e}"))?;
        let val = MetadataValue::try_from(text.as_str())
            .map_err(|e| anyhow::anyhow!("metadata `{name}`: invalid value: {e}"))?;
        map.append(key, val);
    }
    Ok(map)
}

/// Response headers/trailers as a JSON object. A repeated key is joined with
/// `, ` (how HTTP folds them); a binary value shows its base64 form.
pub fn metadata_to_json(map: &MetadataMap) -> serde_json::Value {
    let mut out = serde_json::Map::new();
    for entry in map.iter() {
        let (key, value) = match entry {
            KeyAndValueRef::Ascii(k, v) => (
                k.as_str().to_string(),
                v.to_str().unwrap_or("<unreadable>").to_string(),
            ),
            KeyAndValueRef::Binary(k, v) => (
                k.as_str().to_string(),
                String::from_utf8_lossy(v.as_encoded_bytes()).into_owned(),
            ),
        };
        match out.get_mut(&key) {
            Some(serde_json::Value::String(existing)) => {
                existing.push_str(", ");
                existing.push_str(&value);
            }
            _ => {
                out.insert(key, serde_json::Value::String(value));
            }
        }
    }
    serde_json::Value::Object(out)
}
