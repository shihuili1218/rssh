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
    config.preferred.kex = Cow::Owned(parse_kex(&algorithms.kex));
    config.preferred.key = Cow::Owned(parse_key(&algorithms.key));
    config.preferred.cipher = Cow::Owned(parse_cipher(&algorithms.cipher));
    config.preferred.mac = Cow::Owned(parse_mac(&algorithms.mac));
    config.preferred.compression = Cow::Owned(parse_compression(&algorithms.compression));
}

/// Validate the persisted user policy, not russh's runtime-only KEX markers.
/// Unknown names may coexist with supported names for forward compatibility,
/// but every category must retain at least one algorithm we can actually use.
pub fn validate_policy(algorithms: &SshAlgorithms) -> Result<(), &'static str> {
    if !algorithms
        .kex
        .iter()
        .any(|name| selectable_kex_name(name).is_some())
    {
        return Err("kex");
    }
    if parse_key(&algorithms.key).is_empty() {
        return Err("key");
    }
    if parse_cipher(&algorithms.cipher).is_empty() {
        return Err("cipher");
    }
    if parse_mac(&algorithms.mac).is_empty() {
        return Err("mac");
    }
    if parse_compression(&algorithms.compression).is_empty() {
        return Err("compression");
    }
    Ok(())
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

fn parse_kex(items: &[String]) -> Vec<kex::Name> {
    let mut parsed = items
        .iter()
        .filter_map(|name| selectable_kex_name(name.as_str()))
        .collect::<Vec<_>>();
    parsed.extend([
        kex::EXTENSION_SUPPORT_AS_CLIENT,
        kex::EXTENSION_OPENSSH_STRICT_KEX_AS_CLIENT,
    ]);
    parsed
}

fn parse_key(items: &[String]) -> Vec<Algorithm> {
    items
        .iter()
        .filter(|name| supported_key_algorithm(name))
        .filter_map(|name| Algorithm::from_str(name).ok())
        .collect()
}

fn parse_cipher(items: &[String]) -> Vec<cipher::Name> {
    items
        .iter()
        .filter_map(|name| cipher::Name::try_from(name.as_str()).ok())
        .filter(|name| name.as_ref() != "clear" && name.as_ref() != "none")
        .collect()
}

fn parse_mac(items: &[String]) -> Vec<mac::Name> {
    items
        .iter()
        .filter_map(|name| mac::Name::try_from(name.as_str()).ok())
        .filter(|name| name.as_ref() != "none")
        .collect()
}

fn parse_compression(items: &[String]) -> Vec<compression::Name> {
    items
        .iter()
        .filter_map(|name| compression::Name::try_from(name.as_str()).ok())
        .collect()
}

fn selectable_kex_name(name: &str) -> Option<kex::Name> {
    match name {
        "none"
        | "ext-info-c"
        | "ext-info-s"
        | "kex-strict-c-v00@openssh.com"
        | "kex-strict-s-v00@openssh.com" => None,
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
    fn catalog_does_not_expose_kex_protocol_markers_as_algorithms() {
        let c = catalog();
        for marker in [
            "ext-info-c",
            "ext-info-s",
            "kex-strict-c-v00@openssh.com",
            "kex-strict-s-v00@openssh.com",
        ] {
            assert!(!c.defaults.kex.iter().any(|item| item == marker));
            assert!(!c.supported.kex.iter().any(|item| item == marker));
        }
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

    #[test]
    fn apply_config_adds_client_kex_protocol_markers_outside_user_policy() {
        let algorithms = SshAlgorithms {
            kex: vec!["curve25519-sha256".into()],
            ..SshAlgorithms::default()
        };
        let mut config = russh::client::Config::default();

        apply_to_config(&mut config, &algorithms);

        let names = config
            .preferred
            .kex
            .iter()
            .map(AsRef::<str>::as_ref)
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "curve25519-sha256",
                "ext-info-c",
                "kex-strict-c-v00@openssh.com",
            ]
        );
    }

    #[test]
    fn apply_config_does_not_turn_marker_only_policy_into_real_kex() {
        let algorithms = SshAlgorithms {
            kex: vec![
                "ext-info-c".into(),
                "ext-info-s".into(),
                "kex-strict-c-v00@openssh.com".into(),
                "kex-strict-s-v00@openssh.com".into(),
            ],
            ..SshAlgorithms::default()
        };
        let mut config = russh::client::Config::default();

        apply_to_config(&mut config, &algorithms);

        assert_eq!(
            config
                .preferred
                .kex
                .iter()
                .map(AsRef::<str>::as_ref)
                .collect::<Vec<_>>(),
            vec!["ext-info-c", "kex-strict-c-v00@openssh.com"]
        );
    }

    #[test]
    fn apply_config_fails_closed_when_policy_has_no_supported_cipher() {
        let algorithms = SshAlgorithms {
            cipher: vec!["unknown-cipher".into(), "clear".into(), "none".into()],
            ..SshAlgorithms::default()
        };
        let mut config = russh::client::Config::default();

        apply_to_config(&mut config, &algorithms);

        assert!(config.preferred.cipher.is_empty());
    }

    #[test]
    fn apply_config_fails_closed_when_policy_has_no_supported_host_key() {
        let algorithms = SshAlgorithms {
            key: vec!["unknown-host-key".into()],
            ..SshAlgorithms::default()
        };
        let mut config = russh::client::Config::default();

        apply_to_config(&mut config, &algorithms);

        assert!(config.preferred.key.is_empty());
    }

    #[test]
    fn apply_config_fails_closed_when_policy_has_no_supported_mac() {
        let algorithms = SshAlgorithms {
            mac: vec!["unknown-mac".into(), "none".into()],
            ..SshAlgorithms::default()
        };
        let mut config = russh::client::Config::default();

        apply_to_config(&mut config, &algorithms);

        assert!(config.preferred.mac.is_empty());
    }

    #[test]
    fn apply_config_fails_closed_when_policy_has_no_supported_compression() {
        let algorithms = SshAlgorithms {
            compression: vec!["unknown-compression".into()],
            ..SshAlgorithms::default()
        };
        let mut config = russh::client::Config::default();

        apply_to_config(&mut config, &algorithms);

        assert!(config.preferred.compression.is_empty());
    }
}
