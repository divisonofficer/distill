use anyhow::ensure;

use crate::{NodeConfig, listen::ListenAddr};

pub fn listener_ports(cfg: &NodeConfig) -> Vec<u16> {
    [&cfg.listen, &cfg.untrusted_listen]
        .into_iter()
        .filter_map(|addr| match addr {
            ListenAddr::Tcp(addr) => Some(addr.port()),
            #[cfg(unix)]
            ListenAddr::Unix(_) => None,
        })
        .collect()
}

pub fn validate_node_config(cfg: &NodeConfig) -> anyhow::Result<()> {
    let ports = listener_ports(cfg);
    ensure!(!ports.contains(&0), "TCP listener port must be nonzero");
    ensure!(
        ports.len() < 2 || ports[0] != ports[1],
        "TCP listener ports must be distinct"
    );
    if let Some(origin) = cfg.untrusted_origin.as_deref() {
        let parsed = url::Url::parse(origin)?;
        ensure!(
            matches!(parsed.scheme(), "http" | "https") && parsed.host().is_some(),
            "untrusted origin must use HTTP or HTTPS"
        );
        ensure!(
            parsed.username().is_empty() && parsed.password().is_none(),
            "untrusted origin must not contain user information"
        );
        ensure!(
            parsed.path() == "/" && parsed.query().is_none() && parsed.fragment().is_none(),
            "untrusted origin must not contain path, query, or fragment"
        );
        let canonical = parsed.origin().ascii_serialization();
        ensure!(
            canonical == origin.trim_end_matches('/'),
            "untrusted origin must be an exact origin"
        );
        ensure!(
            !cfg.allowed_origins.iter().any(
                |allowed| url::Url::parse(allowed).is_ok_and(|u| u.origin() == parsed.origin())
            ),
            "untrusted origin must differ from allowed origins"
        );
        if let ListenAddr::Tcp(addr) = cfg.listen {
            let same_host = parsed.host_str().is_some_and(|host| {
                host == addr.ip().to_string() || (host == "localhost" && addr.ip().is_loopback())
            });
            ensure!(
                !same_host || parsed.port_or_known_default() != Some(addr.port()),
                "untrusted origin must differ from trusted TCP origin"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(origin: &str) -> NodeConfig {
        NodeConfig {
            owner: "test@example.com".into(),
            listen: ListenAddr::Tcp("127.0.0.1:7410".parse().unwrap()),
            untrusted_listen: ListenAddr::Tcp("127.0.0.1:7411".parse().unwrap()),
            allowed_origins: vec!["https://studio.example".into()],
            agent: crate::agent::AgentConfig::default(),
            state_dir: "/tmp/postudio-test".into(),
            web_dir: None,
            chat_peer_config: None,
            untrusted_origin: Some(origin.into()),
        }
    }

    #[test]
    fn validates_distinct_origins_and_prohibited_ports() {
        assert!(validate_node_config(&config("https://preview.example")).is_ok());
        assert_eq!(
            listener_ports(&config("https://preview.example")),
            vec![7410, 7411]
        );
        for origin in [
            "https://studio.example",
            "http://127.0.0.1:7410",
            "http://localhost:7410",
            "https://u:p@preview.example",
            "https://preview.example/path",
            "https://preview.example/?q=x",
            "https://preview.example/#x",
            "file:///tmp",
        ] {
            assert!(validate_node_config(&config(origin)).is_err(), "{origin}");
        }
        let mut cfg = config("https://preview.example");
        cfg.listen = ListenAddr::Tcp("127.0.0.1:0".parse().unwrap());
        assert!(validate_node_config(&cfg).is_err());
        cfg.listen = cfg.untrusted_listen.clone();
        assert!(validate_node_config(&cfg).is_err());
    }
}
