//! A zero-copy implementation of IRC message parsing.

use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

use error::{MessageParseError, ProtocolError};


#[derive(Debug, PartialEq, Eq, Clone, Copy)]
struct Part {
    start: u16,
    end: u16,
}

impl Part {
    fn new(start: usize, end: usize) -> Part {
        Part {
            start: start as u16,
            end: end as u16,
        }
    }

    fn index<'a>(&self, s: &'a str) -> &'a str {
        &s[self.start as usize..self.end as usize]
    }
}

/// The maximum number of bytes allowed in a message buffer, currently set to `u16::max_value()` as
/// the maximum value of the buffer's pointer types.
pub const MAX_BYTES: usize = u16::max_value() as usize;

/// A parsed IRC message string, containing a single buffer with pointers to the individual parts.
#[derive(Clone, PartialEq, Debug)]
pub struct MessageBuf {
    buf: String,
    tags: Option<Part>,
    prefix: Option<Part>,
    command: Part,
    args: Part,
    suffix: Option<Part>,
}

impl MessageBuf {
    /// Parses the message, converting the given object into an owned string.
    ///
    /// This will allocate a new `String` to hold the message data, even if a `String` is
    /// passed. To avoid this and transfer ownership instead, use the [`parse_string`] method.
    ///
    /// This function does not parse arguments or tags, as those may have an arbitrary number of
    /// elements and would require additional allocations to hold their pointer data. They have
    /// their own iterator-parsers that produce the elements while avoiding additional allocations;
    /// see the [`args`] and [`tags`] methods for more information.
    ///
    /// # Error
    ///
    /// This method will fail in the following conditions:
    ///
    /// - The message length is longer than the maximum supported number of bytes ([`MAX_BYTES`]).
    /// - The message is missing required components such as the trailing CRLF or the command.
    ///
    /// Note that it does not check whether the parts of the message have illegal forms, as
    /// there is little benefit to restricting that.
    ///
    /// # Examples
    ///
    /// ```
    /// # fn main() -> Result<(), irc_proto::error::MessageParseError> {
    /// use irc_proto::MessageBuf;
    ///
    /// let message = MessageBuf::parse("PRIVMSG #rust :Hello Rustaceans!\r\n")?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// [`parse_string`]: #method.parse_string
    /// [`args`]: #method.args
    /// [`tags`]: #method.tags
    /// [`MAX_BYTES`]: ./constant.MAX_BYTES.html
    pub fn parse<S>(message: S) -> Result<Self, MessageParseError>
    where
        S: ToString,
    {
        MessageBuf::parse_string(message.to_string())
    }

    /// Takes ownership of the given string and parses it into a message.
    ///
    /// For more information about the details of the parser, see the [`parse`] method.
    ///
    /// # Examples
    ///
    /// ```
    /// # fn main() -> Result<(), irc_proto::error::MessageParseError> {
    /// use irc_proto::MessageBuf;
    ///
    /// let message = MessageBuf::parse_string("NICK ferris\r\n".to_string())?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// [`parse`]: #method.parse
    pub fn parse_string(message: String) -> Result<Self, MessageParseError> {
        // To make sure pointers don't overflow:
        if message.len() > MAX_BYTES {
            return Err(MessageParseError::MaxLengthExceeded);
        }

        // Make sure the message is terminated with line endings:
        if !message.ends_with("\r\n") {
            return Err(MessageParseError::MissingCrLf);
        }
        // Used as the end of the "useful" part of the message.
        let crlf = message.len() - '\n'.len_utf8() - '\r'.len_utf8();

        // Accumulating pointer used to keep track of how much has already been parsed.
        let mut i = 0;

        // If word starts with '@', it is a tag.
        let tags;
        if message[i..].starts_with('@') {
            // Take everything between '@' and next space.
            i += '@'.len_utf8();
            let start = i;

            i += message[i..].find(' ').unwrap_or_else(|| crlf - i);
            let end = i;

            tags = Some(Part::new(start, end));
        } else {
            tags = None;
        }

        // Skip to next non-space.
        while message[i..].starts_with(' ') {
            i += ' '.len_utf8();
        }

        // If word starts with ':', it is a prefix.
        let prefix;
        if message[i..].starts_with(':') {
            // Take everything between ':' and next space.
            i += ':'.len_utf8();
            let start = i;

            i += message[i..].find(' ').unwrap_or_else(|| crlf - i);
            let end = i;

            prefix = Some(Part::new(start, end));
        } else {
            prefix = None;
        }

        // Skip to next non-space.
        while message[i..].starts_with(' ') {
            i += ' '.len_utf8();
        }

        // Next word must be command.
        let command = {
            // Take everything between here and next space.
            let start = i;

            i += message[i..].find(' ').unwrap_or_else(|| crlf - i);
            let end = i;

            Part::new(start, end)
        };

        // Command name must not be empty.
        if command.start == command.end {
            return Err(MessageParseError::MissingCommand);
        }

        // Skip to next non-space.
        while message[i..].starts_with(' ') {
            i += ' '.len_utf8();
        }

        // Everything from here to crlf must be args.
        let args;
        let suffix;

        // If " :" exists in the remaining data, the first instance marks the beginning of a
        // suffix.
        if let Some(suffix_idx) = message[i..].find(" :") {
            // Middle args are everything from the current position to the last
            // non-space character before the suffix.
            let start = i;

            // Walking back to the last non-space character:
            let mut j = i + suffix_idx;
            while message[..j].ends_with(' ') {
                j -= ' '.len_utf8();
            }
            let end = j;
            args = Part::new(start, end);

            // Suffix is everything between the leading " :" and crlf.
            i += suffix_idx + ' '.len_utf8() + ':'.len_utf8();
            let start = i;
            i = crlf;
            let end = i;
            suffix = Some(Part::new(start, end));
        } else {
            // Middle arg are everything from the current position to the last non-space
            // character before crlf.
            let start = i;

            // Walking back to the last non-space character:
            let mut j = crlf;
            while message[..j].ends_with(' ') {
                j -= ' '.len_utf8();
            }
            let end = j;
            args = Part::new(start, end);

            // Suffix does not exist:
            suffix = None;
        }

        // Done parsing.
        Ok(MessageBuf {
            buf: message,
            tags,
            prefix,
            command,
            args,
            suffix,
        })
    }

    /// Returns a borrowed string slice containing the serialized message.
    ///
    /// # Examples
    ///
    /// ```
    /// # fn main() -> Result<(), irc_proto::error::MessageParseError> {
    /// use irc_proto::MessageBuf;
    ///
    /// let raw_message = "JOIN #rust\r\n";
    /// let parsed_message = MessageBuf::parse(raw_message)?;
    /// assert_eq!(parsed_message.as_str(), raw_message);
    /// # Ok(())
    /// # }
    pub fn as_str(&self) -> &str {
        &self.buf
    }

    /// Consumes this message, producing the inner string that contains the serialized message.
    ///
    /// # Examples
    ///
    /// ```
    /// # fn main() -> Result<(), irc_proto::error::MessageParseError> {
    /// use irc_proto::MessageBuf;
    ///
    /// let raw_message = "JOIN #rust\r\n";
    /// let parsed_message = MessageBuf::parse(raw_message)?;
    /// assert_eq!(parsed_message.into_string(), raw_message);
    /// # Ok(())
    /// # }
    pub fn into_string(self) -> String {
        self.buf
    }

    /// Produces a parser iterator over the message's tags. The iterator will produce items of
    /// `(&str, Option<Cow<str>>)` for each tag in order, containing the tag's key and its value if
    /// one exists for that key. It is mostly zero-copy, borrowing in all cases except when the
    /// value contains escape sequences, in which case the unescaped value will be produced and
    /// stored in an owned buffer.
    ///
    /// This parser will not dedupe tags, nor will it check whether the tag's key is empty or
    /// whether it contains illegal characters. 
    ///
    /// # Examples
    ///
    /// ```
    /// # fn main() -> Result<(), irc_proto::error::MessageParseError> {
    /// use irc_proto::MessageBuf;
    /// use std::borrow::Cow;
    ///
    /// let message = MessageBuf::parse(
    ///     "@aaa=bbb;ccc;example.com/ddd=eee :nick!ident@host.com PRIVMSG me :Hello\r\n"
    /// )?;
    ///
    /// let mut tags = message.tags();
    /// assert_eq!(tags.len(), 3);
    ///
    /// assert_eq!(tags.next(), Some(("aaa", Some(Cow::Borrowed("bbb")))));
    /// assert_eq!(tags.next(), Some(("ccc", None)));
    /// assert_eq!(tags.next(), Some(("example.com/ddd", Some(Cow::Borrowed("eee")))));
    /// assert_eq!(tags.next(), None);
    /// # Ok(())
    /// # }
    /// ```
    pub fn tags(&self) -> Tags {
        Tags {
            remaining: self.tags.as_ref().map(|part| part.index(&self.buf)).unwrap_or(""),
        }
    }

    /// Returns a string slice containing the message's prefix, if it exists.
    ///
    /// # Examples
    ///
    /// ```
    /// # fn main() -> Result<(), irc_proto::error::MessageParseError> {
    /// use irc_proto::MessageBuf;
    ///
    /// let message = MessageBuf::parse(":nick!ident@host.com PRIVMSG me :Hello\r\n")?;
    /// assert_eq!(message.prefix(), Some("nick!ident@host.com"));
    /// # Ok(())
    /// # }
    /// ```
    pub fn prefix(&self) -> Option<&str> {
        self.prefix.as_ref().map(|part| part.index(&self.buf))
    }

    /// Returns a string slice containing the message's command.
    ///
    /// # Examples
    ///
    /// ```
    /// # fn main() -> Result<(), irc_proto::error::MessageParseError> {
    /// use irc_proto::MessageBuf;
    ///
    /// let message = MessageBuf::parse("NICK ferris\r\n")?;
    /// assert_eq!(message.command(), "NICK");
    /// # Ok(())
    /// # }
    /// ```
    pub fn command(&self) -> &str {
        self.command.index(&self.buf)
    }

    /// Returns a parser iterator over the message's arguments. The iterator will produce items of 
    /// `&str` for each argument in order, containing the raw data in the argument. It is entirely
    /// zero-copy, borrowing each argument slice directly from the message buffer.
    ///
    /// # Examples
    ///
    /// ```
    /// # fn main() -> Result<(), irc_proto::error::MessageParseError> {
    /// use irc_proto::MessageBuf;
    ///
    /// let message = MessageBuf::parse("USER guest tolmoon tolsun :Ronnie Reagan\r\n")?;
    /// let mut args = message.args();
    /// assert_eq!(args.len(), 3);
    /// assert_eq!(args.next(), Some("guest"));
    /// assert_eq!(args.next(), Some("tolmoon"));
    /// assert_eq!(args.next(), Some("tolsun"));
    /// assert_eq!(args.next(), None);
    /// # Ok(())
    /// # }
    /// ```
    pub fn args(&self) -> Args {
        Args {
            remaining: self.args.index(&self.buf),
        }
    }

    /// Returns the suffix of this message, if one exists.
    ///
    /// # Examples
    ///
    /// ```
    /// # fn main() -> Result<(), irc_proto::error::MessageParseError> {
    /// use irc_proto::MessageBuf;
    /// 
    /// let message = MessageBuf::parse("USER guest tolmoon tolsun :Ronnie Reagan\r\n")?;
    /// assert_eq!(message.suffix(), Some("Ronnie Reagan"));
    /// # Ok(())
    /// # }
    pub fn suffix(&self) -> Option<&str> {
        self.suffix.map(|part| part.index(&self.buf))
    }
}

impl FromStr for MessageBuf {
    type Err = ProtocolError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        MessageBuf::parse(s)
            .map_err(|err| ProtocolError::InvalidMessage { string: s.to_string(), cause: err })
    }
}

impl AsRef<str> for MessageBuf {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for MessageBuf {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.buf)
    }
}

/// A parser iterator over a message's tags. See [`MessageBuf::tags`] for more information.
///
/// [`MessageBuf::tags`]: ./struct.MessageBuf.html#method.tags
pub struct Tags<'a> {
    remaining: &'a str,
}

impl<'a> Iterator for Tags<'a> {
    type Item = (&'a str, Option<Cow<'a, str>>);

    fn next(&mut self) -> Option<Self::Item> {
        // If remaining is empty, nothing is left to yield.
        if self.remaining.len() == 0 {
            None
        } else {
            // Take everything from here to next ';'.
            let tag = self.remaining
                .char_indices()
                .find(|&(_i, c)| c == ';')
                .map(|(i, _c)| &self.remaining[..i])
                .unwrap_or(&self.remaining);

            // Remove taken data from the remaining buffer.
            if self.remaining.len() == tag.len() {
                self.remaining = "";
            } else {
                self.remaining = &self.remaining[tag.len() + ';'.len_utf8()..];
            }
            
            // If an equal sign exists in the tag data, it must have an associated value.
            if let Some(key_end) = tag.find('=') {
                // Everything before the first equal sign is the key.
                let key = &tag[..key_end];

                // Everything after the first equal sign is the value.
                let mut raw_value = &tag[key_end + '='.len_utf8()..];

                // Resolve escape sequences if any are found.
                // This will not allocate unless data is given to it.
                let mut value = String::new();
                while let Some(escape_idx) = raw_value.find('\\') {
                    // Copy everything before this escape sequence.
                    value.push_str(&raw_value[..escape_idx]);
                    // Resolve this escape sequence.
                    let c = match raw_value[escape_idx + '\\'.len_utf8()..].chars().next() {
                        Some(':') => Some(';'),
                        Some('s') => Some(' '),
                        Some('\\') => Some('\\'),
                        Some('r') => Some('\r'),
                        Some('n') => Some('\n'),
                        Some(c) => Some(c),
                        None => None,
                    };
                    // If it resolves to a character, then push it.
                    if let Some(c) = c {
                        value.push(c);
                    }
                    // Cut off the beginning of raw_value such that it only contains
                    // everything after the parsed escape sequence.
                    // Upon looping, it will start searching from this point, skipping the last
                    // escape sequence.
                    raw_value = &raw_value[
                        (escape_idx
                            + '\\'.len_utf8()
                            + c.map(char::len_utf8).unwrap_or(0)
                        )..
                    ];
                }

                // If we didn't add data, no escape sequences exist and the raw value can be
                // referenced.
                if value.len() == 0 {
                    Some((key, Some(Cow::Borrowed(raw_value))))
                } else {
                    // Make sure you add the rest of the raw value that doesn't contain escapes.
                    value.push_str(raw_value);
                    Some((key, Some(Cow::Owned(value))))
                }
            } else {
                Some((tag, None))
            }
        }
    }
}

impl<'a> ExactSizeIterator for Tags<'a> {
    fn len(&self) -> usize {
        // Number of tags yielded is number of remaining semicolons plus one, unless the
        // remaining buffer is empty.
        if self.remaining.len() == 0 {
            0
        } else {
            self.remaining.chars().filter(|&c| c == ';').count() + 1
        }
    }
}

/// An iterator over a message's arguments. See [`MessageBuf::args`] for more information.
///
/// [`MessageBuf::args`]: ./struct.MessageBuf.html#method.args
pub struct Args<'a> {
    remaining: &'a str,
}

impl<'a> Iterator for Args<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        // If remaining slice is non-empty, we still have args to take:
        if self.remaining.len() > 0 {
            // Next arg is everything from here to next whitespace character (or end of string).
            let arg_end = self.remaining.find(' ').unwrap_or(self.remaining.len());
            let arg = &self.remaining[..arg_end];

            // Trim this arg and its trailing spaces out of remaining.
            self.remaining = self.remaining[arg_end..].trim_start_matches(' ');

            Some(arg)
        } else {
            // No more args to parse.
            None
        }
    }
}

impl<'a> ExactSizeIterator for Args<'a> {
    fn len(&self) -> usize {
        // Number of args remaining is equal to the number of points where a non-space
        // character is preceded by a space character or the beginning of the string.
        let mut len = 0;
        let mut last = true;
        for c in self.remaining.chars() {
            let current = c == ' ';
            if (last, current) == (true, false) {
                len += 1;
            }
            last = current;
        }
        len
    }
}

#[cfg(test)]
mod test {
    // TODO
}