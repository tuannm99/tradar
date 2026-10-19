pub(crate) fn shape_reply(command: &str, args: &[&str], value: &redis::Value) -> serde_json::Value {
    match command.to_ascii_uppercase().as_str() {
        "HGETALL" => hgetall_to_object(value).unwrap_or_else(|| value_to_json(value)),
        "ZRANGE" | "ZREVRANGE" if args.iter().any(|a| a.eq_ignore_ascii_case("withscores")) => {
            zrange_withscores_to_pairs(value).unwrap_or_else(|| value_to_json(value))
        }
        _ => value_to_json(value),
    }
}

pub(crate) fn hgetall_to_object(value: &redis::Value) -> Option<serde_json::Value> {
    let redis::Value::Array(items) = value else {
        return None;
    };
    let mut object = serde_json::Map::new();
    for pair in items.chunks(2) {
        let [field, val] = pair else { return None };
        object.insert(value_to_string(field)?, value_to_json(val));
    }
    Some(serde_json::Value::Object(object))
}

pub(crate) fn zrange_withscores_to_pairs(value: &redis::Value) -> Option<serde_json::Value> {
    let redis::Value::Array(items) = value else {
        return None;
    };
    let mut pairs = Vec::new();
    for pair in items.chunks(2) {
        let [member, score] = pair else { return None };
        let mut entry = serde_json::Map::new();
        entry.insert(
            "member".to_string(),
            serde_json::Value::String(value_to_string(member)?),
        );
        entry.insert("score".to_string(), value_to_json(score));
        pairs.push(serde_json::Value::Object(entry));
    }
    Some(serde_json::Value::Array(pairs))
}

pub(crate) fn value_to_string(value: &redis::Value) -> Option<String> {
    match value {
        redis::Value::BulkString(bytes) => Some(String::from_utf8_lossy(bytes).to_string()),
        redis::Value::SimpleString(s) => Some(s.clone()),
        redis::Value::Int(i) => Some(i.to_string()),
        _ => None,
    }
}

pub(crate) fn value_to_json(value: &redis::Value) -> serde_json::Value {
    match value {
        redis::Value::Nil => serde_json::Value::Null,
        redis::Value::Int(i) => serde_json::Value::Number((*i).into()),
        redis::Value::BulkString(bytes) => {
            serde_json::Value::String(String::from_utf8_lossy(bytes).to_string())
        }
        redis::Value::SimpleString(s) => serde_json::Value::String(s.clone()),
        redis::Value::Okay => serde_json::Value::String("OK".to_string()),
        redis::Value::Array(items) | redis::Value::Set(items) => {
            serde_json::Value::Array(items.iter().map(value_to_json).collect())
        }
        redis::Value::Map(pairs) => {
            let mut object = serde_json::Map::new();
            for (key, val) in pairs {
                if let Some(key) = value_to_string(key) {
                    object.insert(key, value_to_json(val));
                }
            }
            serde_json::Value::Object(object)
        }
        redis::Value::Double(d) => serde_json::Number::from_f64(*d)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        redis::Value::Boolean(b) => serde_json::Value::Bool(*b),
        redis::Value::VerbatimString { text, .. } => serde_json::Value::String(text.clone()),
        _ => serde_json::Value::Null,
    }
}
