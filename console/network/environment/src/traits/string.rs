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

/// From https://github.com/Geal/nom/blob/main/examples/string.rs
pub mod string_parser {
    //! This example shows an example of how to parse an escaped string. The
    //! rules for the string are similar to JSON and rust. A string is:
    //!
    //! - Enclosed by double quotes
    //! - Can contain any raw unescaped code point besides \ and "
    //! - Matches the following escape sequences: \b, \f, \n, \r, \t, \", \\, \/
    //! - Matches code points like Rust: \u{XXXX}, where XXXX can be up to 6
    //!   hex characters
    //! - an escape followed by whitespace consumes all whitespace between the
    //!   escape and the next non-whitespace character

    use nom::{
        branch::alt,
        bytes::streaming::{is_not, take_while_m_n},
        character::streaming::{char, multispace1},
        combinator::{map, map_opt, map_res, value, verify},
        error::{ErrorKind, FromExternalError, ParseError},
        multi::fold_many0,
        sequence::{delimited, preceded},
        Err::Error,
        IResult,
    };

    /// The list of unsupported "invisible" code points.
    /// See [Sanitizer::parse_safe_char] for an explanation.
    ///
    /// Instead of using this list to test whether its elements occur in a string literal,
    /// we should probably rework the string literal parsing code
    /// to call [Sanitizer::parse_safe_char],
    /// and remove this list.
    const UNSUPPORTED_CODE_POINTS: [&str; 39] = [
        "\x00", "\x01", "\x02", "\x03", "\x04", "\x05", "\x06", "\x07", "\x08", "\x0b", "\x0c", "\x0e", "\x0f", "\x10",
        "\x11", "\x12", "\x13", "\x14", "\x15", "\x16", "\x17", "\x18", "\x19", "\x1a", "\x1b", "\x1c", "\x1d", "\x1e",
        "\x1f", "\x7f", "\u{202a}", "\u{202b}", "\u{202c}", "\u{202d}", "\u{202e}", "\u{2066}", "\u{2067}", "\u{2068}",
        "\u{2069}",
    ];

    /// Parse a unicode sequence, of the form u{XXXX}, where XXXX is 1 to 6
    /// hexadecimal numerals. We will combine this later with parse_escaped_char
    /// to parse sequences like \u{00AC}.
    fn parse_unicode<'a, E>(input: &'a str) -> IResult<&'a str, char, E>
    where
        E: ParseError<&'a str> + FromExternalError<&'a str, std::num::ParseIntError>,
    {
        // `take_while_m_n` parses between `m` and `n` bytes (inclusive) that match
        // a predicate. `parse_hex` here parses between 1 and 6 hexadecimal numerals.
        let parse_hex = take_while_m_n(1, 6, |c: char| c.is_ascii_hexdigit());

        // `preceded` takes a prefix parser, and if it succeeds, returns the result
        // of the body parser. In this case, it parses u{XXXX}.
        let parse_delimited_hex = preceded(
            char('u'),
            // `delimited` is like `preceded`, but it parses both a prefix and a suffix.
            // It returns the result of the middle parser. In this case, it parses
            // {XXXX}, where XXXX is 1 to 6 hex numerals, and returns XXXX
            delimited(char('{'), parse_hex, char('}')),
        );

        // `map_res` takes the result of a parser and applies a function that returns
        // a Result. In this case we take the hex bytes from parse_hex and attempt to
        // convert them to a u32.
        let parse_u32 = map_res(parse_delimited_hex, move |hex| u32::from_str_radix(hex, 16));

        // map_opt is like map_res, but it takes an Option instead of a Result. If
        // the function returns None, map_opt returns an error. In this case, because
        // not all u32 values are valid unicode code points, we have to fallibly
        // convert to char with from_u32.
        map_opt(parse_u32, std::char::from_u32)(input)
    }

    /// Parse an escaped character: \n, \t, \r, \u{00AC}, etc.
    fn parse_escaped_char<'a, E>(input: &'a str) -> IResult<&'a str, char, E>
    where
        E: ParseError<&'a str> + FromExternalError<&'a str, std::num::ParseIntError>,
    {
        preceded(
            char('\\'),
            // `alt` tries each parser in sequence, returning the result of
            // the first successful match
            alt((
                parse_unicode,
                // The `value` parser returns a fixed value (the first argument) if its
                // parser (the second argument) succeeds. In these cases, it looks for
                // the marker characters (n, r, t, etc) and returns the matching
                // character (\n, \r, \t, etc).
                value('\n', char('n')),
                value('\r', char('r')),
                value('\t', char('t')),
                value('\u{08}', char('b')),
                value('\u{0C}', char('f')),
                value('\\', char('\\')),
                value('/', char('/')),
                value('"', char('"')),
            )),
        )(input)
    }

    /// Parse a backslash, followed by any amount of whitespace. This is used later
    /// to discard any escaped whitespace.
    fn parse_escaped_whitespace<'a, E: ParseError<&'a str>>(input: &'a str) -> IResult<&'a str, &'a str, E> {
        preceded(char('\\'), multispace1)(input)
    }

    /// Parse a non-empty block of text that doesn't include \ or "
    fn parse_literal<'a, E: ParseError<&'a str>>(input: &'a str) -> IResult<&'a str, &'a str, E> {
        // Return an error if the literal contains an unsupported code point.
        for code_point in UNSUPPORTED_CODE_POINTS {
            if input.contains(code_point) {
                return Err(Error(E::from_error_kind("String literal contains invalid codepoint", ErrorKind::Char)));
            }
        }

        // `is_not` parses a string of 0 or more characters that aren't one of the
        // given characters.
        let not_quote_slash = is_not("\"\\");

        // `verify` runs a parser, then runs a verification function on the output of
        // the parser. The verification function accepts out output only if it
        // returns true. In this case, we want to ensure that the output of is_not
        // is non-empty.
        verify(not_quote_slash, |s: &str| !s.is_empty())(input)
    }

    /// A string fragment contains a fragment of a string being parsed: either
    /// a non-empty Literal (a series of non-escaped characters), a single
    /// parsed escaped character, or a block of escaped whitespace.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum StringFragment<'a> {
        Literal(&'a str),
        EscapedChar(char),
        EscapedWS,
    }

    /// Combine parse_literal, parse_escaped_whitespace, and parse_escaped_char
    /// into a StringFragment.
    fn parse_fragment<'a, E>(input: &'a str) -> IResult<&'a str, StringFragment<'a>, E>
    where
        E: ParseError<&'a str> + FromExternalError<&'a str, std::num::ParseIntError>,
    {
        alt((
            // The `map` combinator runs a parser, then applies a function to the output
            // of that parser.
            map(parse_literal, StringFragment::Literal),
            map(parse_escaped_char, StringFragment::EscapedChar),
            value(StringFragment::EscapedWS, parse_escaped_whitespace),
        ))(input)
    }

    /// Parse a string. Use a loop of parse_fragment and push all of the fragments
    /// into an output string.
    pub fn parse_string<'a, E>(input: &'a str) -> IResult<&'a str, String, E>
    where
        E: ParseError<&'a str> + FromExternalError<&'a str, std::num::ParseIntError>,
    {
        // fold_many0 is the equivalent of iterator::fold. It runs a parser in a loop,
        // and for each output value, calls a folding function on each output value.
        let build_string = fold_many0(
            // Our parser function– parses a single string fragment
            parse_fragment,
            // Our init value, an empty string
            String::new,
            // Our folding function. For each fragment, append the fragment to the
            // string.
            |mut string, fragment| {
                match fragment {
                    StringFragment::Literal(s) => string.push_str(s),
                    StringFragment::EscapedChar(c) => string.push(c),
                    StringFragment::EscapedWS => {}
                }
                string
            },
        );

        // Finally, parse the string. Note that, if `build_string` could accept a raw
        // " character, the closing delimiter " would never match. When using
        // `delimited` with a looping parser (like fold_many0), be sure that the
        // loop won't accidentally match your closing delimiter!
        delimited(char('"'), build_string, char('"'))(input)
    }
}

#[test]
fn test_parse_string() {
    // to use parse_string_wrapper instead of string_parser::parse_string::<nom::error::VerboseError<&str>> in the tests below:
    fn parse_string_wrapper(input: &str) -> crate::ParserResult<String> {
        string_parser::parse_string(input)
    }

    // tests some correct string literals:
    assert_eq!(("", String::from("")), parse_string_wrapper("\"\"").unwrap());
    assert_eq!(("", String::from("abc")), parse_string_wrapper("\"abc\"").unwrap());
    assert_eq!((" and more", String::from("abc")), parse_string_wrapper("\"abc\" and more").unwrap());
    assert_eq!(("", String::from("\r")), parse_string_wrapper("\"\r\"").unwrap());
    assert_eq!(("", String::from("4\u{4141}x\x09")), parse_string_wrapper("\"4\u{4141}x\x09\"").unwrap());

    // test rejection of disallowed characters:
    assert!(parse_string_wrapper("\"hel\x08lo\"").is_err());
    assert!(parse_string_wrapper("\"hel\x1flo\"").is_err());
    assert!(parse_string_wrapper("\"hel\u{2069}lo\"").is_err());
}
