use std::{num::NonZeroU16, time::Duration};

use bytes::Bytes;
use regex::bytes::Regex;
use reqwest::Client;
use tokio::sync::broadcast::{Receiver, Sender};

use super::futures as rule_futures;

#[derive(Debug, Clone, Copy)]
pub struct After {
    duration: Duration,
}

impl After {
    pub fn new(duration: Duration) -> Self {
        Self { duration }
    }

    pub fn build(&self) -> rule_futures::After {
        rule_futures::After::new(self.duration)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Http {
    port: Option<NonZeroU16>,
}

impl Http {
    pub fn new(port: Option<NonZeroU16>) -> Self {
        Self { port }
    }

    pub fn build<'a>(&self, client: &'a Client) -> rule_futures::Http<'a> {
        rule_futures::Http::new(self.port.or_else(|| NonZeroU16::new(80)).unwrap(), client)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Https {
    port: Option<NonZeroU16>,
}

impl Https {
    pub fn new(port: Option<NonZeroU16>) -> Self {
        Self { port }
    }

    pub fn build<'a>(&self, client: &'a Client) -> rule_futures::Https<'a> {
        rule_futures::Https::new(self.port.or_else(|| NonZeroU16::new(443)).unwrap(), client)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Tcp {
    port: NonZeroU16,
}

impl Tcp {
    pub fn new(port: NonZeroU16) -> Self {
        Self { port }
    }

    pub fn build(&self) -> rule_futures::Tcp {
        rule_futures::Tcp::new(self.port)
    }
}

#[derive(Debug, Clone)]
pub struct Matches {
    pattern: Regex,
}

impl Matches {
    pub fn new(pattern: Regex) -> Self {
        Self { pattern }
    }

    pub fn build(&self, log_lines: Receiver<Bytes>) -> rule_futures::Matches {
        rule_futures::Matches::new(self.pattern.clone(), log_lines)
    }
}

#[derive(Debug, Clone)]
pub enum Rule {
    After(After),
    Tcp(Tcp),
    Http(Http),
    Https(Https),
    Matches(Matches),
}

impl Rule {
    pub fn build<'a>(
        &self,
        client: &'a Client,
        log_lines: &Sender<Bytes>,
    ) -> rule_futures::Rule<'a> {
        match self {
            Rule::After(after) => rule_futures::Rule::After(after.build()),
            Rule::Tcp(tcp) => rule_futures::Rule::Tcp(tcp.build()),
            Rule::Http(http) => rule_futures::Rule::Http(http.build(client)),
            Rule::Https(https) => rule_futures::Rule::Https(https.build(client)),
            Rule::Matches(matches) => {
                rule_futures::Rule::Matches(matches.build(log_lines.subscribe()))
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct AndRules {
    rules: Vec<Rule>,
}

impl AndRules {
    pub fn new(rules: Vec<Rule>) -> Self {
        Self { rules }
    }

    pub fn build<'a>(
        &self,
        client: &'a Client,
        log_lines: &Sender<Bytes>,
    ) -> rule_futures::AndRules<'a> {
        rule_futures::AndRules::new(
            self.rules
                .iter()
                .map(|rule| rule.build(client, log_lines))
                .collect(),
        )
    }
}

#[derive(Debug, Clone)]
pub struct OrRules {
    rules: Vec<AndRules>,
}

impl OrRules {
    pub fn new(rules: Vec<AndRules>) -> Self {
        Self { rules }
    }

    pub fn build<'a>(
        &self,
        client: &'a Client,
        log_lines: &Sender<Bytes>,
    ) -> rule_futures::OrRules<'a> {
        rule_futures::OrRules::new(
            self.rules
                .iter()
                .map(|rule| rule.build(client, log_lines))
                .collect(),
        )
    }
}
