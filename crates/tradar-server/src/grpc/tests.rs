use super::*;

const PROTO: &str = r#"
syntax = "proto3";
package demo;
import "google/protobuf/timestamp.proto";

enum Level { LEVEL_UNSET = 0; LEVEL_HIGH = 1; }

message Address { string city = 1; Level level = 2; }

message Node { string id = 1; Node next = 2; }

message Req {
  string name = 1;
  int64 big = 2;
  bool flag = 3;
  bytes blob = 4;
  repeated string tags = 5;
  map<string, int32> counts = 6;
  Address address = 7;
  google.protobuf.Timestamp at = 8;
  oneof pick {
string by_name = 9;
int32 by_id = 10;
  }
  optional string note = 11;
  Node chain = 12;
}

service Svc { rpc Do (Req) returns (Address); }
"#;

fn pool() -> DescriptorPool {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("demo.proto");
    std::fs::write(&path, PROTO).unwrap();
    let mut pool = DescriptorPool::new();
    load_proto(&mut pool, path.to_str().unwrap()).unwrap();
    pool
}

#[test]
fn skeleton_has_every_field_at_its_default_in_declared_order() {
    let text = skeleton(&pool(), "demo.Svc", "Do", false).unwrap();
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();

    assert_eq!(
        value,
        serde_json::json!({
            "name": "",
            "big": 0,
            "flag": false,
            "blob": "",
            "tags": [],
            "counts": {},
            "address": {"city": "", "level": "LEVEL_UNSET"},
            "at": "1970-01-01T00:00:00Z",
            "byName": "",
            "note": "",
            "chain": {"id": "", "next": {}},
        })
    );
    // Declaration order is kept (a JSON object would not promise it).
    let name = text.find("\"name\"").unwrap();
    let big = text.find("\"big\"").unwrap();
    let chain = text.find("\"chain\"").unwrap();
    assert!(name < big && big < chain, "{text}");
    assert!(
        !text.contains("byId"),
        "only the first oneof member: {text}"
    );
}

#[test]
fn the_skeleton_is_a_request_the_server_would_accept() {
    let pool = pool();
    let text = skeleton(&pool, "demo.Svc", "Do", false).unwrap();
    let method = get_method(&pool, "demo.Svc", "Do").unwrap();
    let mut deserializer = serde_json::Deserializer::from_str(&text);
    DynamicMessage::deserialize(method.input(), &mut deserializer)
        .expect("the generated skeleton must deserialize into its own message");
}

#[test]
fn comments_in_the_skeleton_are_hints_that_do_not_change_the_json() {
    let proto = r#"
syntax = "proto3";
package demo;
enum Level { LEVEL_UNSET = 0; LEVEL_HIGH = 1; }
message Req {
  // Display name of the user.
  // Shown in the UI.
  string name = 1;
  Level level = 2;
  oneof pick {
string by_name = 3;
int32 by_id = 4;
  }
}
service Svc { rpc Do (Req) returns (Req); }
"#;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("c.proto");
    std::fs::write(&path, proto).unwrap();
    let mut pool = DescriptorPool::new();
    load_proto(&mut pool, path.to_str().unwrap()).unwrap();

    let with = skeleton(&pool, "demo.Svc", "Do", true).unwrap();
    assert!(with.contains("// Display name of the user.\n"), "{with}");
    assert!(with.contains("// Shown in the UI."), "{with}");
    assert!(
        with.contains("// enum Level: LEVEL_UNSET | LEVEL_HIGH"),
        "{with}"
    );
    assert!(
        with.contains("// oneof pick: byName | byId (only one)"),
        "{with}"
    );

    // Dropping the comment lines gives exactly the plain skeleton.
    let stripped: String = with
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    let plain = skeleton(&pool, "demo.Svc", "Do", false).unwrap();
    assert_eq!(stripped, plain);
    serde_json::from_str::<serde_json::Value>(&plain).unwrap();
}

#[test]
fn tls_options_come_out_of_the_target_query() {
    let (uri, options) =
        split_target("https://h:443?ca=/a.pem&cert=/c.pem&key=/k.pem&domain=x.test&other=1")
            .unwrap();
    assert_eq!(uri, "https://h:443?other=1");
    assert_eq!(options.ca.as_deref(), Some("/a.pem"));
    assert_eq!(options.cert.as_deref(), Some("/c.pem"));
    assert_eq!(options.key.as_deref(), Some("/k.pem"));
    assert_eq!(options.domain.as_deref(), Some("x.test"));

    let (uri, options) = split_target("localhost:50051").unwrap();
    assert_eq!(uri, "http://localhost:50051");
    assert_eq!(options, TlsOptions::default());

    // A client certificate needs its key.
    assert!(split_target("https://h?cert=/c.pem").is_err());
}

#[test]
fn an_unknown_method_is_an_error() {
    assert!(skeleton(&pool(), "demo.Svc", "Nope", false).is_err());
    assert!(skeleton(&pool(), "demo.Nope", "Do", false).is_err());
}
