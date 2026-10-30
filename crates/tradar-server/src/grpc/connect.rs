use super::*;

/// A live gRPC target: the transport channel plus whatever schema has been
/// learned about it so far (reflection, and/or `.proto` files loaded by
/// hand). Kept together because `invoke` needs both and a connection with
/// an empty pool (reflection unsupported, no `.proto` loaded yet) is still
/// a valid, connected state -- just one `grpc.services` reports as empty.
pub struct GrpcConnection {
    pub channel: Channel,
    pub pool: DescriptorPool,
}

/// TLS settings carried in the target's query string, e.g.
/// `https://api.example.com:443?ca=/etc/ca.pem&cert=/me.pem&key=/me.key`.
/// `ca` adds a trust anchor on top of the bundled webpki roots (a private
/// CA), `cert` + `key` present a client certificate (mTLS), `domain`
/// overrides the name the server certificate is checked against, `insecure`
/// (`1`/`true`/`yes`, or just the word) does not check the certificate at
/// all -- for a self-signed or internal server.
#[derive(Debug, Default, PartialEq)]
pub struct TlsOptions {
    pub ca: Option<String>,
    pub cert: Option<String>,
    pub key: Option<String>,
    pub domain: Option<String>,
    pub insecure: bool,
}

/// Splits `target` into the URI to dial and its TLS options. Any other query
/// parameter stays in the URI untouched.
pub fn split_target(target: &str) -> anyhow::Result<(String, TlsOptions)> {
    let uri = if target.contains("://") {
        target.to_string()
    } else {
        format!("http://{target}")
    };
    let Some((base, query)) = uri.split_once('?') else {
        return Ok((uri, TlsOptions::default()));
    };
    let mut options = TlsOptions::default();
    let mut rest = Vec::new();
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if key == "insecure" {
            options.insecure = !matches!(value.to_ascii_lowercase().as_str(), "0" | "false" | "no");
            continue;
        }
        let slot = match key {
            "ca" => &mut options.ca,
            "cert" => &mut options.cert,
            "key" => &mut options.key,
            "domain" => &mut options.domain,
            _ => {
                rest.push(pair);
                continue;
            }
        };
        *slot = Some(value.to_string());
    }
    if options.cert.is_some() != options.key.is_some() {
        anyhow::bail!("`cert` and `key` go together (client certificate)");
    }
    let uri = if rest.is_empty() {
        base.to_string()
    } else {
        format!("{base}?{}", rest.join("&"))
    };
    Ok((uri, options))
}

/// `target` is a bare `host:port` (same convention as Cassandra/Kafka/
/// Socket) or a full `http(s)://` URL typed by hand. `https://` turns TLS on
/// (server certificate verified against the bundled webpki roots, plus
/// `?ca=` if given -- see `TlsOptions`); anything else is plaintext.
pub async fn connect(target: &str) -> anyhow::Result<Channel> {
    use tonic::transport::{Certificate, ClientTlsConfig, Identity};

    let (uri, options) = split_target(target)?;
    if options.insecure && uri.starts_with("https://") {
        return super::insecure::connect(&uri, &options).await;
    }
    let mut endpoint = Channel::from_shared(uri.clone())?;
    if uri.starts_with("https://") {
        let read = |path: &str| {
            std::fs::read(path).map_err(|e| anyhow::anyhow!("could not read `{path}`: {e}"))
        };
        let mut tls = ClientTlsConfig::new().with_webpki_roots();
        if let Some(ca) = &options.ca {
            tls = tls.ca_certificate(Certificate::from_pem(read(ca)?));
        }
        if let (Some(cert), Some(key)) = (&options.cert, &options.key) {
            tls = tls.identity(Identity::from_pem(read(cert)?, read(key)?));
        }
        if let Some(domain) = &options.domain {
            tls = tls.domain_name(domain.clone());
        }
        endpoint = endpoint.tls_config(tls)?;
    } else if options != TlsOptions::default() {
        anyhow::bail!("`ca`/`cert`/`key`/`domain`/`insecure` only apply to an `https://` target");
    }
    Ok(endpoint.connect().await?)
}
