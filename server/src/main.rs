use opensc_web::{WebState, app, guard::Policy};
use std::{
    collections::HashSet,
    env,
    io::{Read, Write},
    net::{IpAddr, SocketAddr, TcpStream},
    path::PathBuf,
    time::Duration,
};

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let arguments: Vec<String> = env::args().skip(1).collect();
    if arguments.first().is_some_and(|a| a == "--reports") {
        let path = PathBuf::from(env::var_os("OPENSC_REPORT_DB").ok_or("Set OPENSC_REPORT_DB")?);
        let store = opensc_web::reports::Store::open(&path)?;
        match arguments.get(1).map(String::as_str) {
            Some("list") => println!("{}", serde_json::to_string_pretty(&store.list()?)?),
            Some("show") => println!(
                "{}",
                serde_json::to_string_pretty(
                    &store.show(arguments.get(2).ok_or("Specify ticket ID")?)?
                )?
            ),
            Some("status") => store.set_status(
                arguments.get(2).ok_or("Specify ticket ID")?,
                arguments.get(3).ok_or("Specify status")?,
                arguments.get(4).map(String::as_str).unwrap_or(""),
            )?,
            Some("backup") => {
                let destination = PathBuf::from(arguments.get(2).ok_or("Specify new backup path")?);
                if destination.exists() {
                    return Err("Backup destination already exists".into());
                }
                store.backup(&destination)?;
                println!("Backup created");
            }
            _ => {
                return Err(
                    "Usage: --reports list | show ID | status ID STATE [NOTE] | backup PATH".into(),
                );
            }
        }
        return Ok(());
    }
    if arguments.iter().any(|arg| arg == "--healthcheck") {
        let mut connection =
            TcpStream::connect_timeout(&"127.0.0.1:8080".parse()?, Duration::from_secs(2))?;
        connection.set_read_timeout(Some(Duration::from_secs(2)))?;
        connection
            .write_all(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")?;
        let mut response = String::new();
        connection.take(8192).read_to_string(&mut response)?;
        if !response.starts_with("HTTP/1.1 200 ") || !response.ends_with("opensc-ready\n") {
            return Err("Unhealthy HTTP response".into());
        }
        return Ok(());
    }
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(16)
        .enable_all()
        .build()?
        .block_on(serve())
}

async fn serve() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let root = PathBuf::from(env::var("OPENSC_WEB_ROOT").unwrap_or_else(|_| "/srv/shadow".into()))
        .canonicalize()?;
    if !root.is_dir() || !root.join("index.html").is_file() {
        return Err("OPENSC_WEB_ROOT must contain index.html".into());
    }
    let bind: SocketAddr = env::var("OPENSC_WEB_BIND")
        .unwrap_or_else(|_| "0.0.0.0:8080".into())
        .parse()?;
    let host = env::var("OPENSC_TRUSTED_PROXY_HOST").ok();
    let proxies = if let Some(host) = &host {
        resolve(host).await?
    } else {
        HashSet::new()
    };
    let mut state = WebState::new(Policy::default(), proxies, host.is_some());
    if let Some(path) = env::var_os("OPENSC_REPORT_DB") {
        state.reports = Some(std::sync::Arc::new(opensc_web::reports::Store::open(
            &PathBuf::from(path),
        )?));
    }
    if let Some(host) = host {
        let refresh = state.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                match resolve(&host).await {
                    Ok(proxies) => refresh.replace_proxies(proxies),
                    Err(error) => eprintln!("Proxy DNS refresh failed: {error}"),
                }
            }
        });
    }
    let listener = tokio::net::TcpListener::bind(bind).await?;
    eprintln!(
        "OpenSC web {} listening on {bind}; backend=Rust/Axum",
        env!("CARGO_PKG_VERSION")
    );
    axum::serve(
        listener,
        app(root, state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown())
    .await?;
    Ok(())
}

async fn resolve(host: &str) -> Result<HashSet<IpAddr>, Box<dyn std::error::Error + Send + Sync>> {
    let proxies: HashSet<_> = tokio::net::lookup_host((host, 8080))
        .await?
        .map(|address| address.ip())
        .collect();
    if proxies.is_empty() {
        return Err("Trusted proxy DNS returned no addresses".into());
    }
    Ok(proxies)
}

async fn shutdown() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("SIGTERM handler");
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {} }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
