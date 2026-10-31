//! Network plumbing (M18): Settings › Network compiled for the dialer.
//!
//! The `network` settings row is owned by `settings.rs`; this module turns
//! it into what `proxy::dial` asks on every connection (which upstream
//! proxy, which DNS override, which client certificate), keeps the
//! passwords in the [`SecretStore`] and runs the reverse proxy listeners.
//!
//! Passwords and PKCS#12 identities are read lazily, the first time a
//! connection needs them, so launching never waits on the keychain. A save
//! loads every identity straight away, so a wrong path or password is
//! reported to whoever saved it.

use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use globset::GlobSet;
use parking_lot::Mutex;
use pm_model::{
    ClientIdentity, ListenerKind, ListenerStatus, NetworkSettings, ProxyServer, ReverseProxy, UpstreamProxy,
};
use pm_platform::secrets::SecretStore;
use pm_platform::sysproxy::PreviousProxy;
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::sign::CertifiedKey;

use crate::proxy::reverse::ReverseHandle;
use crate::tls::globs;
use crate::{Engine, Error};

/// How long the system's previous proxy is trusted before it is read again.
const PREVIOUS_TTL: Duration = Duration::from_secs(30);

/// The engine's network state.
pub(crate) struct Network {
    secrets: Box<dyn SecretStore>,
    compiled: ArcSwap<Compiled>,
    previous: Mutex<Option<(Instant, Option<Arc<PreviousProxy>>)>>,
    pub(crate) reverse: Mutex<Vec<ReverseListener>>,
}

pub(crate) struct ReverseListener {
    pub config: ReverseProxy,
    pub state: Result<ReverseHandle, String>,
}

impl Network {
    pub fn new(secrets: Box<dyn SecretStore>, settings: &NetworkSettings) -> Network {
        let compiled = Compiled::new(settings).unwrap_or_else(|err| {
            eprintln!("proxymate: {err}; network settings ignored");
            Compiled::default()
        });
        Network {
            secrets,
            compiled: ArcSwap::from_pointee(compiled),
            previous: Mutex::new(None),
            reverse: Mutex::new(Vec::new()),
        }
    }

    pub fn compiled(&self) -> Arc<Compiled> {
        self.compiled.load_full()
    }

    pub fn secrets(&self) -> &dyn SecretStore {
        &*self.secrets
    }
}

/// One upstream proxy server, with its password once read.
pub(crate) struct Server {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    secret_key: Option<String>,
    password: OnceLock<Option<String>>,
}

impl Server {
    fn new(kind: &str, server: &ProxyServer) -> Server {
        Server {
            host: server.host.trim().to_owned(),
            port: server.port,
            username: server.username.clone().filter(|u| !u.is_empty()),
            secret_key: server.has_password.then(|| proxy_secret_key(kind, server)),
            password: OnceLock::new(),
        }
    }

    /// The username and password, if the server has a username.
    pub fn credentials(&self, secrets: &dyn SecretStore) -> Option<(String, String)> {
        let user = self.username.clone()?;
        let password = self
            .password
            .get_or_init(|| {
                let key = self.secret_key.as_deref()?;
                secrets.get(key).unwrap_or_else(|err| {
                    eprintln!("proxymate: can't read the password for {}: {err}", self.host);
                    None
                })
            })
            .clone()
            .unwrap_or_default();
        Some((user, password))
    }

    pub fn authority(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}

/// The hop a connection goes through.
#[derive(Clone)]
pub(crate) enum Hop {
    Direct,
    Http(Arc<Server>),
    Socks5(Arc<Server>),
}

impl Hop {
    /// The upstream proxy as shown in the inspector.
    pub fn label(&self) -> Option<String> {
        match self {
            Hop::Direct => None,
            Hop::Http(s) => Some(format!("http://{}", s.authority())),
            Hop::Socks5(s) => Some(format!("socks5://{}", s.authority())),
        }
    }
}

/// A loaded client certificate.
pub(crate) struct Identity {
    pub key: Arc<CertifiedKey>,
    /// The leaf's subject, for the TLS tab.
    pub subject: String,
}

struct CertEntry {
    identity: ClientIdentity,
    secret_key: Option<String>,
    loaded: OnceLock<Result<Arc<Identity>, String>>,
}

#[derive(Default)]
pub(crate) struct Compiled {
    rules: Vec<(GlobSet, Hop)>,
    bypass: GlobSet,
    use_previous: bool,
    dns: Vec<(GlobSet, IpAddr, String)>,
    certs: Vec<(GlobSet, CertEntry)>,
}

impl Compiled {
    fn new(settings: &NetworkSettings) -> Result<Compiled, Error> {
        let glob = |patterns: &[String]| globs(patterns).map_err(Error::from);
        let mut rules = Vec::new();
        for rule in settings.upstream.rules.iter().filter(|r| r.enabled) {
            let hop = match &rule.proxy {
                UpstreamProxy::Direct => Hop::Direct,
                UpstreamProxy::Http(s) => Hop::Http(Arc::new(Server::new("http", s))),
                UpstreamProxy::Socks5(s) => Hop::Socks5(Arc::new(Server::new("socks5", s))),
            };
            rules.push((glob(&rule.hosts)?, hop));
        }
        let mut dns = Vec::new();
        for o in settings.dns.iter().filter(|o| o.enabled) {
            let ip = parse_ip(&o.ip)?;
            dns.push((
                glob(std::slice::from_ref(&o.host))?,
                ip,
                format!("{} → {}", o.host.trim(), ip),
            ));
        }
        let mut certs = Vec::new();
        for c in settings.client_certs.iter().filter(|c| c.enabled) {
            let secret_key = match &c.identity {
                ClientIdentity::Pkcs12 { path, has_password, .. } => has_password.then(|| cert_secret_key(path)),
                ClientIdentity::Pem { .. } => None,
            };
            certs.push((
                glob(&c.hosts)?,
                CertEntry {
                    identity: c.identity.clone(),
                    secret_key,
                    loaded: OnceLock::new(),
                },
            ));
        }
        Ok(Compiled {
            rules,
            bypass: glob(&settings.upstream.bypass)?,
            use_previous: settings.upstream.use_system_previous,
            dns,
            certs,
        })
    }

    /// The IP a DNS override sends `host` to, and the override as shown.
    pub fn dns_override(&self, host: &str) -> Option<(IpAddr, &str)> {
        let host = normalize(host);
        self.dns
            .iter()
            .find(|(set, ..)| set.is_match(&host))
            .map(|(_, ip, label)| (*ip, label.as_str()))
    }

    /// The configured hop for `host`, or `None` to ask the previous system
    /// proxy (when that option is on).
    fn configured(&self, host: &str) -> Option<Hop> {
        let host = normalize(host);
        if self.bypass.is_match(&host) {
            return Some(Hop::Direct);
        }
        if let Some((_, hop)) = self.rules.iter().find(|(set, _)| set.is_match(&host)) {
            return Some(hop.clone());
        }
        (!self.use_previous).then_some(Hop::Direct)
    }

    /// The client certificate for `host`, loading it on first use.
    pub fn identity(&self, host: &str, secrets: &dyn SecretStore, provider: &CryptoProvider) -> Option<Arc<Identity>> {
        let host = normalize(host);
        let (_, entry) = self.certs.iter().find(|(set, _)| set.is_match(&host))?;
        match entry.load(None, secrets, provider) {
            Ok(identity) => Some(identity.clone()),
            Err(err) => {
                eprintln!("proxymate: client certificate for {host}: {err}");
                None
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty() && self.dns.is_empty() && self.certs.is_empty() && !self.use_previous
    }
}

impl CertEntry {
    /// `password` is the one being saved; otherwise it is read from the
    /// secret store.
    fn load(
        &self,
        password: Option<&str>,
        secrets: &dyn SecretStore,
        provider: &CryptoProvider,
    ) -> &Result<Arc<Identity>, String> {
        self.loaded.get_or_init(|| {
            let password = match password {
                Some(p) => p.to_owned(),
                None => self
                    .secret_key
                    .as_deref()
                    .map(|key| secrets.get(key).map_err(|e| format!("can't read its password: {e}")))
                    .transpose()?
                    .flatten()
                    .unwrap_or_default(),
            };
            load_identity(&self.identity, &password, provider).map(Arc::new)
        })
    }
}

/// Reads a client identity from disk.
fn load_identity(identity: &ClientIdentity, password: &str, provider: &CryptoProvider) -> Result<Identity, String> {
    use rustls::pki_types::pem::PemObject;
    let (chain, key): (Vec<CertificateDer<'static>>, PrivateKeyDer<'static>) = match identity {
        ClientIdentity::Pkcs12 { path, .. } => {
            let data = std::fs::read(path).map_err(|e| format!("can't read {path}: {e}"))?;
            let store =
                p12_keystore::KeyStore::from_pkcs12(&data, password, Default::default()).map_err(|e| match e {
                    p12_keystore::error::Error::MacError(_) => format!("{path}: wrong password"),
                    other => format!("{path} isn't a PKCS#12 file Proxymate can read: {other}"),
                })?;
            let (_, chain) = store
                .private_key_chain()
                .ok_or_else(|| format!("{path} has no private key with a certificate"))?;
            let certs = chain
                .certs()
                .iter()
                .map(|c| CertificateDer::from(c.as_der().to_vec()))
                .collect();
            let key = PrivateKeyDer::try_from(chain.key().as_der().to_vec())
                .map_err(|e| format!("{path}: unsupported private key: {e}"))?;
            (certs, key)
        }
        ClientIdentity::Pem { cert_path, key_path } => {
            let certs = CertificateDer::pem_file_iter(cert_path)
                .map_err(|e| format!("can't read {cert_path}: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("{cert_path}: {e}"))?;
            let key_file = if key_path.trim().is_empty() {
                cert_path
            } else {
                key_path
            };
            let key = PrivateKeyDer::from_pem_file(key_file)
                .map_err(|e| format!("no unencrypted private key in {key_file}: {e}"))?;
            (certs, key)
        }
    };
    let Some(leaf) = chain.first() else {
        return Err("the identity has no certificate".into());
    };
    let subject = x509_parser::parse_x509_certificate(leaf)
        .map(|(_, cert)| cert.subject().to_string())
        .unwrap_or_default();
    let key = CertifiedKey::from_der(chain, key, provider)
        .map_err(|e| format!("the private key doesn't fit the certificate: {e}"))?;
    Ok(Identity {
        key: Arc::new(key),
        subject,
    })
}

fn proxy_secret_key(kind: &str, server: &ProxyServer) -> String {
    format!(
        "upstream/{kind}/{}:{}/{}",
        server.host.trim().to_ascii_lowercase(),
        server.port,
        server.username.as_deref().unwrap_or_default()
    )
}

fn cert_secret_key(path: &str) -> String {
    format!("client-cert/{path}")
}

fn parse_ip(text: &str) -> Result<IpAddr, Error> {
    text.trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .parse()
        .map_err(|_| Error::Invalid(format!("{text:?} isn't an IP address")))
}

fn normalize(host: &str) -> String {
    host.trim_end_matches('.')
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase()
}

/// Whether `host` (a name or literal) is this machine, before resolving.
fn is_loopback_name(host: &str) -> bool {
    let host = normalize(host);
    host == "localhost" || host.ends_with(".localhost") || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// Validates new settings against the listeners in `own_ports`; returns
/// what to save and the secrets to write (`None`: delete).
fn check(
    settings: &mut NetworkSettings,
    own_ports: &[u16],
    is_local: impl Fn(IpAddr) -> bool,
) -> Result<Vec<(String, Option<String>)>, Error> {
    let mut secrets = Vec::new();
    let is_self = |host: &str, port: u16| {
        own_ports.contains(&port) && (is_loopback_name(host) || normalize(host).parse::<IpAddr>().is_ok_and(&is_local))
    };
    for (i, rule) in settings.upstream.rules.iter_mut().enumerate() {
        let n = i + 1;
        let (kind, server) = match &mut rule.proxy {
            UpstreamProxy::Direct => continue,
            UpstreamProxy::Http(s) => ("http", s),
            UpstreamProxy::Socks5(s) => ("socks5", s),
        };
        server.host = server.host.trim().to_owned();
        if server.host.is_empty() || server.port == 0 {
            return Err(Error::Invalid(format!("upstream rule {n} needs a proxy host and port")));
        }
        if rule.enabled && rule.hosts.iter().all(|h| h.trim().is_empty()) {
            return Err(Error::Invalid(format!(
                "upstream rule {n} matches no hosts (use * for every host)"
            )));
        }
        if is_self(&server.host, server.port) {
            return Err(Error::Invalid(format!(
                "upstream rule {n} points at Proxymate itself ({}:{}), which would loop",
                server.host, server.port
            )));
        }
        let key = proxy_secret_key(kind, server);
        match server.password.take() {
            Some(p) if p.is_empty() => {
                server.has_password = false;
                secrets.push((key, None));
            }
            Some(p) => {
                server.has_password = true;
                secrets.push((key, Some(p)));
            }
            None => {}
        }
        if server.username.as_deref().is_some_and(str::is_empty) {
            server.username = None;
        }
    }
    let mut ports = Vec::new();
    for rp in &settings.reverse_proxies {
        let port = rp.listen_port;
        if port < 1024 {
            return Err(Error::Invalid(format!(
                "reverse proxy port {port}: pick one from 1024 up (lower ones need administrator rights)"
            )));
        }
        if rp.enabled {
            if ports.contains(&port) {
                return Err(Error::Invalid(format!("two reverse proxies listen on port {port}")));
            }
            ports.push(port);
        }
        let url: url::Url = rp
            .target_url
            .trim()
            .parse()
            .map_err(|_| Error::Invalid(format!("{:?} isn't a URL", rp.target_url)))?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none_or(str::is_empty) {
            return Err(Error::Invalid(format!(
                "the reverse proxy target {:?} must be an http:// or https:// URL",
                rp.target_url
            )));
        }
        let target_port = url.port_or_known_default().unwrap_or(80);
        let host = url.host_str().unwrap_or_default();
        let loops = settings
            .reverse_proxies
            .iter()
            .filter(|o| o.enabled)
            .any(|o| o.listen_port == target_port)
            || own_ports.first() == Some(&target_port);
        if loops && (is_loopback_name(host) || normalize(host).parse::<IpAddr>().is_ok_and(&is_local)) {
            return Err(Error::Invalid(format!(
                "the reverse proxy on port {port} points at a Proxymate listener ({}), which would loop",
                rp.target_url
            )));
        }
    }
    if let Some(&main) = own_ports.first()
        && ports.contains(&main)
    {
        return Err(Error::Invalid(format!(
            "port {main} is the proxy's own port; pick another for the reverse proxy"
        )));
    }
    for o in &settings.dns {
        if o.host.trim().is_empty() {
            return Err(Error::Invalid("a DNS override needs a host".into()));
        }
        parse_ip(&o.ip)?;
    }
    for cert in &mut settings.client_certs {
        if cert.enabled && cert.hosts.iter().all(|h| h.trim().is_empty()) {
            return Err(Error::Invalid("a client certificate needs the hosts it is for".into()));
        }
        match &mut cert.identity {
            ClientIdentity::Pkcs12 {
                path,
                password,
                has_password,
            } => {
                let key = cert_secret_key(path);
                match password.as_deref() {
                    Some("") => {
                        *has_password = false;
                        secrets.push((key, None));
                    }
                    Some(p) => {
                        *has_password = true;
                        secrets.push((key, Some(p.to_owned())));
                    }
                    None => {}
                }
            }
            ClientIdentity::Pem { cert_path, .. } if cert_path.trim().is_empty() => {
                return Err(Error::Invalid(
                    "a PEM client certificate needs its certificate file".into(),
                ));
            }
            ClientIdentity::Pem { .. } => {}
        }
    }
    Ok(secrets)
}

/// Every secret key the settings refer to.
fn secret_keys(settings: &NetworkSettings) -> Vec<String> {
    let mut keys: Vec<String> = settings
        .upstream
        .rules
        .iter()
        .filter_map(|r| match &r.proxy {
            UpstreamProxy::Http(s) if s.has_password => Some(proxy_secret_key("http", s)),
            UpstreamProxy::Socks5(s) if s.has_password => Some(proxy_secret_key("socks5", s)),
            _ => None,
        })
        .collect();
    keys.extend(settings.client_certs.iter().filter_map(|c| match &c.identity {
        ClientIdentity::Pkcs12 {
            path,
            has_password: true,
            ..
        } => Some(cert_secret_key(path)),
        _ => None,
    }));
    keys
}

impl Engine {
    pub(crate) fn network(&self) -> &Network {
        &self.network
    }

    /// The hop for a connection to `host`. `tunnel` picks the previous
    /// system proxy's secure entry (TLS, WebSockets) over its web one.
    pub(crate) async fn upstream_hop(&self, host: &str, tunnel: bool) -> Hop {
        let compiled = self.network.compiled();
        if let Some(hop) = compiled.configured(host) {
            return hop;
        }
        let Some(previous) = self.previous_system_proxy().await else {
            return Hop::Direct;
        };
        let host = normalize(host);
        if globs(&previous.bypass).is_ok_and(|set| set.is_match(&host)) || is_loopback_name(&host) {
            return Hop::Direct;
        }
        let entry = if tunnel { &previous.secure } else { &previous.web };
        match entry.as_ref().or(previous.web.as_ref()) {
            Some((server, port)) => Hop::Http(Arc::new(Server {
                host: server.clone(),
                port: *port,
                username: None,
                secret_key: None,
                password: OnceLock::new(),
            })),
            None => Hop::Direct,
        }
    }

    /// The proxy the system had before Proxymate, read at most every
    /// [`PREVIOUS_TTL`] (the live settings take `networksetup` calls).
    async fn previous_system_proxy(&self) -> Option<Arc<PreviousProxy>> {
        if let Some((at, previous)) = &*self.network.previous.lock()
            && at.elapsed() < PREVIOUS_TTL
        {
            return previous.clone();
        }
        let addr = self.proxy_addr()?;
        let sysproxy = self.platform.sysproxy_handle();
        let previous = tokio::task::spawn_blocking(move || {
            sysproxy
                .previous(&addr.ip().to_string(), addr.port())
                .ok()
                .flatten()
                .map(Arc::new)
        })
        .await
        .ok()
        .flatten();
        *self.network.previous.lock() = Some((Instant::now(), previous.clone()));
        previous
    }

    /// True if `addr` is one of Proxymate's own listeners, so connecting
    /// to it would loop.
    pub(crate) fn is_own_endpoint(&self, addr: SocketAddr) -> bool {
        let ports = self.own_ports();
        ports.contains(&addr.port()) && self.device_registry().is_local(addr.ip())
    }

    /// The main listener's port first, then the reverse proxies'.
    fn own_ports(&self) -> Vec<u16> {
        let mut ports: Vec<u16> = self.proxy_addr().map(|a| a.port()).into_iter().collect();
        ports.extend(
            self.network
                .reverse
                .lock()
                .iter()
                .filter_map(|l| l.state.as_ref().ok().map(|h| h.addr.port())),
        );
        ports
    }

    /// The SOCKS5 entry and one per reverse proxy, for the toolbar and
    /// `get_status`.
    pub(crate) fn extra_listeners(&self, main: Option<&str>) -> Vec<ListenerStatus> {
        let mut listeners = vec![ListenerStatus {
            kind: ListenerKind::Socks5,
            addr: main.map(str::to_owned),
            target: None,
            error: None,
        }];
        for l in self.network.reverse.lock().iter() {
            listeners.push(ListenerStatus {
                kind: if l.config.tls {
                    ListenerKind::ReverseTls
                } else {
                    ListenerKind::Reverse
                },
                addr: l.state.as_ref().ok().map(|h| h.addr.to_string()),
                target: Some(l.config.target_url.clone()),
                error: l.state.as_ref().err().cloned(),
            });
        }
        listeners
    }

    /// Validates and saves Settings › Network, moves passwords into the
    /// secret store, and applies it: new connections use it at once, the
    /// plain-HTTP pool starts over and the reverse proxies are restarted.
    pub(crate) async fn apply_network_settings(
        self: &Arc<Self>,
        mut settings: NetworkSettings,
    ) -> Result<NetworkSettings, Error> {
        let mut own = self.own_ports();
        if own.is_empty() {
            own.push(self.proxy_settings().port);
        }
        // Reverse proxies being replaced don't count as our own ports.
        let main = own[0];
        let old = self.network_settings();
        let old_ports: Vec<u16> = old
            .reverse_proxies
            .iter()
            .filter(|r| r.enabled)
            .map(|r| r.listen_port)
            .collect();
        own.retain(|p| *p == main || !old_ports.contains(p));
        let registry = self.device_registry();
        let writes = check(&mut settings, &own, |ip| registry.is_local(ip))?;
        let compiled = Compiled::new(&settings)?;
        // Load every identity now, with the password being saved if any.
        let provider = rustls::crypto::ring::default_provider();
        for (cert, (_, entry)) in settings.client_certs.iter().filter(|c| c.enabled).zip(&compiled.certs) {
            let password = writes
                .iter()
                .find(|(key, _)| matches!(&cert.identity, ClientIdentity::Pkcs12 { path, .. } if *key == cert_secret_key(path)))
                .map(|(_, p)| p.clone().unwrap_or_default());
            if let Err(err) = entry.load(password.as_deref(), &*self.network.secrets, &provider) {
                return Err(Error::Invalid(format!("client certificate: {err}")));
            }
        }
        for (key, secret) in &writes {
            match secret {
                Some(secret) => self.network.secrets.set(key, secret)?,
                None => self.network.secrets.delete(key)?,
            }
        }
        let keep = secret_keys(&settings);
        for key in secret_keys(&old) {
            if !keep.contains(&key) {
                let _ = self.network.secrets.delete(&key);
            }
        }
        self.save_network_row(&settings)?;
        self.network.compiled.store(Arc::new(compiled));
        *self.network.previous.lock() = None;
        self.rebuild_pool();
        self.restart_reverse_proxies().await;
        Ok(settings)
    }

    /// Starts the enabled reverse proxies, stopping any that run.
    pub(crate) async fn restart_reverse_proxies(self: &Arc<Self>) {
        let old = std::mem::take(&mut *self.network.reverse.lock());
        for listener in old {
            if let Ok(handle) = listener.state {
                handle.stop().await;
            }
        }
        let mut started = Vec::new();
        for config in self
            .network_settings()
            .reverse_proxies
            .into_iter()
            .filter(|r| r.enabled)
        {
            let state = crate::proxy::reverse::start(self, &config).await;
            started.push(ReverseListener { config, state });
        }
        *self.network.reverse.lock() = started;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_model::{DnsOverride, UpstreamRule, UpstreamSettings};

    fn http(host: &str, port: u16) -> UpstreamProxy {
        UpstreamProxy::Http(ProxyServer {
            host: host.into(),
            port,
            ..Default::default()
        })
    }

    fn settings(rules: Vec<UpstreamRule>) -> NetworkSettings {
        NetworkSettings {
            upstream: UpstreamSettings {
                rules,
                bypass: vec!["*.internal".into()],
                use_system_previous: false,
            },
            ..Default::default()
        }
    }

    #[test]
    fn rules_bypass_and_dns_overrides_pick_the_route() {
        let mut s = settings(vec![
            UpstreamRule {
                enabled: true,
                hosts: vec!["*.example.com".into()],
                proxy: http("proxy.corp", 3128),
            },
            UpstreamRule {
                enabled: false,
                hosts: vec!["*".into()],
                proxy: http("off.corp", 1),
            },
        ]);
        s.dns.push(DnsOverride {
            enabled: true,
            host: "api.test".into(),
            ip: "127.0.0.1".into(),
        });
        let c = Compiled::new(&s).unwrap();
        assert!(matches!(c.configured("API.example.com."), Some(Hop::Http(s)) if s.host == "proxy.corp"));
        assert!(matches!(c.configured("db.internal"), Some(Hop::Direct)));
        assert!(matches!(c.configured("other.org"), Some(Hop::Direct)));
        assert_eq!(
            c.dns_override("api.test").map(|(ip, _)| ip.to_string()).as_deref(),
            Some("127.0.0.1")
        );
        assert!(c.dns_override("www.test").is_none());
    }

    #[test]
    fn checks_catch_loops_and_bad_input() {
        let local = |ip: IpAddr| ip.to_string() == "10.0.0.5";
        let mut s = settings(vec![UpstreamRule {
            enabled: true,
            hosts: vec!["*".into()],
            proxy: http("localhost", 9090),
        }]);
        let err = check(&mut s, &[9090], local).unwrap_err().to_string();
        assert!(err.contains("loop"), "{err}");
        let mut s = settings(vec![UpstreamRule {
            enabled: true,
            hosts: vec!["*".into()],
            proxy: http("10.0.0.5", 9090),
        }]);
        assert!(check(&mut s, &[9090], local).is_err());
        let mut s = settings(vec![UpstreamRule {
            enabled: true,
            hosts: vec!["*".into()],
            proxy: http("127.0.0.1", 3128),
        }]);
        assert!(check(&mut s, &[9090], local).is_ok());

        let mut s = NetworkSettings::default();
        s.reverse_proxies.push(ReverseProxy {
            enabled: true,
            listen_port: 8080,
            target_url: "http://localhost:8080".into(),
            preserve_host: false,
            tls: false,
        });
        assert!(check(&mut s, &[9090], local).unwrap_err().to_string().contains("loop"));
        s.reverse_proxies[0].target_url = "ftp://example.com".into();
        assert!(check(&mut s, &[9090], local).is_err());
        s.reverse_proxies[0].target_url = "http://localhost:3000/api".into();
        assert!(check(&mut s, &[9090], local).is_ok());
        s.reverse_proxies[0].listen_port = 9090;
        assert!(check(&mut s, &[9090], local).is_err());

        let mut s = NetworkSettings::default();
        s.dns.push(DnsOverride {
            enabled: true,
            host: "a.test".into(),
            ip: "nope".into(),
        });
        assert!(check(&mut s, &[9090], local).is_err());
    }

    #[test]
    fn passwords_leave_the_row() {
        let mut s = settings(vec![UpstreamRule {
            enabled: true,
            hosts: vec!["*".into()],
            proxy: UpstreamProxy::Socks5(ProxyServer {
                host: "socks.corp".into(),
                port: 1080,
                username: Some("me".into()),
                password: Some("pw".into()),
                has_password: false,
            }),
        }]);
        let writes = check(&mut s, &[9090], |_| false).unwrap();
        assert_eq!(
            writes,
            vec![("upstream/socks5/socks.corp:1080/me".to_owned(), Some("pw".to_owned()))]
        );
        let UpstreamProxy::Socks5(server) = &s.upstream.rules[0].proxy else {
            panic!()
        };
        assert_eq!(server.password, None);
        assert!(server.has_password);
        assert_eq!(secret_keys(&s), vec!["upstream/socks5/socks.corp:1080/me".to_owned()]);
    }
}
