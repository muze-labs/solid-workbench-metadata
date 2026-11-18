use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};

use ipnet::IpNet;
use reqwest::{
    Client,
    header::{ACCEPT_ENCODING, LOCATION},
};
use url::{Host, Url};

fn routable(address: IpAddr) -> bool {
    let blocked: &[&str] = match address {
        IpAddr::V4(_) => &[
            "0.0.0.0/8",
            "10.0.0.0/8",
            "100.64.0.0/10",
            "127.0.0.0/8",
            "169.254.0.0/16",
            "172.16.0.0/12",
            "192.0.0.0/24",
            "192.0.2.0/24",
            "192.88.99.0/24",
            "192.168.0.0/16",
            "198.18.0.0/15",
            "198.51.100.0/24",
            "203.0.113.0/24",
            "224.0.0.0/3",
        ],
        IpAddr::V6(_) => {
            if !("2000::/3".parse::<IpNet>().unwrap().contains(&address)) {
                return false;
            }
            &["2001::/23", "2001:db8::/32", "2002::/16", "3fff::/20"]
        }
    };
    !blocked
        .iter()
        .any(|range| range.parse::<IpNet>().unwrap().contains(&address))
}

pub fn checked_url(mut url: Url) -> Option<Url> {
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.as_str().len() > 4096
    {
        return None;
    }
    match url.host()? {
        Host::Ipv4(ip) if !routable(ip.into()) => return None,
        Host::Ipv6(ip) if !routable(ip.into()) => return None,
        Host::Domain(domain) if !domain.trim_end_matches('.').contains('.') => return None,
        _ => {}
    }
    url.set_fragment(None);
    Some(url)
}

pub enum Body {
    Html,
    Json,
    Image,
}

pub async fn get(url: Url, kind: Body) -> Option<(Url, Vec<u8>)> {
    let mut target = url;
    for _ in 0..=3 {
        let url = checked_url(target)?;
        let mut client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(3))
            .user_agent("Solid-Workbench-Shortcut-Metadata/0.1");
        if let Some(Host::Domain(domain)) = url.host() {
            let port = url.port_or_known_default()?;
            let addresses: Vec<SocketAddr> = tokio::time::timeout(
                Duration::from_secs(2),
                tokio::net::lookup_host((domain, port)),
            )
            .await
            .ok()?
            .ok()?
            .collect();
            if addresses.is_empty() || addresses.iter().any(|address| !routable(address.ip())) {
                return None;
            }
            // Connect to the checked addresses while retaining the host for TLS.
            client = client.resolve_to_addrs(domain, &addresses);
        }
        let mut response = client
            .build()
            .ok()?
            .get(url.clone())
            .header(ACCEPT_ENCODING, "identity")
            .send()
            .await
            .ok()?;
        if matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
            target = url
                .join(response.headers().get(LOCATION)?.to_str().ok()?)
                .ok()?;
            continue;
        }
        if !response.status().is_success() {
            return None;
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .split(';')
            .next()?
            .trim()
            .to_ascii_lowercase();
        let limit = match kind {
            Body::Html
                if matches!(content_type.as_str(), "text/html" | "application/xhtml+xml") =>
            {
                128 * 1024
            }
            Body::Json
                if matches!(
                    content_type.as_str(),
                    "application/manifest+json"
                        | "application/json"
                        | "text/plain"
                        | "application/octet-stream"
                        | ""
                ) =>
            {
                64 * 1024
            }
            Body::Image => 512 * 1024,
            _ => return None,
        };
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.ok()? {
            let remaining = limit - bytes.len();
            if chunk.len() > remaining {
                if matches!(kind, Body::Html) {
                    bytes.extend_from_slice(&chunk[..remaining]);
                    break;
                }
                return None;
            }
            bytes.extend_from_slice(&chunk);
            if matches!(kind, Body::Html) && bytes.len() == limit {
                break;
            }
        }
        return Some((url, bytes));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outbound_address_policy() {
        for target in [
            "http://127.0.0.1/",
            "http://[::1]/",
            "http://[fc00::1]/",
            "http://[::ffff:192.168.0.1]/",
            "http://10.0.0.1/",
            "https://user@example.com/",
            "http://example.com:8080/",
            "file:///tmp/data",
        ] {
            assert!(
                checked_url(Url::parse(target).unwrap()).is_none(),
                "{target}"
            );
        }
        assert!(
            checked_url(Url::parse("https://example.com/path?query=yes#fragment").unwrap())
                .unwrap()
                .fragment()
                .is_none()
        );
        assert!(routable("8.8.8.8".parse().unwrap()));
        assert!(routable("2606:4700::1111".parse().unwrap()));
    }
}
