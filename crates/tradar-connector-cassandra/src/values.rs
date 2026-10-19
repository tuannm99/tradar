use scylla::value::{CqlValue, Row};

/// Turns one cell into display text, the way `cqlsh` prints it: ISO
/// timestamps, plain decimals, `{a, b}` for a set, `[a, b]` for a list,
/// `{k: v}` for a map. An earlier version fell back to Rust's `Debug` for
/// anything composite or exotic, which put `Timestamp(CqlTimestamp(1785804417000))`
/// and `Decimal(CqlDecimal { int_val: CqlVarint([4, 226]), scale: 2 })` in
/// the grid -- found by running against a real Cassandra.
pub(crate) fn stringify_cql_value(value: &CqlValue) -> String {
    match value {
        CqlValue::Ascii(s) | CqlValue::Text(s) => s.clone(),
        CqlValue::Boolean(b) => b.to_string(),
        CqlValue::TinyInt(n) => n.to_string(),
        CqlValue::SmallInt(n) => n.to_string(),
        CqlValue::Int(n) => n.to_string(),
        CqlValue::BigInt(n) => n.to_string(),
        CqlValue::Counter(c) => c.0.to_string(),
        CqlValue::Float(n) => n.to_string(),
        CqlValue::Double(n) => n.to_string(),
        CqlValue::Uuid(u) => u.to_string(),
        CqlValue::Timeuuid(u) => u.to_string(),
        CqlValue::Inet(ip) => ip.to_string(),
        CqlValue::Blob(bytes) => format!("<blob {} bytes>", bytes.len()),
        CqlValue::Varint(v) => signed_be_to_decimal(v.as_signed_bytes_be_slice()),
        CqlValue::Decimal(d) => {
            let (bytes, scale) = d.as_signed_be_bytes_slice_and_exponent();
            format_decimal(&signed_be_to_decimal(bytes), scale)
        }
        CqlValue::Timestamp(t) => format_timestamp_ms(t.0),
        CqlValue::Date(d) => {
            let (y, m, day) = civil_from_days(i64::from(d.0) - (1i64 << 31));
            format!("{y:04}-{m:02}-{day:02}")
        }
        CqlValue::Time(t) => {
            let ns = t.0.max(0);
            let secs = ns / 1_000_000_000;
            format!(
                "{:02}:{:02}:{:02}.{:09}",
                secs / 3600,
                secs / 60 % 60,
                secs % 60,
                ns % 1_000_000_000
            )
        }
        CqlValue::Duration(d) => format!("{}mo{}d{}ns", d.months, d.days, d.nanoseconds),
        CqlValue::List(items) | CqlValue::Vector(items) => {
            format!("[{}]", join_nested(items.iter()))
        }
        CqlValue::Set(items) => format!("{{{}}}", join_nested(items.iter())),
        CqlValue::Map(pairs) => {
            let body: Vec<String> = pairs
                .iter()
                .map(|(k, v)| format!("{}: {}", nested(k), nested(v)))
                .collect();
            format!("{{{}}}", body.join(", "))
        }
        CqlValue::Tuple(items) => {
            let body: Vec<String> = items
                .iter()
                .map(|v| v.as_ref().map_or("NULL".to_string(), nested))
                .collect();
            format!("({})", body.join(", "))
        }
        CqlValue::UserDefinedType { fields, .. } => {
            let body: Vec<String> = fields
                .iter()
                .map(|(name, v)| {
                    format!("{name}: {}", v.as_ref().map_or("NULL".to_string(), nested))
                })
                .collect();
            format!("{{{}}}", body.join(", "))
        }
        CqlValue::Empty => String::new(),
        // `CqlValue` is non-exhaustive across driver releases; a variant
        // added later still shows something rather than failing to build.
        #[allow(unreachable_patterns)]
        other => format!("{other:?}"),
    }
}

/// A value inside a collection: text is single-quoted (`'a'`), as `cqlsh`
/// does, so `{'a, b'}` and `{'a', 'b'}` stay distinguishable.
pub(crate) fn nested(value: &CqlValue) -> String {
    match value {
        CqlValue::Ascii(s) | CqlValue::Text(s) => format!("'{}'", s.replace('\'', "''")),
        other => stringify_cql_value(other),
    }
}

pub(crate) fn join_nested<'a>(items: impl Iterator<Item = &'a CqlValue>) -> String {
    items.map(nested).collect::<Vec<_>>().join(", ")
}

/// A two's-complement big-endian integer of any width as decimal text --
/// what a CQL `varint`, and the unscaled part of a `decimal`, are on the
/// wire. Repeated division by 10^9 over base-2^32 limbs; no bignum crate.
pub(crate) fn signed_be_to_decimal(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return "0".to_string();
    }
    let negative = bytes[0] & 0x80 != 0;
    let mut magnitude: Vec<u8> = bytes.to_vec();
    if negative {
        // Two's complement -> magnitude: invert, then add one.
        for b in magnitude.iter_mut() {
            *b = !*b;
        }
        for b in magnitude.iter_mut().rev() {
            let (sum, carry) = b.overflowing_add(1);
            *b = sum;
            if !carry {
                break;
            }
        }
    }
    // Limbs, most significant first.
    let mut limbs: Vec<u32> = Vec::new();
    for chunk in magnitude.rchunks(4).collect::<Vec<_>>().into_iter().rev() {
        let mut limb = 0u32;
        for &b in chunk {
            limb = (limb << 8) | u32::from(b);
        }
        limbs.push(limb);
    }
    let mut groups: Vec<u32> = Vec::new();
    while limbs.iter().any(|&l| l != 0) {
        let mut remainder = 0u64;
        for limb in limbs.iter_mut() {
            let cur = (remainder << 32) | u64::from(*limb);
            *limb = (cur / 1_000_000_000) as u32;
            remainder = cur % 1_000_000_000;
        }
        groups.push(remainder as u32);
    }
    let mut out = String::new();
    if negative && !groups.is_empty() {
        out.push('-');
    }
    match groups.split_last() {
        None => return "0".to_string(),
        Some((top, rest)) => {
            out.push_str(&top.to_string());
            for g in rest.iter().rev() {
                out.push_str(&format!("{g:09}"));
            }
        }
    }
    out
}

/// `unscaled` (decimal text, maybe negative) with `scale` digits after the
/// point; a negative scale means trailing zeros.
pub(crate) fn format_decimal(unscaled: &str, scale: i32) -> String {
    let (sign, digits) = match unscaled.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", unscaled),
    };
    if scale <= 0 {
        let zeros = "0".repeat(scale.unsigned_abs() as usize);
        return if digits == "0" {
            "0".to_string()
        } else {
            format!("{sign}{digits}{zeros}")
        };
    }
    let scale = scale as usize;
    let padded = if digits.len() <= scale {
        format!("{}{digits}", "0".repeat(scale - digits.len() + 1))
    } else {
        digits.to_string()
    };
    let (int_part, frac_part) = padded.split_at(padded.len() - scale);
    format!("{sign}{int_part}.{frac_part}")
}

/// Days since 1970-01-01 -> (year, month, day), proleptic Gregorian
/// (Howard Hinnant's `civil_from_days`).
pub(crate) fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// A CQL `timestamp` (ms since the epoch) as `cqlsh` prints it, in UTC.
pub(crate) fn format_timestamp_ms(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let in_day = ms.rem_euclid(86_400_000);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}.{:03}+0000",
        in_day / 3_600_000,
        in_day / 60_000 % 60,
        in_day / 1_000 % 60,
        in_day % 1_000
    )
}

pub(crate) fn stringify_row(row: &Row) -> Vec<String> {
    row.columns
        .iter()
        .map(|cell| match cell {
            Some(value) => stringify_cql_value(value),
            None => "NULL".to_string(),
        })
        .collect()
}
