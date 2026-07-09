use std::borrow::Cow;
use std::str::FromStr;

use russh::keys::Algorithm;
use russh::{cipher, compression, kex, mac};

use crate::models::{SshAlgorithmCatalog, SshAlgorithms};

pub fn catalog() -> SshAlgorithmCatalog {
    SshAlgorithmCatalog {
        defaults: SshAlgorithms::default(),
        supported: supported_algorithms(),
    }
}

pub fn apply_to_config(config: &mut russh::client::Config, algorithms: &SshAlgorithms) {
    if let Some(kex) = parse_kex(&algorithms.kex) {
        config.preferred.kex = Cow::Owned(kex);
    }
    if let Some(key) = parse_key(&algorithms.key) {
        config.preferred.key = Cow::Owned(key);
    }
    if let Some(cipher) = parse_cipher(&algorithms.cipher) {
        config.preferred.cipher = Cow::Owned(cipher);
    }
    if let Some(mac) = parse_mac(&algorithms.mac) {
        config.preferred.mac = Cow::Owned(mac);
    }
    if let Some(compression) = parse_compression(&algorithms.compression) {
        config.preferred.compression = Cow::Owned(compression);
    }
}

fn supported_algorithms() -> SshAlgorithms {
    let defaults = SshAlgorithms::default();
    SshAlgorithms {
        kex: append_new(
            defaults.kex.clone(),
            kex::ALL_KEX_ALGORITHMS.iter().filter_map(|name| {
                let s = name.as_ref();
                (s != "none").then_some(s)
            }),
        ),
        key: append_new(
            defaults.key.clone(),
            russh::keys::key::ALL_KEY_TYPES
                .iter()
                .map(Algorithm::as_str),
        ),
        cipher: append_new(
            defaults.cipher.clone(),
            cipher::ALL_CIPHERS.iter().filter_map(|name| {
                let s = name.as_ref();
                (s != "clear" && s != "none").then_some(s)
            }),
        ),
        mac: append_new(
            defaults.mac.clone(),
            mac::ALL_MAC_ALGORITHMS.iter().filter_map(|name| {
                let s = name.as_ref();
                (s != "none").then_some(s)
            }),
        ),
        compression: append_new(
            defaults.compression.clone(),
            compression::ALL_COMPRESSION_ALGORITHMS
                .iter()
                .map(|name| name.as_ref()),
        ),
    }
}

fn append_new<'a>(mut base: Vec<String>, rest: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    for item in rest {
        if !base.iter().any(|existing| existing == item) {
            base.push(item.to_string());
        }
    }
    base
}

fn parse_kex(items: &[String]) -> Option<Vec<kex::Name>> {
    let parsed = items
        .iter()
        .filter_map(|name| kex_name(name.as_str()))
        .collect::<Vec<_>>();
    (!parsed.is_empty()).then_some(parsed)
}

fn parse_key(items: &[String]) -> Option<Vec<Algorithm>> {
    let parsed = items
        .iter()
        .filter(|name| supported_key_algorithm(name))
        .filter_map(|name| Algorithm::from_str(name).ok())
        .collect::<Vec<_>>();
    (!parsed.is_empty()).then_some(parsed)
}

fn parse_cipher(items: &[String]) -> Option<Vec<cipher::Name>> {
    let parsed = items
        .iter()
        .filter_map(|name| cipher::Name::try_from(name.as_str()).ok())
        .filter(|name| name.as_ref() != "clear" && name.as_ref() != "none")
        .collect::<Vec<_>>();
    (!parsed.is_empty()).then_some(parsed)
}

fn parse_mac(items: &[String]) -> Option<Vec<mac::Name>> {
    let parsed = items
        .iter()
        .filter_map(|name| mac::Name::try_from(name.as_str()).ok())
        .filter(|name| name.as_ref() != "none")
        .collect::<Vec<_>>();
    (!parsed.is_empty()).then_some(parsed)
}

fn parse_compression(items: &[String]) -> Option<Vec<compression::Name>> {
    let parsed = items
        .iter()
        .filter_map(|name| compression::Name::try_from(name.as_str()).ok())
        .collect::<Vec<_>>();
    (!parsed.is_empty()).then_some(parsed)
}

fn kex_name(name: &str) -> Option<kex::Name> {
    match name {
        "ext-info-c" => Some(kex::EXTENSION_SUPPORT_AS_CLIENT),
        "ext-info-s" => Some(kex::EXTENSION_SUPPORT_AS_SERVER),
        "kex-strict-c-v00@openssh.com" => Some(kex::EXTENSION_OPENSSH_STRICT_KEX_AS_CLIENT),
        "kex-strict-s-v00@openssh.com" => Some(kex::EXTENSION_OPENSSH_STRICT_KEX_AS_SERVER),
        "none" => None,
        _ => kex::Name::try_from(name).ok(),
    }
}

fn supported_key_algorithm(name: &str) -> bool {
    russh::keys::key::ALL_KEY_TYPES
        .iter()
        .any(|algorithm| algorithm.as_str() == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_includes_legacy_kex_but_defaults_do_not() {
        let c = catalog();
        assert!(c
            .supported
            .kex
            .contains(&"diffie-hellman-group1-sha1".into()));
        assert!(c
            .supported
            .kex
            .contains(&"diffie-hellman-group14-sha1".into()));
        assert!(!c
            .defaults
            .kex
            .contains(&"diffie-hellman-group1-sha1".into()));
        assert!(!c
            .defaults
            .kex
            .contains(&"diffie-hellman-group14-sha1".into()));
    }

    #[test]
    fn apply_config_filters_disabled_no_encryption_names() {
        let algorithms = SshAlgorithms {
            kex: vec!["curve25519-sha256".into()],
            key: vec!["unsupported-key".into(), "ssh-ed25519".into()],
            cipher: vec!["clear".into(), "none".into(), "aes128-ctr".into()],
            mac: vec!["none".into(), "hmac-sha1".into()],
            compression: vec!["none".into()],
        };
        let mut config = russh::client::Config::default();
        apply_to_config(&mut config, &algorithms);

        assert_eq!(config.preferred.cipher.len(), 1);
        assert_eq!(config.preferred.cipher[0].as_ref(), "aes128-ctr");
        assert_eq!(config.preferred.key.len(), 1);
        assert_eq!(config.preferred.key[0].as_str(), "ssh-ed25519");
        assert_eq!(config.preferred.mac.len(), 1);
        assert_eq!(config.preferred.mac[0].as_ref(), "hmac-sha1");
    }
}
