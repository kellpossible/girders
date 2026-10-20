//! Example 07: compact vs pretty output. The style is an encode-time
//! choice; the codec is unchanged and both forms decode to the same value.

use girders::prelude::*;

#[derive(Debug, Clone, PartialEq)]
pub struct Server {
    pub host: String,
    pub port: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub name: String,
    pub servers: Vec<Server>,
}

fn server_codec() -> impl Codec<Output = Server> {
    record((
        field("host", string(), |s: &Server| &s.host),
        field("port", i64(), |s: &Server| &s.port),
    ))
    .build(|(host, port)| Server { host, port })
}

pub fn config_codec() -> impl Codec<Output = Config> {
    record((
        field("name", string(), |c: &Config| &c.name),
        field("servers", list(server_codec()), |c: &Config| &c.servers),
    ))
    .build(|(name, servers)| Config { name, servers })
}

pub fn main() {
    let config = Config {
        name: "prod".into(),
        servers: vec![
            Server {
                host: "a".into(),
                port: 1,
            },
            Server {
                host: "b".into(),
                port: 2,
            },
        ],
    };
    let compact = girders_json::encode(&config, &config_codec());
    assert_eq!(
        compact,
        r#"{"name":"prod","servers":[{"host":"a","port":1},{"host":"b","port":2}]}"#
    );
    let pretty = girders_json::encode_pretty(&config, &config_codec(), "  ");
    assert_eq!(
        pretty,
        r#"{
  "name": "prod",
  "servers": [
    {
      "host": "a",
      "port": 1
    },
    {
      "host": "b",
      "port": 2
    }
  ]
}"#
    );
    assert_eq!(
        girders_json::decode(&compact, &config_codec()).unwrap(),
        config
    );
    assert_eq!(
        girders_json::decode(&pretty, &config_codec()).unwrap(),
        config
    );
    println!("07_pretty_printing ok");
}
