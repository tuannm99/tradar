use super::*;

#[test]
fn index_fields_flattens_nested_properties_into_paths() {
    let mappings = serde_json::json!({
        "orders": {
            "mappings": {
                "properties": {
                    "id": {"type": "long"},
                    "customer": {
                        "properties": {
                            "name": {"type": "text"},
                            "address": {"properties": {"city": {"type": "keyword"}}}
                        }
                    }
                }
            }
        }
    });

    let fields = index_fields(&mappings, "orders");

    let named: Vec<(&str, &str)> = fields
        .iter()
        .map(|c| (c.name.as_str(), c.type_name.as_str()))
        .collect();
    assert!(named.contains(&("id", "long")));
    assert!(
        named.contains(&("customer.name", "text")),
        "a nested field is named the way you'd write it in a query: {named:?}"
    );
    assert!(named.contains(&("customer.address.city", "keyword")));
    assert!(
        !named.iter().any(|(name, _)| *name == "customer"),
        "an object node is not itself a field: {named:?}"
    );
}

#[test]
fn a_multi_field_is_not_reported_as_a_separate_field() {
    let mappings = serde_json::json!({
        "posts": {
            "mappings": {
                "properties": {
                    "title": {
                        "type": "text",
                        "fields": {"keyword": {"type": "keyword"}}
                    }
                }
            }
        }
    });

    let fields = index_fields(&mappings, "posts");

    assert_eq!(fields.len(), 1, "title.keyword is an indexing detail");
    assert_eq!(fields[0].name, "title");
    assert_eq!(fields[0].type_name, "text");
}

#[test]
fn an_index_with_no_mapping_reports_no_fields() {
    let mappings = serde_json::json!({"other": {"mappings": {}}});

    assert!(index_fields(&mappings, "missing").is_empty());
    assert!(index_fields(&mappings, "other").is_empty());
    assert!(index_fields(&serde_json::Value::Null, "any").is_empty());
}
