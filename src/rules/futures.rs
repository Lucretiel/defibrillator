use std::{
    net::{Ipv4Addr, SocketAddrV4},
    num::NonZeroU16,
    time::Duration,
};

use bytes::Bytes;
use futures::{future::pending, stream::FuturesUnordered, StreamExt};
use regex::bytes::Regex;
use reqwest::Client;
use tokio::{
    net::TcpStream,
    sync::broadcast::{error::RecvError, Receiver},
    time::{sleep, sleep_until, Instant},
};
use tracing::{debug, debug_span, error, trace, warn, Instrument, Level};

#[derive(Debug)]
pub struct After {
    duration: Duration,
}

impl After {
    pub(super) fn new(duration: Duration) -> Self {
        Self { duration }
    }

    #[tracing::instrument(name = "after", level = Level::DEBUG, skip(self), fields(duration = ?self.duration))]
    pub async fn wait(&self) {
        sleep(self.duration).await;
        debug!("timer completed");
    }
}

#[tracing::instrument(name = "http", level = Level::DEBUG, skip(client))]
async fn http_family_ready(protocol: &str, port: NonZeroU16, client: &Client) {
    let request = client
        .head(format!("{}://127.0.0.1:{}", protocol, port))
        .timeout(Duration::from_secs(60));

    loop {
        // At most 1 attempt per second
        let now = Instant::now();

        trace!("sending request...");
        match request.try_clone().unwrap().send().await {
            // We don't care *what* the response is, only that a response was received
            Ok(..) => {
                debug!("request successful");
                return;
            }
            // Make at most 1 attempt per second.
            Err(..) => sleep_until(now + Duration::from_secs(1)).await,
        }
    }
}

#[derive(Debug)]
pub struct Http<'a> {
    port: NonZeroU16,
    client: &'a Client,
}

impl<'a> Http<'a> {
    pub(super) fn new(port: NonZeroU16, client: &'a Client) -> Self {
        Self { port, client }
    }

    pub async fn wait(self) {
        http_family_ready("http", self.port, self.client).await
    }
}
#[derive(Debug)]
pub struct Https<'a> {
    port: NonZeroU16,
    client: &'a Client,
}

impl<'a> Https<'a> {
    pub(super) fn new(port: NonZeroU16, client: &'a Client) -> Self {
        Self { port, client }
    }

    pub async fn wait(self) {
        http_family_ready("http", self.port, self.client).await
    }
}

#[derive(Debug)]
pub struct Tcp {
    port: NonZeroU16,
}

impl Tcp {
    pub(super) fn new(port: NonZeroU16) -> Self {
        Self { port }
    }

    #[tracing::instrument(name = "tcp", level = Level::DEBUG, skip(self), fields(port = ?self.port))]
    pub async fn wait(self) {
        let localhost = Ipv4Addr::new(127, 0, 0, 1);
        let socket = SocketAddrV4::new(localhost, self.port.get());

        loop {
            let now = Instant::now();

            trace!("connecting...");
            match TcpStream::connect(socket).await {
                Ok(..) => {
                    debug!("connection established");
                    return;
                }
                // Make at most 1 attempt per second.
                Err(..) => sleep_until(now + Duration::from_secs(1)).await,
            }
        }
    }
}

#[derive(Debug)]
pub struct Matches {
    pattern: Regex,
    log_lines: Receiver<Bytes>,
}

impl Matches {
    pub(super) fn new(pattern: Regex, log_lines: Receiver<Bytes>) -> Self {
        Self { pattern, log_lines }
    }

    #[tracing::instrument(name = "matches", skip(self), fields(pattern = %self.pattern))]
    pub async fn wait(mut self) {
        loop {
            match self.log_lines.recv().await {
                Ok(line) => {
                    trace!("testing log line");
                    if self.pattern.is_match(&line) {
                        debug!("log line matched");
                        return;
                    }
                }
                Err(err) => match err {
                    RecvError::Closed => {
                        warn!("log lines channel closed");
                        pending().await
                    }

                    // We panic here because we assuming that the process
                    // won't produce lines faster than we can process them
                    // (especially during startup)
                    RecvError::Lagged(lines) => error!(missed = lines, "log lines channel lagged"),
                },
            }
        }
    }
}

#[derive(Debug)]
pub enum Rule<'a> {
    After(After),
    Http(Http<'a>),
    Https(Https<'a>),
    Tcp(Tcp),
    Matches(Matches),
}

impl Rule<'_> {
    pub async fn wait(self) {
        match self {
            Rule::After(after) => after.wait().await,
            Rule::Http(http) => http.wait().await,
            Rule::Https(https) => https.wait().await,
            Rule::Tcp(tcp) => tcp.wait().await,
            Rule::Matches(matches) => matches.wait().await,
        }
    }
}

#[derive(Debug)]
pub struct AndRules<'a> {
    rules: Vec<Rule<'a>>,
}

impl<'a> AndRules<'a> {
    pub(super) fn new(rules: Vec<Rule<'a>>) -> Self {
        Self { rules }
    }

    pub async fn wait(mut self) {
        if self.rules.len() == 1 {
            self.rules.pop().unwrap().wait().await
        } else {
            let futures: FuturesUnordered<_> = self
                .rules
                .into_iter()
                .enumerate()
                .map(|(id, rule)| rule.wait().instrument(debug_span!("rule", id)))
                .collect();

            futures.collect().instrument(debug_span!("rules")).await
        }
    }
}

#[derive(Debug)]
pub struct OrRules<'a> {
    rules: Vec<AndRules<'a>>,
}

impl<'a> OrRules<'a> {
    pub(super) fn new(rules: Vec<AndRules<'a>>) -> Self {
        Self { rules }
    }
    pub async fn wait(mut self) {
        if self.rules.len() == 1 {
            self.rules.pop().unwrap().wait().await
        } else {
            let mut futures: FuturesUnordered<_> = self
                .rules
                .into_iter()
                .enumerate()
                .map(|(id, rule)| rule.wait().instrument(debug_span!("rule group", id)))
                .collect();

            let _ = futures.next().instrument(debug_span!("rule groups")).await;
        }
    }
}
