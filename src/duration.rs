use std::{str::FromStr, time::Duration as StdDuration};

use nom::{
    branch::alt,
    character::complete::{digit0, space0},
    error::ParseError,
    IResult, Parser,
};
use nom_supreme::{
    error::ErrorTree,
    final_parser::{final_parser, Location},
    parser_ext::ParserExt,
    tag::{complete::tag_no_case, TagError},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Duration {
    inner: std::time::Duration,
}

impl Duration {
    pub fn get(&self) -> StdDuration {
        self.inner
    }
}

impl From<StdDuration> for Duration {
    fn from(duration: StdDuration) -> Self {
        Self { inner: duration }
    }
}

fn three_tags<'i, E: ParseError<&'i str> + TagError<&'i str, &'static str>>(
    short: &'static str,
    singular: &'static str,
    plural: &'static str,
) -> impl Parser<&'i str, &'i str, E> {
    alt((
        tag_no_case(plural),
        tag_no_case(singular),
        tag_no_case(short),
    ))
}

fn parse_duration_suffix(input: &str) -> IResult<&str, StdDuration, ErrorTree<&str>> {
    alt((
        three_tags("s", "second", "seconds").value(StdDuration::from_secs(1)),
        three_tags("ms", "millisecond", "milliseconds").value(StdDuration::from_millis(1)),
        three_tags("μs", "microsecond", "microseconds").value(StdDuration::from_micros(1)),
        three_tags("m", "minute", "minutes").value(StdDuration::from_secs(60)),
    ))
    .parse(input)
}

pub fn parse_duration(input: &str) -> IResult<&str, StdDuration, ErrorTree<&str>> {
    digit0
        .parse_from_str()
        .terminated(space0)
        .and(parse_duration_suffix.context("duration suffix"))
        .map(|(count, units): (u32, StdDuration)| units * count)
        .parse(input)
}

impl FromStr for Duration {
    type Err = ErrorTree<Location>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        final_parser(
            parse_duration
                .context("duration")
                .map(|duration| Duration { inner: duration }),
        )(s)
    }
}
