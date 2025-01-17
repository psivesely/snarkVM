// Copyright (C) 2019-2022 Aleo Systems Inc.
// This file is part of the snarkVM library.

// The snarkVM library is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// The snarkVM library is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with the snarkVM library. If not, see <https://www.gnu.org/licenses/>.

use crate::ParserResult;

use nom::{
    branch::alt,
    bytes::complete::tag,
    character::complete::{anychar, char, line_ending, multispace1},
    combinator::{cut, map, recognize, value, verify},
    error::{ErrorKind, VerboseError, VerboseErrorKind},
    multi::fold_many0,
    sequence::{preceded, terminated},
};

pub struct Sanitizer;

impl Sanitizer {
    /// Removes all leading whitespaces and comments from the given input, returning the sanitized input.
    pub fn parse(string: &str) -> ParserResult<&str> {
        preceded(Self::parse_whitespaces, Self::parse_comments)(string)
    }

    /// Removes leading whitespaces from the given input.
    pub fn parse_whitespaces(string: &str) -> ParserResult<&str> {
        recognize(Self::many0_(alt((multispace1, tag("\\\n")))))(string)
    }

    /// Removes multiple leading comments from the given input.
    pub fn parse_comments(string: &str) -> ParserResult<&str> {
        recognize(Self::many0_(terminated(Self::parse_comment, Self::parse_whitespaces)))(string)
    }

    /// Removes the first leading comment from the given input.
    pub fn parse_comment(string: &str) -> ParserResult<&str> {
        preceded(
            char('/'),
            alt((preceded(char('/'), cut(Self::str_till_eol)), preceded(char('*'), cut(Self::str_till_star_slash)))),
        )(string)
    }

    /// Parse a safe character (in the sense explained below).
    /// Returns an error if no character is found or a non-safe character is found.
    /// The character is returned, along with the remaining input.
    ///
    /// This is used for otherwise unconstrained characters
    /// in (line and block) comments and in string literals.
    ///
    /// We regard the following characters as safe:
    /// - Horizontal tab (code 9).
    /// - Line feed (code 10).
    /// - Carriage return (code 13).
    /// - Space (code 32).
    /// - Visible ASCII (codes 33-126).
    /// - Non-ASCII Unicode scalar values except bidi embeddings, overrides, and isolates.
    ///
    /// The Unicode bidi characters are well-known for presenting Trojan Source dangers.
    /// The ASCII backspace (code 8) can be also used to make text look different from what it is,
    /// and a similar danger may apply to delete (126).
    /// Other ASCII control characters
    /// (except for horizontal tab, space, line feed, and carriage return, which are allowed)
    /// may or may not present dangers, but we see no good reason for allowing them.
    /// At some point we may want disallow additional non-ASCII characters,
    /// if we see no good reason to allow them.
    ///
    /// Note that we say 'Unicode scalar values' above,
    /// because we read UTF-8-decoded characters,
    /// and thus we will never encounter surrogate code points,
    /// and we do not need to explicitly exclude them in this function.
    ///
    /// Note also that the `nom` documentation for `anychar` says that
    /// it matches one byte as a character.
    /// However, simple experiments show that it matches a Unicode character,
    /// e.g. attempting to parse `"\u{4141}"` yields one CJK character and exhausts the input,
    /// as opposed to returning `A` and leaving another `A` in the input.
    pub fn parse_safe_char(string: &str) -> ParserResult<char> {
        fn is_safe(ch: &char) -> bool {
            let code = *ch as u32;
            code == 9
                || code == 10
                || code == 13
                || (32..=126).contains(&code)
                || (128..=0x2029).contains(&code)
                || (0x202f..=0x2065).contains(&code)
                || (0x206a <= code)
        }
        verify(anychar, is_safe)(string)
    }
}

impl Sanitizer {
    /// End-of-input parser.
    ///
    /// Yields `()` if the parser is at the end of the input; an error otherwise.
    fn eoi(string: &str) -> ParserResult<()> {
        match string.is_empty() {
            true => Ok((string, ())),
            false => {
                Err(nom::Err::Error(VerboseError { errors: vec![(string, VerboseErrorKind::Nom(ErrorKind::Eof))] }))
            }
        }
    }

    /// A newline parser that accepts:
    ///
    /// - A newline.
    /// - The end of input.
    fn eol(string: &str) -> ParserResult<()> {
        alt((
            Self::eoi, // this one goes first because it’s very cheap
            value((), line_ending),
        ))(string)
    }

    /// Apply the `f` parser until `g` succeeds. Both parsers consume the input.
    fn till<'a, A, B, F, G>(mut f: F, mut g: G) -> impl FnMut(&'a str) -> ParserResult<'a, ()>
    where
        F: FnMut(&'a str) -> ParserResult<'a, A>,
        G: FnMut(&'a str) -> ParserResult<'a, B>,
    {
        move |mut i| loop {
            if let Ok((i2, _)) = g(i) {
                break Ok((i2, ()));
            }

            let (i2, _) = f(i)?;
            i = i2;
        }
    }

    /// Parse a string until the end of line.
    ///
    /// This parser accepts the multiline annotation (\) to break the string on several lines.
    ///
    /// Discard any leading newline.
    fn str_till_eol(string: &str) -> ParserResult<&str> {
        map(
            recognize(Self::till(alt((value((), tag("\\\n")), value((), Sanitizer::parse_safe_char))), Self::eol)),
            |i| {
                if i.as_bytes().last() == Some(&b'\n') { &i[0..i.len() - 1] } else { i }
            },
        )(string)
    }

    /// Parse a string until `*/` is encountered.
    ///
    /// This is used to parse the body of a block comment, after the opening `/*`.
    ///
    /// Return the body of the comment, i.e. what is between `/*` and `*/`.
    fn str_till_star_slash(string: &str) -> ParserResult<&str> {
        map(recognize(Self::till(value((), Sanitizer::parse_safe_char), tag("*/"))), |i| {
            &i[0..i.len() - 2] // subtract 2 to discard the closing `*/`
        })(string)
    }

    /// A version of many0 that discards the result of the parser, preventing allocating.
    fn many0_<'a, A, F>(mut f: F) -> impl FnMut(&'a str) -> ParserResult<'a, ()>
    where
        F: FnMut(&'a str) -> ParserResult<'a, A>,
    {
        move |string| fold_many0(&mut f, || (), |_, _| ())(string)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_safe_char() {
        // test correct acceptance of ASCII and non-ASCII:
        assert_eq!(("", 'A'), Sanitizer::parse_safe_char("A").unwrap());
        assert_eq!((" and more", 'A'), Sanitizer::parse_safe_char("A and more").unwrap());
        assert_eq!(("", '\u{4141}'), Sanitizer::parse_safe_char("\u{4141}").unwrap());
        assert_eq!((" and more", '\u{4141}'), Sanitizer::parse_safe_char("\u{4141} and more").unwrap());

        // test rejection and acceptance of ASCII control characters:
        assert!(Sanitizer::parse_safe_char("\x00").is_err());
        assert!(Sanitizer::parse_safe_char("\x01").is_err());
        assert!(Sanitizer::parse_safe_char("\x02").is_err());
        assert!(Sanitizer::parse_safe_char("\x03").is_err());
        assert!(Sanitizer::parse_safe_char("\x04").is_err());
        assert!(Sanitizer::parse_safe_char("\x05").is_err());
        assert!(Sanitizer::parse_safe_char("\x06").is_err());
        assert!(Sanitizer::parse_safe_char("\x07").is_err());
        assert!(Sanitizer::parse_safe_char("\x08").is_err());
        assert!(Sanitizer::parse_safe_char("\x09").is_ok());
        assert!(Sanitizer::parse_safe_char("\x0a").is_ok());
        assert!(Sanitizer::parse_safe_char("\x0b").is_err());
        assert!(Sanitizer::parse_safe_char("\x0c").is_err());
        assert!(Sanitizer::parse_safe_char("\x0d").is_ok());
        assert!(Sanitizer::parse_safe_char("\x0e").is_err());
        assert!(Sanitizer::parse_safe_char("\x0f").is_err());
        assert!(Sanitizer::parse_safe_char("\x10").is_err());
        assert!(Sanitizer::parse_safe_char("\x11").is_err());
        assert!(Sanitizer::parse_safe_char("\x12").is_err());
        assert!(Sanitizer::parse_safe_char("\x13").is_err());
        assert!(Sanitizer::parse_safe_char("\x14").is_err());
        assert!(Sanitizer::parse_safe_char("\x15").is_err());
        assert!(Sanitizer::parse_safe_char("\x16").is_err());
        assert!(Sanitizer::parse_safe_char("\x17").is_err());
        assert!(Sanitizer::parse_safe_char("\x18").is_err());
        assert!(Sanitizer::parse_safe_char("\x19").is_err());
        assert!(Sanitizer::parse_safe_char("\x1a").is_err());
        assert!(Sanitizer::parse_safe_char("\x1b").is_err());
        assert!(Sanitizer::parse_safe_char("\x1c").is_err());
        assert!(Sanitizer::parse_safe_char("\x1d").is_err());
        assert!(Sanitizer::parse_safe_char("\x1e").is_err());
        assert!(Sanitizer::parse_safe_char("\x1f").is_err());
        assert!(Sanitizer::parse_safe_char("\x7f").is_err());

        // test rejection of bidi characters, and acceptance of the ones just above/below:
        assert!(Sanitizer::parse_safe_char("\u{2029}").is_ok());
        assert!(Sanitizer::parse_safe_char("\u{202a}").is_err());
        assert!(Sanitizer::parse_safe_char("\u{202b}").is_err());
        assert!(Sanitizer::parse_safe_char("\u{202c}").is_err());
        assert!(Sanitizer::parse_safe_char("\u{202d}").is_err());
        assert!(Sanitizer::parse_safe_char("\u{202e}").is_err());
        assert!(Sanitizer::parse_safe_char("\u{202f}").is_ok());
        assert!(Sanitizer::parse_safe_char("\u{2065}").is_ok());
        assert!(Sanitizer::parse_safe_char("\u{2066}").is_err());
        assert!(Sanitizer::parse_safe_char("\u{2067}").is_err());
        assert!(Sanitizer::parse_safe_char("\u{2068}").is_err());
        assert!(Sanitizer::parse_safe_char("\u{2069}").is_err());
        assert!(Sanitizer::parse_safe_char("\u{206a}").is_ok());
    }

    #[test]
    fn test_sanitize() {
        // Whitespaces
        assert_eq!(("hello world", ""), Sanitizer::parse("hello world").unwrap());
        assert_eq!(("hello world", ""), Sanitizer::parse(" hello world").unwrap());
        assert_eq!(("hello world", ""), Sanitizer::parse("  hello world").unwrap());
        assert_eq!(("hello world", ""), Sanitizer::parse("\nhello world").unwrap());
        assert_eq!(("hello world", ""), Sanitizer::parse(" \nhello world").unwrap());
        assert_eq!(("hello world ", ""), Sanitizer::parse("hello world ").unwrap());

        // Comments
        assert_eq!(("hello world", "// hello\n"), Sanitizer::parse("// hello\nhello world").unwrap());
        assert_eq!(("hello world", "/* hello */"), Sanitizer::parse("/* hello */hello world").unwrap());
        assert_eq!(("hello world", "/* hello */\n"), Sanitizer::parse("/* hello */\nhello world").unwrap());
        assert_eq!(("hello world", "/** hello */"), Sanitizer::parse("/** hello */hello world").unwrap());
        assert_eq!(("hello world", "/** hello */\n"), Sanitizer::parse("/** hello */\nhello world").unwrap());
        assert_eq!(("/\nhello world", ""), Sanitizer::parse("/\nhello world").unwrap());

        // Whitespaces and comments
        assert_eq!(("hello world", "// hello\n"), Sanitizer::parse(" \n// hello\nhello world").unwrap());
        assert_eq!(("hello world", "/* hello */\n"), Sanitizer::parse(" \n /* hello */\nhello world").unwrap());
        assert_eq!(("hello world", "/** hello */\n"), Sanitizer::parse(" \n\t  /** hello */\nhello world").unwrap());
        assert_eq!(("/\nhello world", ""), Sanitizer::parse(" /\nhello world").unwrap());
    }

    #[test]
    fn test_whitespaces() {
        assert_eq!(("hello world", ""), Sanitizer::parse_whitespaces("hello world").unwrap());
        assert_eq!(("hello world", " "), Sanitizer::parse_whitespaces(" hello world").unwrap());
        assert_eq!(("hello world", "  "), Sanitizer::parse_whitespaces("  hello world").unwrap());
        assert_eq!(("hello world", "\n"), Sanitizer::parse_whitespaces("\nhello world").unwrap());
        assert_eq!(("hello world", " \n"), Sanitizer::parse_whitespaces(" \nhello world").unwrap());
        assert_eq!(("hello world", "\t"), Sanitizer::parse_whitespaces("\thello world").unwrap());
        assert_eq!(("hello world", " \t"), Sanitizer::parse_whitespaces(" \thello world").unwrap());
        assert_eq!(("hello world", " \n\t"), Sanitizer::parse_whitespaces(" \n\thello world").unwrap());
        assert_eq!(("hello world ", ""), Sanitizer::parse_whitespaces("hello world ").unwrap());
    }

    #[test]
    fn test_comments() {
        assert_eq!(("hello world", "// hello\n"), Sanitizer::parse_comments("// hello\nhello world").unwrap());
        assert_eq!(("hello world", "/* hello */\n"), Sanitizer::parse_comments("/* hello */\nhello world").unwrap());
        assert_eq!(("hello world", "/** hello */\n"), Sanitizer::parse_comments("/** hello */\nhello world").unwrap());
        assert_eq!(("/\nhello world", ""), Sanitizer::parse_comments("/\nhello world").unwrap());
        assert_eq!(
            ("hello world", "// hel\u{4141}lo\n"),
            Sanitizer::parse_comments("// hel\u{4141}lo\nhello world").unwrap()
        );
        assert!(Sanitizer::parse_comments("// hel\x08lo\nhello world").is_err());
        assert!(Sanitizer::parse_comments("// hel\u{2066}lo\nhello world").is_err());
        assert!(Sanitizer::parse_comments("/* hel\x7flo */\nhello world").is_err());
        assert!(Sanitizer::parse_comments("/* hel\u{202d}lo */\nhello world").is_err());
        assert!(Sanitizer::parse_comments("/** hel\x00lo */\nhello world").is_err());
        assert!(Sanitizer::parse_comments("/** hel\u{202a}lo */\nhello world").is_err());
    }
}
