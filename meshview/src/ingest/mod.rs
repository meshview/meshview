// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! Reading the mesh from an MQTT broker into the database.
//!
//! Two tasks: one keeps the connection to the broker and does nothing but take
//! messages off it, and the other opens and stores them. Kept apart so that a
//! slow write — the database busy with a cleanup, say — never stops the
//! connection from answering the broker's keep-alives, which would cost a
//! reconnection and every message sent in the meantime.

pub mod keys;
pub mod opened;
pub mod store;

use crate::config::Config;
use crate::state::{CHECK_MQTT, Readiness};
use anyhow::{Context, Result, bail};
use keys::Keyring;
use opened::{NotOpened, Opened};
use rumqttc::{
    AsyncClient, Event, MqttOptions, Outgoing, Packet, QoS, TlsConfiguration, Transport,
};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::{ClientConfig, RootCertStore};
use std::collections::HashSet;
use std::hash::{BuildHasher, RandomState};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use store::Store;
use tokio::sync::{mpsc, watch};
use tracing::{debug, error, info, warn};

/// MQTT's own port, and its port over TLS.
const DEFAULT_PORT: u16 = 1883;
const DEFAULT_TLS_PORT: u16 = 8883;

/// How long the broker gets to hear that this client is leaving.
const DISCONNECT_GRACE: Duration = Duration::from_secs(1);

/// How long to wait before connecting again after losing the broker.
const RECONNECT_DELAY: Duration = Duration::from_secs(1);

/// What paho, under the Python version, asks the broker for.
const KEEP_ALIVE: Duration = Duration::from_secs(60);

/// The largest message taken off the connection. Far beyond any real packet,
/// which the store refuses past a kilobyte anyway, but a message over this
/// breaks the connection rather than being skipped, so it has to be generous:
/// an oversized retained message would otherwise break every reconnection too.
const MAX_INCOMING: usize = 1024 * 1024;

/// Messages waiting to be stored before new ones are dropped. At the few a
/// second a busy mesh sends, minutes' worth.
const BACKLOG: usize = 10_000;

/// How often the count of messages processed is logged, as in the Python
/// version.
const REPORT_EVERY: u64 = 10_000;

/// The `[mqtt]` section.
#[derive(Debug, Clone)]
pub struct Settings {
    pub server: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    pub topics: Vec<String>,
    pub skip_node_ids: HashSet<u32>,
    pub secondary_keys: Vec<String>,
    pub tls: Option<Tls>,
}

/// TLS to the broker, which the Python version never had.
#[derive(Debug, Clone, Default)]
pub struct Tls {
    /// Trust only the certificates in this PEM file instead of the public roots,
    /// for a broker with a certificate of its own making.
    pub ca_file: Option<PathBuf>,
    /// A client certificate and its key, both PEM, for a broker that asks for
    /// one.
    pub client_cert: Option<PathBuf>,
    pub client_key: Option<PathBuf>,
}

impl Settings {
    pub fn from_config(config: &Config) -> Result<Self> {
        let mqtt = &config.mqtt;
        let text = |value: &str| {
            Some(value.trim())
                .filter(|v| !v.is_empty())
                .map(str::to_string)
        };
        let path = |value: &str| text(value).map(PathBuf::from);

        let Some(server) = text(&mqtt.server) else {
            bail!("[mqtt] server is required to ingest");
        };
        if mqtt.topics.is_empty() {
            bail!("[mqtt] topics is empty: there would be nothing to ingest");
        }

        let tls = if mqtt.tls {
            let tls = Tls {
                ca_file: path(&mqtt.tls_ca_file),
                client_cert: path(&mqtt.tls_client_cert),
                client_key: path(&mqtt.tls_client_key),
            };
            if tls.client_cert.is_some() != tls.client_key.is_some() {
                bail!("[mqtt] tls_client_cert and tls_client_key go together");
            }
            Some(tls)
        } else {
            None
        };

        let port = match mqtt.port {
            Some(port) => port,
            None if tls.is_some() => DEFAULT_TLS_PORT,
            None => DEFAULT_PORT,
        };

        Ok(Self {
            server,
            port,
            username: text(&mqtt.username),
            password: text(&mqtt.password),
            topics: mqtt.topics.clone(),
            skip_node_ids: mqtt.skip_node_ids.iter().copied().collect(),
            secondary_keys: mqtt.secondary_keys.clone(),
            tls,
        })
    }

    fn options(&self) -> Result<MqttOptions> {
        // A client id of its own every start, as the Python version: two
        // ingesters with the same one would keep throwing each other off.
        let id = RandomState::new().hash_one(Instant::now()) as u16;
        let mut options = MqttOptions::new(format!("meshview-{id}"), &self.server, self.port);
        options
            .set_keep_alive(KEEP_ALIVE)
            .set_clean_session(true)
            .set_max_packet_size(MAX_INCOMING, MAX_INCOMING);

        if let Some(username) = &self.username {
            options.set_credentials(username, self.password.as_deref().unwrap_or(""));
        }
        if let Some(tls) = &self.tls {
            options.set_transport(Transport::tls_with_config(TlsConfiguration::Rustls(
                Arc::new(tls.client_config()?),
            )));
        }
        Ok(options)
    }
}

impl Tls {
    fn client_config(&self) -> Result<ClientConfig> {
        let mut roots = RootCertStore::empty();
        match &self.ca_file {
            Some(file) => {
                for cert in CertificateDer::pem_file_iter(file)
                    .with_context(|| format!("reading {}", file.display()))?
                {
                    roots
                        .add(cert.with_context(|| format!("reading {}", file.display()))?)
                        .with_context(|| format!("a certificate in {}", file.display()))?;
                }
                if roots.is_empty() {
                    bail!("{} has no certificates in it", file.display());
                }
            }
            None => roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned()),
        }

        let builder =
            ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .context("setting up TLS")?
                .with_root_certificates(roots);

        let config = match (&self.client_cert, &self.client_key) {
            (Some(cert), Some(key)) => {
                let chain = CertificateDer::pem_file_iter(cert)
                    .and_then(Iterator::collect::<Result<Vec<_>, _>>)
                    .with_context(|| format!("reading {}", cert.display()))?;
                let key = PrivateKeyDer::from_pem_file(key)
                    .with_context(|| format!("reading {}", key.display()))?;
                builder
                    .with_client_auth_cert(chain, key)
                    .context("the client certificate and key")?
            }
            _ => builder.with_no_client_auth(),
        };
        Ok(config)
    }
}

/// Ingests until told to stop.
pub async fn run(
    settings: Settings,
    store: Store,
    readiness: Arc<Readiness>,
    mut stopping: watch::Receiver<bool>,
) -> Result<()> {
    let keyring = Keyring::new(&settings.secondary_keys);
    info!(
        "{} decryption key(s): the default and {} secondary",
        keyring.len(),
        keyring.len() - 1
    );

    let (client, mut events) = AsyncClient::new(settings.options()?, 64);
    let (queue, backlog) = mpsc::channel(BACKLOG);
    let mut writer = tokio::spawn(store_messages(
        backlog,
        keyring,
        settings.skip_node_ids.clone(),
        store,
    ));

    info!(
        "Starting MQTT ingestion from {}:{}{}",
        settings.server,
        settings.port,
        if settings.tls.is_some() {
            " over TLS"
        } else {
            ""
        }
    );

    let mut dropped: u64 = 0;
    loop {
        let event = tokio::select! {
            event = events.poll() => event,
            _ = stopping.wait_for(|stop| *stop) => break,
        };

        match event {
            Ok(Event::Incoming(Packet::ConnAck(_))) => {
                info!(
                    "Connected to MQTT broker at {}:{}",
                    settings.server, settings.port
                );
                // A clean session forgets what it subscribed to, so every
                // connection subscribes again.
                for topic in &settings.topics {
                    info!("Subscribing to: {topic}");
                    if let Err(e) = client.try_subscribe(topic.as_str(), QoS::AtMostOnce) {
                        error!("could not subscribe to {topic}: {e}");
                    }
                }
                readiness.set(CHECK_MQTT, true);
            }
            Ok(Event::Incoming(Packet::Publish(publish))) => {
                if queue.try_send((publish.topic, publish.payload)).is_err() {
                    dropped += 1;
                    if dropped.is_power_of_two() {
                        warn!("the database is not keeping up: {dropped} messages dropped so far");
                    }
                }
            }
            Ok(_) => {}
            Err(e) => {
                readiness.set(CHECK_MQTT, false);
                error!("MQTT error: {e}, reconnecting in 1s...");
                tokio::time::sleep(RECONNECT_DELAY).await;
            }
        }
    }

    // Tell the broker, which takes driving the connection a little longer: a
    // disconnect that is only queued is never sent.
    readiness.set(CHECK_MQTT, false);
    if client.try_disconnect().is_ok() {
        let _ = tokio::time::timeout(DISCONNECT_GRACE, async {
            loop {
                match events.poll().await {
                    Ok(Event::Outgoing(Outgoing::Disconnect)) | Err(_) => break,
                    Ok(_) => {}
                }
            }
        })
        .await;
    }

    // What has arrived is stored, as long as that does not take longer than a
    // container runtime would wait.
    drop(queue);
    match tokio::time::timeout(crate::app::DRAIN_GRACE, &mut writer).await {
        Ok(result) => result.context("the storing task")?,
        Err(_) => {
            writer.abort();
            warn!(
                "still storing after {:?}; the messages left are lost",
                crate::app::DRAIN_GRACE
            );
        }
    }
    Ok(())
}

/// Opens and stores what the connection hands over, one message at a time.
async fn store_messages(
    mut backlog: mpsc::Receiver<(String, bytes::Bytes)>,
    keyring: Keyring,
    skip: HashSet<u32>,
    mut store: Store,
) {
    let started = Instant::now();
    let mut processed: u64 = 0;
    let mut confirmed = HashSet::new();

    while let Some((topic, payload)) = backlog.recv().await {
        let opened = match Opened::open(&payload, &keyring) {
            Ok(opened) => opened,
            Err(NotOpened::NotAnEnvelope) => continue,
            Err(NotOpened::NoKey { id, channel }) => {
                debug!(
                    "Unable to decrypt MQTT packet id={id} channel={channel} topic={topic:?} \
                     with configured keys"
                );
                continue;
            }
        };
        let key = opened.key;
        if opened.was_encrypted && key > 0 && confirmed.insert(key) {
            info!("Successfully decrypted MQTT packet using secondary key #{key}");
        }

        let packet = opened.packet();
        if skip.contains(&packet.from) {
            continue;
        }

        processed += 1;
        if processed.is_multiple_of(REPORT_EVERY) {
            let rate = processed as f64 / started.elapsed().as_secs_f64().max(f64::EPSILON);
            info!("Processed {processed} messages so far... ({rate:.2} msg/sec)");
        }

        // One bad or unexpected message must not stop ingestion for everyone.
        if let Err(e) = store.process(&topic, &opened).await {
            error!("Failed to store a message from topic {topic:?}; skipping it: {e:#}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sample_mqtt_section_is_enough() {
        let config = Config::parse(include_str!("../../../sample.config.toml")).unwrap();
        let settings = Settings::from_config(&config).unwrap();

        assert_eq!(settings.port, 1883);
        assert_eq!(settings.topics.len(), 3);
        assert_eq!(settings.username.as_deref(), Some("meshdev"));
        assert!(settings.tls.is_none());
        assert!(settings.skip_node_ids.is_empty());
    }

    #[test]
    fn tls_moves_the_default_port_and_builds_a_client() {
        let config =
            Config::parse("[mqtt]\nserver = \"broker\"\ntopics = [\"msh/#\"]\ntls = true\n")
                .unwrap();
        let settings = Settings::from_config(&config).unwrap();

        assert_eq!(settings.port, DEFAULT_TLS_PORT);
        assert!(settings.options().is_ok());
    }

    #[test]
    fn a_client_certificate_needs_its_key() {
        let config = Config::parse(
            "[mqtt]\nserver = \"broker\"\ntopics = [\"msh/#\"]\ntls = true\n\
             tls_client_cert = \"c.pem\"\n",
        )
        .unwrap();
        assert!(Settings::from_config(&config).is_err());
    }

    #[test]
    fn ingesting_needs_a_server_and_topics() {
        let no_topics = Config::parse("[mqtt]\nserver = \"broker\"\n").unwrap();
        assert!(Settings::from_config(&no_topics).is_err());
        let no_server = Config::parse("[mqtt]\ntopics = [\"msh/#\"]\n").unwrap();
        assert!(Settings::from_config(&no_server).is_err());
    }
}
