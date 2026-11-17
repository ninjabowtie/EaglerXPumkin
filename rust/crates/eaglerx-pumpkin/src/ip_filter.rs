//! IPv4/IPv6 allow and block lists with optional CIDR prefixes.
//!
//! Rules are parsed once at startup. A non-empty allow list restricts access to
//! matching addresses; the block list always wins. IPv4-mapped IPv6 peers are
//! matched against IPv4 rules so dual-stack listeners behave consistently.
use std::net::IpAddr;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct IpMask {
    addr: IpAddr,
    prefix: u8,
}

impl IpMask {
    fn parse(value: &str) -> Result<Self, String> {
        let value = value.trim();
        if value.is_empty() {
            return Err("IP filter entries cannot be empty".into());
        }
        let (addr, prefix) = match value.split_once('/') {
            Some((addr, prefix)) => (addr, Some(prefix)),
            None => (value, None),
        };
        let mut addr: IpAddr = addr
            .parse()
            .map_err(|_| format!("invalid IP filter entry: {value}"))?;
        let bits = if addr.is_ipv4() { 32 } else { 128 };
        let mut prefix = match prefix {
            Some(prefix) => prefix
                .parse::<u8>()
                .map_err(|_| format!("invalid CIDR prefix in {value}"))?,
            None => bits,
        };
        if prefix > bits {
            return Err(format!("CIDR prefix out of range in {value}"));
        }
        // Normalize ::ffff:a.b.c.d/1xx to the equivalent IPv4 rule.
        if let IpAddr::V6(v6) = addr
            && let Some(v4) = v6.to_ipv4_mapped()
            && prefix >= 96
        {
            addr = IpAddr::V4(v4);
            prefix -= 96;
        }
        Ok(Self { addr, prefix })
    }

    fn contains(&self, ip: IpAddr) -> bool {
        match (self.addr, ip) {
            (IpAddr::V4(net), IpAddr::V4(ip)) => {
                let mask = mask32(self.prefix);
                (u32::from(net) & mask) == (u32::from(ip) & mask)
            }
            (IpAddr::V6(net), IpAddr::V6(ip)) => {
                let mask = mask128(self.prefix);
                (u128::from(net) & mask) == (u128::from(ip) & mask)
            }
            _ => false,
        }
    }
}

fn mask32(prefix: u8) -> u32 {
    if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix as u32)
    }
}

fn mask128(prefix: u8) -> u128 {
    if prefix == 0 {
        0
    } else {
        u128::MAX << (128 - prefix as u32)
    }
}

fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        ip => ip,
    }
}

/// Parsed address policy for new connections.
#[derive(Default)]
pub struct IpFilter {
    allow: Vec<IpMask>,
    block: Vec<IpMask>,
}

impl IpFilter {
    pub fn parse(allow: &[String], block: &[String]) -> Result<Self, String> {
        Ok(Self {
            allow: allow
                .iter()
                .map(|entry| IpMask::parse(entry))
                .collect::<Result<_, _>>()?,
            block: block
                .iter()
                .map(|entry| IpMask::parse(entry))
                .collect::<Result<_, _>>()?,
        })
    }

    pub fn allows(&self, ip: IpAddr) -> bool {
        let ip = canonical(ip);
        if self.block.iter().any(|rule| rule.contains(ip)) {
            return false;
        }
        self.allow.is_empty() || self.allow.iter().any(|rule| rule.contains(ip))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(value: &str) -> IpAddr {
        value.parse().unwrap()
    }

    #[test]
    fn parses_cidr_and_exact_rules() {
        let filter = IpFilter::parse(
            &["192.168.1.0/24".into(), "::1".into(), "10.0.0.0/8".into()],
            &["192.168.1.9".into()],
        )
        .unwrap();
        assert!(filter.allows(ip("192.168.1.5")));
        assert!(!filter.allows(ip("192.168.1.9")));
        assert!(!filter.allows(ip("192.168.2.5")));
        assert!(filter.allows(ip("10.22.33.44")));
        assert!(filter.allows(ip("::1")));
        assert!(!filter.allows(ip("2001:db8::1")));
    }

    #[test]
    fn empty_allow_list_permits_everything_but_blocks_still_apply() {
        let filter = IpFilter::parse(&[], &["203.0.113.0/24".into()]).unwrap();
        assert!(filter.allows(ip("198.51.100.1")));
        assert!(!filter.allows(ip("203.0.113.7")));
    }

    #[test]
    fn zero_prefix_and_ipv4_mapped_addresses() {
        let all = IpFilter::parse(&["0.0.0.0/0".into(), "::/0".into()], &[]).unwrap();
        assert!(all.allows(ip("8.8.8.8")));
        assert!(all.allows(ip("2001:db8::1")));
        let mapped = IpFilter::parse(&["::ffff:127.0.0.0/120".into()], &[]).unwrap();
        assert!(mapped.allows(ip("127.0.0.1")));
        assert!(mapped.allows(ip("::ffff:127.0.0.1")));
        assert!(!mapped.allows(ip("127.0.1.1")));
    }

    #[test]
    fn invalid_rules_are_rejected() {
        for entry in ["", "/24", "not-an-ip", "10.0.0.0/33", "::1/129"] {
            assert!(
                IpFilter::parse(&[entry.into()], &[]).is_err(),
                "expected {entry:?} to fail"
            );
        }
    }
}
