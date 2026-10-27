use std::{
    collections::HashMap,
    net::{IpAddr, Ipv6Addr},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;

const IDLE_TTL: Duration = Duration::from_secs(20 * 60);
const BAN_TIME: Duration = Duration::from_secs(15 * 60);
const PROBE_WINDOW: Duration = Duration::from_secs(5 * 60);

#[derive(Clone, Copy, Debug)]
pub enum Traffic {
    Page,
    Update,
    Download,
    Report,
}

#[derive(Clone, Copy)]
pub struct Policy {
    pub page_burst: u32,
    pub page_per_second: f64,
    pub update_burst: u32,
    pub update_per_second: f64,
    pub download_burst: u32,
    pub download_per_second: f64,
    pub report_burst: u32,
    pub report_per_second: f64,
    pub max_clients: usize,
    pub downloads_per_ip: usize,
    pub downloads_total: usize,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            page_burst: 40,
            page_per_second: 2.0,
            update_burst: 20,
            update_per_second: 1.0,
            download_burst: 8,
            download_per_second: 0.2,
            report_burst: 3,
            report_per_second: 0.1,
            max_clients: 10_000,
            downloads_per_ip: 4,
            downloads_total: 32,
        }
    }
}

#[derive(Debug)]
pub enum Rejection {
    Limited(u64),
    Banned,
    Capacity,
}

struct Bucket {
    tokens: f64,
    updated: Instant,
    burst: f64,
    rate: f64,
}

impl Bucket {
    fn new(burst: u32, rate: f64, now: Instant) -> Self {
        Self {
            tokens: burst.into(),
            updated: now,
            burst: burst.into(),
            rate,
        }
    }

    fn take(&mut self, now: Instant) -> Result<(), u64> {
        self.tokens = (self.tokens
            + now.saturating_duration_since(self.updated).as_secs_f64() * self.rate)
            .min(self.burst);
        self.updated = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            Ok(())
        } else {
            Err(((1.0 - self.tokens) / self.rate).ceil().max(1.0) as u64)
        }
    }
}

struct Client {
    page: Bucket,
    update: Bucket,
    download: Bucket,
    report: Bucket,
    downloads: Arc<Semaphore>,
    last_seen: Instant,
    probes_since: Instant,
    probes: u8,
    banned_until: Option<Instant>,
}

impl Client {
    fn new(policy: Policy, now: Instant) -> Self {
        Self {
            page: Bucket::new(policy.page_burst, policy.page_per_second, now),
            update: Bucket::new(policy.update_burst, policy.update_per_second, now),
            download: Bucket::new(policy.download_burst, policy.download_per_second, now),
            report: Bucket::new(policy.report_burst, policy.report_per_second, now),
            downloads: Arc::new(Semaphore::new(policy.downloads_per_ip)),
            last_seen: now,
            probes_since: now,
            probes: 0,
            banned_until: None,
        }
    }
}

struct Clients {
    entries: HashMap<IpAddr, Client>,
    last_cleanup: Instant,
}

pub struct Guard {
    policy: Policy,
    clients: Mutex<Clients>,
    pub downloads: Arc<Semaphore>,
}

impl Guard {
    pub fn new(policy: Policy) -> Self {
        Self {
            policy,
            clients: Mutex::new(Clients {
                entries: HashMap::new(),
                last_cleanup: Instant::now(),
            }),
            downloads: Arc::new(Semaphore::new(policy.downloads_total)),
        }
    }

    pub fn check(
        &self,
        ip: IpAddr,
        traffic: Traffic,
        probe: bool,
        now: Instant,
    ) -> Result<Arc<Semaphore>, Rejection> {
        let ip = client_key(ip);
        let mut clients = self
            .clients
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if now.saturating_duration_since(clients.last_cleanup) >= Duration::from_secs(60) {
            clients.entries.retain(|_, client| {
                // Keep an entry while its response body still owns a download permit.
                now.saturating_duration_since(client.last_seen) < IDLE_TTL
                    || client.downloads.available_permits() < self.policy.downloads_per_ip
            });
            clients.last_cleanup = now;
        }
        if !clients.entries.contains_key(&ip) && clients.entries.len() >= self.policy.max_clients {
            return Err(Rejection::Capacity);
        }
        let client = clients
            .entries
            .entry(ip)
            .or_insert_with(|| Client::new(self.policy, now));
        client.last_seen = now;
        if client.banned_until.is_some_and(|until| now < until) {
            return Err(Rejection::Banned);
        }
        client.banned_until = None;
        client.page.take(now).map_err(Rejection::Limited)?;
        match traffic {
            Traffic::Page => {}
            Traffic::Update => client.update.take(now).map_err(Rejection::Limited)?,
            Traffic::Download => client.download.take(now).map_err(Rejection::Limited)?,
            Traffic::Report => client.report.take(now).map_err(Rejection::Limited)?,
        }
        if probe {
            if now.saturating_duration_since(client.probes_since) >= PROBE_WINDOW {
                client.probes_since = now;
                client.probes = 0;
            }
            client.probes = client.probes.saturating_add(1);
            if client.probes >= 5 {
                client.banned_until = Some(now + BAN_TIME);
                return Err(Rejection::Banned);
            }
        }
        Ok(Arc::clone(&client.downloads))
    }
}

/// Group IPv6 clients by /64; rotating the interface ID must not reset limits.
pub fn client_key(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(ip) => match ip.to_ipv4_mapped() {
            Some(ip) => IpAddr::V4(ip),
            None => {
                let segments = ip.segments();
                IpAddr::V6(Ipv6Addr::new(
                    segments[0],
                    segments[1],
                    segments[2],
                    segments[3],
                    0,
                    0,
                    0,
                    0,
                ))
            }
        },
        ip => ip,
    }
}

pub fn scanner_agent(agent: &str) -> bool {
    let agent = agent.to_ascii_lowercase();
    [
        "sqlmap",
        "nikto",
        "nuclei",
        "masscan",
        "wpscan",
        "zgrab",
        "dirbuster",
        "acunetix",
        "bytespider",
        "dotbot",
        "mj12bot",
        "blexbot",
    ]
    .iter()
    .any(|name| agent.contains(name))
}

pub fn scanner_path(path: &str) -> bool {
    let path = path.to_ascii_lowercase();
    path.split('/').any(|part| part.starts_with('.'))
        || [
            "/wp-admin",
            "/wp-login",
            "/xmlrpc.php",
            "/phpmyadmin",
            "/actuator",
            "/server-status",
            "/cgi-bin/",
        ]
        .iter()
        .any(|prefix| path.starts_with(prefix))
}
