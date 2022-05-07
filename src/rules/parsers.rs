use std::{num::NonZeroU16, str::FromStr};

use nom::{
    self,
    branch::alt,
    bytes::complete::{escaped_transform, take_till1},
    character::complete::{char, digit1, space0, space1},
    combinator::eof,
    IResult, Parser,
};
use nom_supreme::{
    error::ErrorTree,
    final_parser::{final_parser, Location},
    multi::collect_separated_terminated,
    parser_ext::ParserExt,
    tag::complete::tag_no_case,
};
use regex::bytes::Regex;

use crate::duration::parse_duration;

use super::descriptors::{After, AndRules, Http, Https, Matches, OrRules, Rule, Tcp};

fn parse_after(input: &str) -> IResult<&str, After, ErrorTree<&str>> {
    tag_no_case("after")
        .terminated(space1.cut())
        .precedes(parse_duration.cut())
        .map(After::new)
        .parse(input)
}

fn parse_port(input: &str) -> IResult<&str, NonZeroU16, ErrorTree<&str>> {
    tag_no_case("port")
        .terminated(space1)
        .precedes(digit1)
        .parse_from_str()
        .parse(input)
}

trait FromMaybePort: Sized {
    fn from_maybe_port(port: Option<NonZeroU16>) -> Self;
}

impl FromMaybePort for Http {
    fn from_maybe_port(port: Option<NonZeroU16>) -> Self {
        Self::new(port)
    }
}

impl FromMaybePort for Https {
    fn from_maybe_port(port: Option<NonZeroU16>) -> Self {
        Self::new(port)
    }
}

fn parse_http_family<'i, T: FromMaybePort>(
    protocol: &'static str,
) -> impl Parser<&'i str, T, ErrorTree<&'i str>> {
    alt((
        parse_port
            .map(Some)
            .terminated(space1)
            .terminated(tag_no_case("ready")),
        tag_no_case("ready").value(None),
    ))
    .map(T::from_maybe_port)
    .cut()
    .preceded_by(tag_no_case(protocol).terminated(space1))
}

fn parse_http(input: &str) -> IResult<&str, Http, ErrorTree<&str>> {
    parse_http_family("http").parse(input)
}

fn parse_https(input: &str) -> IResult<&str, Https, ErrorTree<&str>> {
    parse_http_family("https").parse(input)
}

fn parse_tcp(input: &str) -> IResult<&str, Tcp, ErrorTree<&str>> {
    tag_no_case("tcp")
        .terminated(space1.cut())
        .precedes(parse_port.cut())
        .terminated(space1.cut())
        .terminated(tag_no_case("ready").cut())
        .map(Tcp::new)
        .parse(input)
}

fn parse_quoted_pattern(input: &str) -> IResult<&str, Regex, ErrorTree<&str>> {
    escaped_transform(
        take_till1(|c| c == '"' || c == '\\'),
        '\\',
        char('"').value('"'),
    )
    .map_res(|s| Regex::new(&s))
    .delimited_by(char('"'))
    .parse(input)
}

fn parse_raw_pattern(input: &str) -> IResult<&str, Regex, ErrorTree<&str>> {
    take_till1(|c: char| c.is_whitespace())
        .map_res(Regex::new)
        .parse(input)
}

fn parse_matches(input: &str) -> IResult<&str, Matches, ErrorTree<&str>> {
    tag_no_case("matches")
        .terminated(space1.cut())
        .precedes(alt((parse_quoted_pattern, parse_raw_pattern)).cut())
        .map(Matches::new)
        .parse(input)
}

fn parse_rule(input: &str) -> IResult<&str, Rule, ErrorTree<&str>> {
    alt((
        parse_after.map(Rule::After).context("after"),
        parse_tcp.map(Rule::Tcp).context("tcp"),
        parse_http.map(Rule::Http).context("http"),
        parse_https.map(Rule::Https).context("https"),
        parse_matches.map(Rule::Matches).context("matches"),
    ))
    .parse(input)
}

fn parse_and_rules(input: &str) -> IResult<&str, AndRules, ErrorTree<&str>> {
    collect_separated_terminated(
        parse_rule.context("rule"),
        tag_no_case("and").delimited_by(space1),
        eof.preceded_by(space0)
            .or(tag_no_case("or").preceded_by(space1).peek()),
    )
    .map(AndRules::new)
    .parse(input)
}

fn parse_or_rules(input: &str) -> IResult<&str, OrRules, ErrorTree<&str>> {
    collect_separated_terminated(
        parse_and_rules.context("rule group"),
        tag_no_case("or").delimited_by(space1),
        eof.preceded_by(space0),
    )
    .map(OrRules::new)
    .parse(input)
}

impl FromStr for OrRules {
    type Err = ErrorTree<Location>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        final_parser(parse_or_rules)(s)
    }
}
