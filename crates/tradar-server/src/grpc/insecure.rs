//! `?insecure=1` on an `https://` gRPC target: TLS without checking the server
//! certificate, for a self-signed or internal server. tonic's `ClientTlsConfig`
//! always verifies, so this builds the connection itself -- rustls with a
//! verifier that accepts any certificate (the handshake signatures are still
//! checked, so the session is encrypted, just not authenticated) -- and hands
//! the stream to tonic.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use hyper_util::rt::TokioIo;
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
use tokio_rustls::rustls::crypto::{CryptoProvider, ring};
use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use tokio_rustls::rustls::{ClientConfig, DigitallySignedStruct, Error, SignatureScheme};
use tonic::transport::{Channel, Uri};

use super::TlsOptions;

/// Accepts any server certificate.
#[derive(Debug)]
struct AcceptAny(Arc<CryptoProvider>);

impl ServerCertVerifier for AcceptAny {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        tokio_rustls::rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        tokio_rustls::rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// Dials `uri`'s host and runs the TLS handshake with `connector`; the server
/// name sent is `domain`, else the URI's host.
#[derive(Clone)]
struct Connector {
    tls: TlsConnector,
    domain: Option<String>,
}

impl tower_service::Service<Uri> for Connector {
    type Response = TokioIo<tokio_rustls::client::TlsStream<TcpStream>>;
    type Error = std::io::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, uri: Uri) -> Self::Future {
        let this = self.clone();
        Box::pin(async move {
            let other = |m: String| std::io::Error::other(m);
            let host = uri
                .host()
                .ok_or_else(|| other(format!("no host in `{uri}`")))?
                .trim_matches(['[', ']'])
                .to_string();
            let port = uri.port_u16().unwrap_or(443);
            let tcp = TcpStream::connect((host.as_str(), port)).await?;
            tcp.set_nodelay(true)?;
            let name = this.domain.clone().unwrap_or(host);
            let name = ServerName::try_from(name.clone())
                .map_err(|e| other(format!("`{name}` is not a valid server name: {e}")))?;
            let tls = this.tls.connect(name, tcp).await?;
            Ok(TokioIo::new(tls))
        })
    }
}

fn read(path: &str) -> anyhow::Result<Vec<u8>> {
    std::fs::read(path).map_err(|e| anyhow::anyhow!("could not read `{path}`: {e}"))
}

pub(super) async fn connect(uri: &str, options: &TlsOptions) -> anyhow::Result<Channel> {
    let provider = Arc::new(ring::default_provider());
    let builder = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAny(provider)));
    let mut config = match (&options.cert, &options.key) {
        (Some(cert), Some(key)) => {
            let certs = rustls_pemfile::certs(&mut read(cert)?.as_slice())
                .collect::<Result<Vec<_>, _>>()?;
            let key =
                rustls_pemfile::private_key(&mut read(key)?.as_slice())?.ok_or_else(|| {
                    anyhow::anyhow!(
                        "no private key in `{}`",
                        options.key.as_deref().unwrap_or("")
                    )
                })?;
            builder.with_client_auth_cert(certs, key)?
        }
        _ => builder.with_no_client_auth(),
    };
    config.alpn_protocols = vec![b"h2".to_vec()];
    let connector = Connector {
        tls: TlsConnector::from(Arc::new(config)),
        domain: options.domain.clone(),
    };
    // tonic refuses an `https://` endpoint it has no TLS config for, so it is
    // given the `http://` form to dial (our connector does the TLS itself) and
    // the real one as the origin of the requests.
    let origin: Uri = uri.parse()?;
    let dial = uri.replacen("https://", "http://", 1);
    Channel::from_shared(dial)?
        .origin(origin)
        .connect_with_connector(connector)
        .await
        .map_err(|e| {
            // tonic's own message is just "transport error": say why.
            let mut message = e.to_string();
            let mut source = std::error::Error::source(&e);
            while let Some(cause) = source {
                message.push_str(&format!(": {cause}"));
                source = cause.source();
            }
            anyhow::anyhow!(message)
        })
}
