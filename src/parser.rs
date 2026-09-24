use crate::{Arg, Error, ErrorKind, Value};
use core::iter::FusedIterator;
use core::ops::Range;
use core::str::FromStr;
use std::env;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

/// A small, borrow-first parser over OS-native command-line arguments.
///
/// The parser is intentionally imperative:
///
/// 1. Pull one [`Arg`] at a time with [`Parser::next`].
/// 2. Match that argument explicitly in user code.
/// 3. Call [`Parser::value`] or [`Parser::value_opt`] immediately when the
///    current option expects a value.
///
/// Any value access only applies to the most recently returned option.
///
/// The parser recognizes the common CLI forms listed in the crate-level docs,
/// but it deliberately stays schema-free:
///
/// - it does not know which flags are valid ahead of time
/// - it does not infer whether a value is required or optional
/// - it never owns help text or command metadata
///
/// Instead, user code keeps full control over matching, validation, and output.
#[derive(Debug)]
pub struct Parser<I> {
    iter: I,
    peeked: Option<OsString>,
    current: Option<OsString>,
    value_slot: Option<OsString>,
    pending_shorts: Option<PendingShorts>,
    value_source: ValueSource,
    options_done: bool,
}

/// Remaining raw OS arguments yielded after partially parsing a command line.
///
/// This iterator is primarily useful for wrapper and passthrough CLIs that
/// need to stop structured parsing and forward the rest of the command line.
///
/// It preserves any token already peeked by [`Parser::value_opt`], and it
/// reconstructs grouped short tails so the forwarded view matches what the
/// parser would have seen next.
///
/// See [`Parser::into_remaining`] for an example.
#[derive(Debug)]
pub struct Remaining<I> {
    front: Option<OsString>,
    peeked: Option<OsString>,
    iter: I,
}

#[derive(Debug, Clone, Copy)]
struct PendingShorts {
    next_index: usize,
}

#[derive(Debug, Clone)]
enum ValueSource {
    None,
    Attached(Range<usize>),
    ShortTail(Range<usize>),
    NextArgument,
    Consumed,
}

impl Parser<std::iter::Skip<env::ArgsOs>> {
    /// Builds a parser from `std::env::args_os()` and skips `argv[0]`.
    ///
    /// This is the normal entry point for executable code.
    #[must_use = "the parser does nothing until you iterate it"]
    pub fn from_env() -> Self {
        Self::new(env::args_os().skip(1))
    }
}

impl Parser<std::vec::IntoIter<OsString>> {
    /// Builds a parser by collecting any argument source into owned [`OsString`] values.
    ///
    /// This is primarily useful in docs, tests, examples, and wrapper code that
    /// starts from `&str`, `String`, or existing `OsString` values.
    ///
    /// Unlike [`Parser::new`], this convenience constructor first owns the
    /// arguments by collecting them. If you already have an iterator of
    /// [`OsString`] values, prefer [`Parser::new`] to avoid the extra allocation.
    ///
    /// ```rust
    /// use osarg::{Arg, Parser};
    ///
    /// let mut parser = Parser::from_args(["--port", "8080"]);
    ///
    /// assert_eq!(parser.next()?, Some(Arg::Long("port")));
    /// assert_eq!(parser.parse::<u16>()?, 8080);
    /// # Ok::<(), osarg::Error>(())
    /// ```
    #[must_use = "the parser does nothing until you iterate it"]
    pub fn from_args<T, S>(args: T) -> Self
    where
        T: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        Self::new(
            args.into_iter()
                .map(Into::into)
                .collect::<Vec<_>>()
                .into_iter(),
        )
    }
}

impl<I> Parser<I>
where
    I: Iterator<Item = OsString>,
{
    /// Builds a parser from any iterator of [`OsString`] values.
    ///
    /// This is primarily useful in tests, examples, and wrapper APIs that
    /// already own an argument iterator.
    #[must_use = "the parser does nothing until you iterate it"]
    pub fn new(iter: I) -> Self {
        Self {
            iter,
            peeked: None,
            current: None,
            value_slot: None,
            pending_shorts: None,
            value_source: ValueSource::None,
            options_done: false,
        }
    }

    /// Returns the next parsed argument.
    ///
    /// If the current option expects a value, call [`Parser::value`] or
    /// [`Parser::value_opt`] before calling `next()` again.
    ///
    /// Short-option tails are handled lazily. For example, `-p8080` returns
    /// `Arg::Short('p')`; calling [`Parser::value`] immediately afterwards
    /// yields `8080`. If you do not ask for a value, the remaining bytes are
    /// emitted as more short options.
    ///
    /// `--` switches the parser into positional-only mode.
    ///
    /// ```rust
    /// use osarg::{Arg, Parser};
    ///
    /// let mut parser = Parser::from_args(["-vp8080", "--", "--literal"]);
    ///
    /// assert_eq!(parser.next()?, Some(Arg::Short('v')));
    /// assert_eq!(parser.next()?, Some(Arg::Short('p')));
    /// assert_eq!(parser.value()?.parse::<u16>()?, 8080);
    /// assert_eq!(
    ///     parser.next()?.and_then(|arg| arg.as_value()).map(|value| value.to_os_string()),
    ///     Some(std::ffi::OsString::from("--literal"))
    /// );
    /// assert_eq!(parser.next()?, None);
    /// # Ok::<(), osarg::Error>(())
    /// ```
    #[expect(
        clippy::should_implement_trait,
        reason = "the parser intentionally exposes a fallible next() that mirrors iterator-style control flow"
    )]
    pub fn next(&mut self) -> Result<Option<Arg<'_>>, Error> {
        self.value_slot = None;
        self.value_source = ValueSource::None;

        if self.pending_shorts.is_some() {
            return Ok(Some(self.emit_short()?));
        }

        loop {
            let Some(current) = self.take_next_raw() else {
                self.current = None;
                return Ok(None);
            };

            self.current = Some(current);

            if self.options_done {
                return Ok(Some(Arg::Value(Value::new(self.current_os_str()))));
            }

            enum CurrentKind {
                Sentinel,
                Long {
                    name: Range<usize>,
                    value: Option<Range<usize>>,
                },
                Short,
                Value,
            }

            let kind = {
                let bytes = self.current_os_str().as_encoded_bytes();

                if bytes == b"--" {
                    CurrentKind::Sentinel
                } else if bytes.len() > 2 && bytes.starts_with(b"--") {
                    let body = &bytes[2..];
                    let eq_index = body.iter().position(|byte| *byte == b'=');
                    let name_end = eq_index.map_or(bytes.len(), |index| index + 2);
                    let value = eq_index.map(|index| index + 3..bytes.len());

                    CurrentKind::Long {
                        name: 2..name_end,
                        value,
                    }
                } else if bytes.len() > 1 && bytes[0] == b'-' && bytes[1] != b'-' {
                    CurrentKind::Short
                } else {
                    CurrentKind::Value
                }
            };

            match kind {
                CurrentKind::Sentinel => {
                    self.current = None;
                    self.options_done = true;
                }
                CurrentKind::Long { name, value } => {
                    let bytes = self.current_os_str().as_encoded_bytes();
                    if name.is_empty() {
                        return Err(self.invalid_option_name());
                    }

                    let name_is_utf8 = core::str::from_utf8(&bytes[name.clone()]).is_ok();

                    if !name_is_utf8 {
                        return Err(self.invalid_option_name());
                    }

                    self.value_source = match value {
                        Some(range) => ValueSource::Attached(range),
                        None => ValueSource::NextArgument,
                    };

                    let bytes = self.current_os_str().as_encoded_bytes();
                    // SAFETY: this slice was validated as UTF-8 immediately above.
                    let name = unsafe { core::str::from_utf8_unchecked(&bytes[name]) };

                    return Ok(Some(Arg::Long(name)));
                }
                CurrentKind::Short => {
                    self.pending_shorts = Some(PendingShorts { next_index: 2 });
                    return Ok(Some(self.emit_short()?));
                }
                CurrentKind::Value => {
                    return Ok(Some(Arg::Value(Value::new(self.current_os_str()))));
                }
            }
        }
    }

    /// Returns the required value for the current option.
    ///
    /// This method must be called immediately after the option returned by
    /// [`Parser::next`]. It understands the common attached forms
    /// `--name=value` and `-p8080`.
    ///
    /// When no value is available, it returns [`ErrorKind::MissingValue`].
    ///
    /// ```rust
    /// use osarg::{Arg, Parser};
    ///
    /// let mut parser = Parser::from_args(["--port=8080", "-p9090"]);
    ///
    /// assert_eq!(parser.next()?, Some(Arg::Long("port")));
    /// assert_eq!(parser.value()?.parse::<u16>()?, 8080);
    ///
    /// assert_eq!(parser.next()?, Some(Arg::Short('p')));
    /// assert_eq!(parser.value()?.parse::<u16>()?, 9090);
    /// # Ok::<(), osarg::Error>(())
    /// ```
    #[must_use = "callers must use or propagate the parsed value"]
    pub fn value(&mut self) -> Result<Value<'_>, Error> {
        match self.value_source.clone() {
            ValueSource::Attached(range) => {
                self.value_source = ValueSource::Consumed;
                Ok(Value::new(slice_os_str(self.current_os_str(), range)))
            }
            ValueSource::ShortTail(range) => {
                self.pending_shorts = None;
                self.value_source = ValueSource::Consumed;
                Ok(Value::new(slice_os_str(self.current_os_str(), range)))
            }
            ValueSource::NextArgument => {
                let value = self
                    .take_next_raw()
                    .ok_or_else(|| Error::missing_value(self.current_os_str()))?;
                self.value_source = ValueSource::Consumed;
                self.value_slot = Some(value);
                Ok(Value::new(
                    self.value_slot.as_deref().expect("value slot set"),
                ))
            }
            ValueSource::None | ValueSource::Consumed => {
                Err(Error::value_unavailable(self.current.as_deref()))
            }
        }
    }

    /// Returns the required value for the current option as an owned [`OsString`].
    ///
    /// This is shorthand for `parser.value()?.to_os_string()`.
    ///
    /// ```rust
    /// use osarg::{Arg, Parser};
    ///
    /// let mut parser = Parser::from_args(["--path", "./data"]);
    ///
    /// assert_eq!(parser.next()?, Some(Arg::Long("path")));
    /// assert_eq!(parser.os_string()?, std::ffi::OsString::from("./data"));
    /// # Ok::<(), osarg::Error>(())
    /// ```
    #[must_use = "callers must use or propagate the owned OS string"]
    pub fn os_string(&mut self) -> Result<OsString, Error> {
        Ok(self.value()?.to_os_string())
    }

    /// Returns the required value for the current option as an owned [`PathBuf`].
    ///
    /// This is shorthand for `parser.value()?.to_path_buf()`.
    ///
    /// ```rust
    /// use osarg::{Arg, Parser};
    /// use std::path::PathBuf;
    ///
    /// let mut parser = Parser::from_args(["--root", "./data"]);
    ///
    /// assert_eq!(parser.next()?, Some(Arg::Long("root")));
    /// assert_eq!(parser.path_buf()?, PathBuf::from("./data"));
    /// # Ok::<(), osarg::Error>(())
    /// ```
    #[must_use = "callers must use or propagate the owned path value"]
    pub fn path_buf(&mut self) -> Result<PathBuf, Error> {
        Ok(self.value()?.to_path_buf())
    }

    /// Returns the required value for the current option as UTF-8.
    ///
    /// This is shorthand for `parser.value()?.to_str()`.
    ///
    /// ```rust
    /// use osarg::{Arg, Parser};
    ///
    /// let mut parser = Parser::from_args(["--bind", "0.0.0.0"]);
    ///
    /// assert_eq!(parser.next()?, Some(Arg::Long("bind")));
    /// assert_eq!(parser.string()?, "0.0.0.0");
    /// # Ok::<(), osarg::Error>(())
    /// ```
    #[must_use = "callers must use or propagate the UTF-8 string value"]
    pub fn string(&mut self) -> Result<&str, Error> {
        self.value()?.to_str()
    }

    /// Returns the required value for the current option as an owned UTF-8 [`String`].
    ///
    /// This is shorthand for `parser.value()?.to_owned_string()`.
    ///
    /// ```rust
    /// use osarg::{Arg, Parser};
    ///
    /// let mut parser = Parser::from_args(["--bind", "0.0.0.0"]);
    ///
    /// assert_eq!(parser.next()?, Some(Arg::Long("bind")));
    /// assert_eq!(parser.string_owned()?, String::from("0.0.0.0"));
    /// # Ok::<(), osarg::Error>(())
    /// ```
    #[must_use = "callers must use or propagate the owned UTF-8 string value"]
    pub fn string_owned(&mut self) -> Result<String, Error> {
        self.value()?.to_owned_string()
    }

    /// Stores the current flag into a boolean slot that must be set at most once.
    ///
    /// This is useful for single-occurrence flags such as `--dry-run`.
    /// If the slot is already `true`, this returns
    /// [`crate::ErrorKind::UnexpectedArgument`] for the current flag.
    ///
    /// ```rust
    /// use osarg::{Arg, Parser};
    ///
    /// let mut parser = Parser::from_args(["--dry-run"]);
    /// let mut dry_run = false;
    ///
    /// assert_eq!(parser.next()?, Some(Arg::Long("dry-run")));
    /// parser.store_flag(&mut dry_run)?;
    ///
    /// assert!(dry_run);
    /// # Ok::<(), osarg::Error>(())
    /// ```
    pub fn store_flag(&self, slot: &mut bool) -> Result<(), Error> {
        if *slot {
            return Err(self.unexpected_current_argument());
        }

        *slot = true;
        Ok(())
    }

    /// Stores the required value for the current option into an [`OsString`] slot that must be set at most once.
    ///
    /// This is useful for single-occurrence options such as `--output FILE`.
    /// If the slot is already populated, this returns [`crate::ErrorKind::UnexpectedArgument`]
    /// for the current option.
    pub fn store_os_string(&mut self, slot: &mut Option<OsString>) -> Result<(), Error> {
        if slot.is_some() {
            return Err(self.unexpected_current_argument());
        }

        *slot = Some(self.os_string()?);
        Ok(())
    }

    /// Stores the required value for the current option into a [`PathBuf`] slot that must be set at most once.
    ///
    /// This is useful for single-occurrence options such as `--root DIR`.
    pub fn store_path_buf(&mut self, slot: &mut Option<PathBuf>) -> Result<(), Error> {
        if slot.is_some() {
            return Err(self.unexpected_current_argument());
        }

        *slot = Some(self.path_buf()?);
        Ok(())
    }

    /// Stores the required value for the current option into a UTF-8 [`String`] slot that must be set at most once.
    ///
    /// This is useful for single-occurrence options such as `--bind ADDR`.
    pub fn store_string(&mut self, slot: &mut Option<String>) -> Result<(), Error> {
        if slot.is_some() {
            return Err(self.unexpected_current_argument());
        }

        *slot = Some(self.string_owned()?);
        Ok(())
    }

    /// Stores the parsed required value for the current option into a typed slot that must be set at most once.
    ///
    /// This is useful for single-occurrence options such as `--port PORT`.
    pub fn store_parse<T>(&mut self, slot: &mut Option<T>) -> Result<(), Error>
    where
        T: FromStr,
    {
        if slot.is_some() {
            return Err(self.unexpected_current_argument());
        }

        *slot = Some(self.parse::<T>()?);
        Ok(())
    }

    /// Splits the required value for the current option, requires a non-empty key,
    /// and stores the owned pair into a slot that must be set at most once.
    ///
    /// This is useful for single-occurrence options such as `--env KEY=VALUE`.
    pub fn store_split_once_nonempty_key_owned(
        &mut self,
        delimiter: char,
        slot: &mut Option<(String, String)>,
    ) -> Result<(), Error> {
        if slot.is_some() {
            return Err(self.unexpected_current_argument());
        }

        *slot = Some(self.split_once_nonempty_key_owned(delimiter)?);
        Ok(())
    }

    /// Splits the required value for the current option, requires a non-empty key,
    /// parses the right-hand side, and stores the pair into a slot that must be
    /// set at most once.
    ///
    /// This is useful for single-occurrence options such as `--port-map HTTP=8080`.
    pub fn store_split_once_nonempty_key_parse<T>(
        &mut self,
        delimiter: char,
        slot: &mut Option<(String, T)>,
    ) -> Result<(), Error>
    where
        T: FromStr,
    {
        if slot.is_some() {
            return Err(self.unexpected_current_argument());
        }

        let (key, value) = self.split_once_nonempty_key_parse(delimiter)?;
        *slot = Some((key.to_owned(), value));
        Ok(())
    }

    /// Pushes the required value for the current option into a repeated OS-native collector.
    ///
    /// This is shorthand for `parser.value()?.push_os_string(values)`.
    pub fn push_os_string(&mut self, values: &mut Vec<OsString>) -> Result<(), Error> {
        self.value()?.push_os_string(values);
        Ok(())
    }

    /// Pushes the required value for the current option into a repeated path collector.
    ///
    /// This is shorthand for `parser.value()?.push_path_buf(values)`.
    ///
    /// ```rust
    /// use osarg::{Arg, Parser};
    /// use std::path::PathBuf;
    ///
    /// let mut parser = Parser::from_args(["--include", "include", "--include", "generated"]);
    /// let mut includes = Vec::new();
    ///
    /// assert_eq!(parser.next()?, Some(Arg::Long("include")));
    /// parser.push_path_buf(&mut includes)?;
    /// assert_eq!(parser.next()?, Some(Arg::Long("include")));
    /// parser.push_path_buf(&mut includes)?;
    ///
    /// assert_eq!(includes, vec![PathBuf::from("include"), PathBuf::from("generated")]);
    /// # Ok::<(), osarg::Error>(())
    /// ```
    pub fn push_path_buf(&mut self, values: &mut Vec<PathBuf>) -> Result<(), Error> {
        self.value()?.push_path_buf(values);
        Ok(())
    }

    /// Pushes the required value for the current option into a repeated UTF-8 collector.
    ///
    /// This is shorthand for `parser.value()?.push_string(values)`.
    pub fn push_string(&mut self, values: &mut Vec<String>) -> Result<(), Error> {
        self.value()?.push_string(values)
    }

    /// Parses the required value for the current option and pushes it into a repeated typed collector.
    ///
    /// This is shorthand for `parser.value()?.push_parse(values)`.
    pub fn push_parse<T>(&mut self, values: &mut Vec<T>) -> Result<(), Error>
    where
        T: FromStr,
    {
        self.value()?.push_parse(values)
    }

    /// Splits the required value for the current option, requires a non-empty key,
    /// and pushes the owned pair into a repeated collector.
    ///
    /// This is shorthand for
    /// `parser.value()?.push_split_once_nonempty_key_owned(delimiter, values)`.
    ///
    /// ```rust
    /// use osarg::{Arg, Parser};
    ///
    /// let mut parser = Parser::from_args(["--env", "RUST_LOG=debug", "--env", "MODE=release"]);
    /// let mut envs = Vec::new();
    ///
    /// assert_eq!(parser.next()?, Some(Arg::Long("env")));
    /// parser.push_split_once_nonempty_key_owned('=', &mut envs)?;
    /// assert_eq!(parser.next()?, Some(Arg::Long("env")));
    /// parser.push_split_once_nonempty_key_owned('=', &mut envs)?;
    ///
    /// assert_eq!(
    ///     envs,
    ///     vec![
    ///         (String::from("RUST_LOG"), String::from("debug")),
    ///         (String::from("MODE"), String::from("release")),
    ///     ]
    /// );
    /// # Ok::<(), osarg::Error>(())
    /// ```
    pub fn push_split_once_nonempty_key_owned(
        &mut self,
        delimiter: char,
        values: &mut Vec<(String, String)>,
    ) -> Result<(), Error> {
        self.value()?
            .push_split_once_nonempty_key_owned(delimiter, values)
    }

    /// Splits the required value for the current option, requires a non-empty key,
    /// parses the right-hand side, and pushes the pair into a repeated collector.
    ///
    /// This is shorthand for
    /// `parser.value()?.push_split_once_nonempty_key_parse(delimiter, values)`.
    pub fn push_split_once_nonempty_key_parse<T>(
        &mut self,
        delimiter: char,
        values: &mut Vec<(String, T)>,
    ) -> Result<(), Error>
    where
        T: FromStr,
    {
        self.value()?
            .push_split_once_nonempty_key_parse(delimiter, values)
    }

    /// Returns an optional value for the current option.
    ///
    /// Attached values are consumed first. For separated values such as
    /// `--color auto`, the next token is only consumed when it does not look
    /// like another option.
    ///
    /// This keeps optional values predictable without requiring a schema.
    ///
    /// ```rust
    /// use osarg::{Arg, Parser};
    ///
    /// let mut parser = Parser::from_args(["--color", "--help"]);
    ///
    /// assert_eq!(parser.next()?, Some(Arg::Long("color")));
    /// assert_eq!(parser.value_opt()?.map(|value| value.to_str()), None);
    /// assert_eq!(parser.next()?, Some(Arg::Long("help")));
    /// # Ok::<(), osarg::Error>(())
    /// ```
    #[must_use = "callers must handle whether an optional value was present"]
    pub fn value_opt(&mut self) -> Result<Option<Value<'_>>, Error> {
        match self.value_source.clone() {
            ValueSource::Attached(range) => {
                self.value_source = ValueSource::Consumed;
                Ok(Some(Value::new(slice_os_str(self.current_os_str(), range))))
            }
            ValueSource::ShortTail(range) => {
                self.pending_shorts = None;
                self.value_source = ValueSource::Consumed;
                Ok(Some(Value::new(slice_os_str(self.current_os_str(), range))))
            }
            ValueSource::NextArgument => {
                let Some(_) = self.peek_or_pull_next() else {
                    self.value_source = ValueSource::Consumed;
                    return Ok(None);
                };

                if self
                    .peeked
                    .as_deref()
                    .is_some_and(|candidate| looks_like_option(candidate) && !self.options_done)
                {
                    self.value_source = ValueSource::Consumed;
                    return Ok(None);
                }

                let value = self.take_next_raw().expect("peeked value available");
                self.value_source = ValueSource::Consumed;
                self.value_slot = Some(value);
                Ok(Some(Value::new(
                    self.value_slot.as_deref().expect("value slot set"),
                )))
            }
            ValueSource::None | ValueSource::Consumed => {
                Err(Error::value_unavailable(self.current.as_deref()))
            }
        }
    }

    /// Returns the optional value for the current option as an owned [`OsString`].
    ///
    /// This is shorthand for `parser.value_opt()?.map(Value::to_os_string)`.
    ///
    /// When the next token already looks like another option, this returns
    /// `Ok(None)` and leaves that token untouched.
    #[must_use = "callers must handle whether an optional owned OS string was present"]
    pub fn os_string_opt(&mut self) -> Result<Option<OsString>, Error> {
        Ok(self.value_opt()?.map(Value::to_os_string))
    }

    /// Returns the optional value for the current option as an owned [`OsString`],
    /// or the provided default when no value is present.
    ///
    /// This is shorthand for `parser.os_string_opt()?.unwrap_or(default)`.
    ///
    /// ```rust
    /// use osarg::{Arg, Parser};
    /// use std::ffi::OsString;
    ///
    /// let mut parser = Parser::from_args(["--path", "--help"]);
    ///
    /// assert_eq!(parser.next()?, Some(Arg::Long("path")));
    /// assert_eq!(
    ///     parser.os_string_opt_or(OsString::from("./fallback"))?,
    ///     OsString::from("./fallback")
    /// );
    /// assert_eq!(parser.next()?, Some(Arg::Long("help")));
    /// # Ok::<(), osarg::Error>(())
    /// ```
    #[must_use = "callers must use or propagate the owned OS string or default"]
    pub fn os_string_opt_or(&mut self, default: OsString) -> Result<OsString, Error> {
        Ok(self.os_string_opt()?.unwrap_or(default))
    }

    /// Returns the optional value for the current option as an owned [`OsString`],
    /// or [`Default::default`] when no value is present.
    ///
    /// This is shorthand for `parser.os_string_opt()?.unwrap_or_default()`.
    ///
    /// ```rust
    /// use osarg::{Arg, Parser};
    /// use std::ffi::OsString;
    ///
    /// let mut parser = Parser::from_args(["--path", "./data"]);
    ///
    /// assert_eq!(parser.next()?, Some(Arg::Long("path")));
    /// assert_eq!(parser.os_string_opt_or_default()?, OsString::from("./data"));
    /// # Ok::<(), osarg::Error>(())
    /// ```
    #[must_use = "callers must use or propagate the owned OS string or default"]
    pub fn os_string_opt_or_default(&mut self) -> Result<OsString, Error> {
        Ok(self.os_string_opt()?.unwrap_or_default())
    }

    /// Returns the optional value for the current option as an owned [`PathBuf`].
    ///
    /// This is shorthand for `parser.value_opt()?.map(Value::to_path_buf)`.
    ///
    /// When the next token already looks like another option, this returns
    /// `Ok(None)` and leaves that token untouched.
    #[must_use = "callers must handle whether an optional owned path was present"]
    pub fn path_buf_opt(&mut self) -> Result<Option<PathBuf>, Error> {
        Ok(self.value_opt()?.map(Value::to_path_buf))
    }

    /// Returns the optional value for the current option as an owned [`PathBuf`],
    /// or the provided default when no value is present.
    ///
    /// This is shorthand for `parser.path_buf_opt()?.unwrap_or(default)`.
    ///
    /// ```rust
    /// use osarg::{Arg, Parser};
    /// use std::path::PathBuf;
    ///
    /// let mut parser = Parser::from_args(["--path", "--help"]);
    ///
    /// assert_eq!(parser.next()?, Some(Arg::Long("path")));
    /// assert_eq!(
    ///     parser.path_buf_opt_or(PathBuf::from("./fallback"))?,
    ///     PathBuf::from("./fallback")
    /// );
    /// assert_eq!(parser.next()?, Some(Arg::Long("help")));
    /// # Ok::<(), osarg::Error>(())
    /// ```
    #[must_use = "callers must use or propagate the owned path or default"]
    pub fn path_buf_opt_or(&mut self, default: PathBuf) -> Result<PathBuf, Error> {
        Ok(self.path_buf_opt()?.unwrap_or(default))
    }

    /// Returns the optional value for the current option as an owned [`PathBuf`],
    /// or [`Default::default`] when no value is present.
    ///
    /// This is shorthand for `parser.path_buf_opt()?.unwrap_or_default()`.
    ///
    /// ```rust
    /// use osarg::{Arg, Parser};
    /// use std::path::PathBuf;
    ///
    /// let mut parser = Parser::from_args(["--path", "./data"]);
    ///
    /// assert_eq!(parser.next()?, Some(Arg::Long("path")));
    /// assert_eq!(parser.path_buf_opt_or_default()?, PathBuf::from("./data"));
    /// # Ok::<(), osarg::Error>(())
    /// ```
    #[must_use = "callers must use or propagate the owned path or default"]
    pub fn path_buf_opt_or_default(&mut self) -> Result<PathBuf, Error> {
        Ok(self.path_buf_opt()?.unwrap_or_default())
    }

    /// Returns the optional value for the current option as UTF-8.
    ///
    /// This is shorthand for
    /// `parser.value_opt()?.map(Value::to_str).transpose()`.
    ///
    /// When the next token already looks like another option, this returns
    /// `Ok(None)` and leaves that token untouched.
    #[must_use = "callers must handle whether an optional UTF-8 string was present"]
    pub fn string_opt(&mut self) -> Result<Option<&str>, Error> {
        self.value_opt()?.map(Value::to_str).transpose()
    }

    /// Returns the optional value for the current option as an owned UTF-8 [`String`].
    ///
    /// This is shorthand for
    /// `parser.value_opt()?.map(Value::to_owned_string).transpose()`.
    ///
    /// When the next token already looks like another option, this returns
    /// `Ok(None)` and leaves that token untouched.
    #[must_use = "callers must handle whether an optional owned UTF-8 string was present"]
    pub fn string_opt_owned(&mut self) -> Result<Option<String>, Error> {
        self.value_opt()?.map(Value::to_owned_string).transpose()
    }

    /// Returns the optional value for the current option as an owned UTF-8 [`String`],
    /// or the provided default when no value is present.
    ///
    /// This is shorthand for `parser.string_opt_owned()?.unwrap_or(default)`.
    ///
    /// ```rust
    /// use osarg::{Arg, Parser};
    ///
    /// let mut parser = Parser::from_args(["--color", "--help"]);
    ///
    /// assert_eq!(parser.next()?, Some(Arg::Long("color")));
    /// assert_eq!(parser.string_opt_or(String::from("auto"))?, "auto");
    /// assert_eq!(parser.next()?, Some(Arg::Long("help")));
    /// # Ok::<(), osarg::Error>(())
    /// ```
    #[must_use = "callers must use or propagate the owned UTF-8 string or default"]
    pub fn string_opt_or(&mut self, default: String) -> Result<String, Error> {
        Ok(self.string_opt_owned()?.unwrap_or(default))
    }

    /// Returns the optional value for the current option as an owned UTF-8 [`String`],
    /// or [`Default::default`] when no value is present.
    ///
    /// This is shorthand for `parser.string_opt_owned()?.unwrap_or_default()`.
    ///
    /// ```rust
    /// use osarg::{Arg, Parser};
    ///
    /// let mut parser = Parser::from_args(["--color", "always"]);
    ///
    /// assert_eq!(parser.next()?, Some(Arg::Long("color")));
    /// assert_eq!(parser.string_opt_or_default()?, "always");
    /// # Ok::<(), osarg::Error>(())
    /// ```
    #[must_use = "callers must use or propagate the owned UTF-8 string or default"]
    pub fn string_opt_or_default(&mut self) -> Result<String, Error> {
        Ok(self.string_opt_owned()?.unwrap_or_default())
    }

    /// Splits the required value for the current option around the first delimiter.
    ///
    /// This is shorthand for `parser.value()?.split_once_required(delimiter)`.
    ///
    /// ```rust
    /// use osarg::{Arg, Parser};
    ///
    /// let mut parser = Parser::from_args(["--define", "KEY=VALUE"]);
    ///
    /// assert_eq!(parser.next()?, Some(Arg::Long("define")));
    /// assert_eq!(parser.split_once_required('=')?, ("KEY", "VALUE"));
    /// # Ok::<(), osarg::Error>(())
    /// ```
    #[must_use = "callers must use or propagate the split result"]
    pub fn split_once_required(&mut self, delimiter: char) -> Result<(&str, &str), Error> {
        self.value()?.split_once_required(delimiter)
    }

    /// Splits the required value for the current option, requiring a non-empty key.
    ///
    /// This is shorthand for
    /// `parser.value()?.split_once_nonempty_key(delimiter)`.
    #[must_use = "callers must use or propagate the validated split result"]
    pub fn split_once_nonempty_key(&mut self, delimiter: char) -> Result<(&str, &str), Error> {
        self.value()?.split_once_nonempty_key(delimiter)
    }

    /// Splits the required value for the current option, requiring a non-empty key,
    /// and returns owned UTF-8 strings.
    ///
    /// This is shorthand for
    /// `parser.value()?.split_once_nonempty_key_owned(delimiter)`.
    ///
    /// ```rust
    /// use osarg::{Arg, Parser};
    ///
    /// let mut parser = Parser::from_args(["--define", "KEY=VALUE"]);
    ///
    /// assert_eq!(parser.next()?, Some(Arg::Long("define")));
    /// assert_eq!(
    ///     parser.split_once_nonempty_key_owned('=')?,
    ///     (String::from("KEY"), String::from("VALUE"))
    /// );
    /// # Ok::<(), osarg::Error>(())
    /// ```
    #[must_use = "callers must use or propagate the owned validated split result"]
    pub fn split_once_nonempty_key_owned(
        &mut self,
        delimiter: char,
    ) -> Result<(String, String), Error> {
        self.value()?.split_once_nonempty_key_owned(delimiter)
    }

    /// Splits the required value for the current option and parses the right-hand side.
    ///
    /// This is shorthand for `parser.value()?.split_once_parse_value(delimiter)`.
    #[must_use = "callers must use or propagate the split and parse result"]
    pub fn split_once_parse_value<T>(&mut self, delimiter: char) -> Result<(&str, T), Error>
    where
        T: FromStr,
    {
        self.value()?.split_once_parse_value(delimiter)
    }

    /// Splits the required value for the current option, requires a non-empty key,
    /// and parses the right-hand side.
    ///
    /// This is shorthand for
    /// `parser.value()?.split_once_nonempty_key_parse(delimiter)`.
    #[must_use = "callers must use or propagate the validated split and parse result"]
    pub fn split_once_nonempty_key_parse<T>(&mut self, delimiter: char) -> Result<(&str, T), Error>
    where
        T: FromStr,
    {
        self.value()?.split_once_nonempty_key_parse(delimiter)
    }

    /// Parses the required value for the current option using [`FromStr`].
    ///
    /// This is shorthand for `parser.value()?.parse::<T>()`.
    ///
    /// ```rust
    /// use osarg::{Arg, Parser};
    ///
    /// let mut parser = Parser::from_args(["--port", "8080"]);
    ///
    /// assert_eq!(parser.next()?, Some(Arg::Long("port")));
    /// assert_eq!(parser.parse::<u16>()?, 8080);
    /// # Ok::<(), osarg::Error>(())
    /// ```
    #[must_use = "callers must use or propagate the typed parse result"]
    pub fn parse<T>(&mut self) -> Result<T, Error>
    where
        T: FromStr,
    {
        self.value()?.parse()
    }

    /// Parses the optional value for the current option using [`FromStr`].
    ///
    /// This is shorthand for `parser.value_opt()?.map(Value::parse).transpose()`.
    ///
    /// ```rust
    /// use osarg::{Arg, Parser};
    ///
    /// let mut parser = Parser::from_args(["--color", "--help"]);
    ///
    /// assert_eq!(parser.next()?, Some(Arg::Long("color")));
    /// assert_eq!(parser.parse_opt::<u8>()?, None);
    /// assert_eq!(parser.next()?, Some(Arg::Long("help")));
    /// # Ok::<(), osarg::Error>(())
    /// ```
    #[must_use = "callers must handle whether an optional typed value was present"]
    pub fn parse_opt<T>(&mut self) -> Result<Option<T>, Error>
    where
        T: FromStr,
    {
        self.value_opt()?.map(Value::parse).transpose()
    }

    /// Parses the optional value for the current option or returns the provided default.
    ///
    /// This is shorthand for `parser.parse_opt()?.unwrap_or(default)`.
    ///
    /// ```rust
    /// use osarg::{Arg, Parser};
    ///
    /// let mut parser = Parser::from_args(["--color", "--help"]);
    ///
    /// assert_eq!(parser.next()?, Some(Arg::Long("color")));
    /// assert_eq!(parser.parse_opt_or::<u8>(7)?, 7);
    /// assert_eq!(parser.next()?, Some(Arg::Long("help")));
    /// # Ok::<(), osarg::Error>(())
    /// ```
    #[must_use = "callers must use or propagate the parsed value or default"]
    pub fn parse_opt_or<T>(&mut self, default: T) -> Result<T, Error>
    where
        T: FromStr,
    {
        Ok(self.parse_opt::<T>()?.unwrap_or(default))
    }

    /// Parses the optional value for the current option or returns [`Default::default`].
    ///
    /// This is shorthand for `parser.parse_opt()?.unwrap_or_default()`.
    ///
    /// ```rust
    /// use osarg::{Arg, Parser};
    ///
    /// #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    /// enum Color {
    ///     Always,
    ///     #[default]
    ///     Auto,
    /// }
    ///
    /// impl core::str::FromStr for Color {
    ///     type Err = ();
    ///
    ///     fn from_str(value: &str) -> Result<Self, Self::Err> {
    ///         match value {
    ///             "always" => Ok(Self::Always),
    ///             "auto" => Ok(Self::Auto),
    ///             _ => Err(()),
    ///         }
    ///     }
    /// }
    ///
    /// let mut parser = Parser::from_args(["--color", "--help"]);
    ///
    /// assert_eq!(parser.next()?, Some(Arg::Long("color")));
    /// assert_eq!(parser.parse_opt_or_default::<Color>()?, Color::Auto);
    /// assert_eq!(parser.next()?, Some(Arg::Long("help")));
    /// # Ok::<(), osarg::Error>(())
    /// ```
    #[must_use = "callers must use or propagate the parsed value or default"]
    pub fn parse_opt_or_default<T>(&mut self) -> Result<T, Error>
    where
        T: FromStr + Default,
    {
        Ok(self.parse_opt::<T>()?.unwrap_or_default())
    }

    /// Consumes the parser and returns the remaining raw OS arguments.
    ///
    /// This is intended for wrapper and passthrough tools that need to stop
    /// parsing and forward the rest of the command line.
    ///
    /// If parsing stopped in the middle of a grouped short option token,
    /// the first remaining token is reconstructed so that it preserves the
    /// parser's future view. For example, after consuming `-a` from `-abc`,
    /// the remaining iterator starts with `-bc`.
    ///
    /// ```rust
    /// use osarg::{Arg, Parser};
    ///
    /// let mut parser = Parser::from_args([
    ///     "--env",
    ///     "RUST_LOG=debug",
    ///     "cargo",
    ///     "test",
    ///     "--",
    ///     "--nocapture",
    /// ]);
    ///
    /// assert_eq!(parser.next()?, Some(Arg::Long("env")));
    /// assert_eq!(parser.value()?.to_str()?, "RUST_LOG=debug");
    ///
    /// let command = match parser.next()? {
    ///     Some(Arg::Value(value)) => value.to_os_string(),
    ///     other => panic!("unexpected argument: {other:?}"),
    /// };
    ///
    /// let forwarded = parser.into_remaining().collect::<Vec<_>>();
    ///
    /// assert_eq!(command, std::ffi::OsString::from("cargo"));
    /// assert_eq!(
    ///     forwarded,
    ///     vec![
    ///         std::ffi::OsString::from("test"),
    ///         std::ffi::OsString::from("--"),
    ///         std::ffi::OsString::from("--nocapture"),
    ///     ]
    /// );
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use = "the remaining arguments are returned from this method"]
    pub fn into_remaining(self) -> Remaining<I> {
        Remaining {
            front: self.remaining_front(),
            peeked: self.peeked,
            iter: self.iter,
        }
    }

    /// Consumes the current positional value and returns it together with the remaining raw arguments.
    ///
    /// This is a thin convenience for passthrough and wrapper CLIs that treat
    /// the first positional as a command and forward the rest unchanged.
    ///
    /// It must be called immediately after [`Parser::next`] returned an
    /// [`Arg::Value`]. Calling it for an option returns
    /// [`crate::ErrorKind::ValueUnavailable`].
    ///
    /// ```rust
    /// use osarg::{Arg, Parser};
    ///
    /// let mut parser = Parser::from_args(["cargo", "test", "--", "--nocapture"]);
    ///
    /// assert!(matches!(parser.next()?, Some(Arg::Value(_))));
    ///
    /// let (command, forwarded) = parser.current_value_and_remaining()?;
    ///
    /// assert_eq!(command, std::ffi::OsString::from("cargo"));
    /// assert_eq!(
    ///     forwarded,
    ///     vec![
    ///         std::ffi::OsString::from("test"),
    ///         std::ffi::OsString::from("--"),
    ///         std::ffi::OsString::from("--nocapture"),
    ///     ]
    /// );
    /// # Ok::<(), osarg::Error>(())
    /// ```
    #[must_use = "the current positional value and remaining arguments are returned from this method"]
    pub fn current_value_and_remaining(mut self) -> Result<(OsString, Vec<OsString>), Error> {
        let Some(current) = self.current.take() else {
            return Err(Error::value_unavailable_for("<current argument>".into()));
        };

        if !self.options_done && looks_like_option(&current) {
            return Err(Error::value_unavailable_for(current));
        }

        let remaining = self.remaining_vec();
        Ok((current, remaining))
    }

    /// Consumes the parser and collects the remaining raw OS arguments.
    ///
    /// This is a convenience wrapper over [`Parser::into_remaining`].
    #[must_use = "the collected remaining arguments are returned from this method"]
    pub fn remaining_vec(self) -> Vec<OsString> {
        self.into_remaining().collect()
    }

    fn emit_short(&mut self) -> Result<Arg<'_>, Error> {
        let pending = self.pending_shorts.expect("pending short state present");
        let (byte, has_more) = {
            let bytes = self.current_os_str().as_encoded_bytes();
            let byte = *bytes
                .get(pending.next_index - 1)
                .expect("pending short index is in bounds");
            let has_more = pending.next_index < bytes.len();
            (byte, has_more)
        };

        if !byte.is_ascii() {
            return Err(self.invalid_option_name());
        }

        if has_more {
            let bytes_len = self.current_os_str().as_encoded_bytes().len();
            self.pending_shorts = Some(PendingShorts {
                next_index: pending.next_index + 1,
            });
            self.value_source = ValueSource::ShortTail(pending.next_index..bytes_len);
        } else {
            self.pending_shorts = None;
            self.value_source = ValueSource::NextArgument;
        }

        Ok(Arg::Short(byte as char))
    }

    fn current_os_str(&self) -> &OsStr {
        self.current
            .as_deref()
            .expect("current argument is present")
    }

    fn invalid_option_name(&mut self) -> Error {
        self.pending_shorts = None;
        self.value_source = ValueSource::None;
        self.current.take().map_or_else(
            || Error::without_argument(ErrorKind::InvalidOptionName),
            Error::invalid_option_name,
        )
    }

    fn unexpected_current_argument(&self) -> Error {
        self.current.as_deref().map_or_else(
            || Error::without_argument(ErrorKind::UnexpectedArgument),
            |argument| Error::unexpected_argument(argument.to_os_string()),
        )
    }

    fn take_next_raw(&mut self) -> Option<OsString> {
        self.peeked.take().or_else(|| self.iter.next())
    }

    fn peek_or_pull_next(&mut self) -> Option<&OsStr> {
        if self.peeked.is_none() {
            self.peeked = self.iter.next();
        }

        self.peeked.as_deref()
    }

    fn remaining_front(&self) -> Option<OsString> {
        let pending = self.pending_shorts?;
        let current = self.current.as_deref()?;
        let bytes = current.as_encoded_bytes();
        let start = pending.next_index.checked_sub(1)?;
        let tail = bytes.get(start..).filter(|tail| !tail.is_empty())?;

        let mut rebuilt = Vec::with_capacity(1 + tail.len());
        rebuilt.push(b'-');
        rebuilt.extend_from_slice(tail);

        // SAFETY: `rebuilt` is made of the original `OsStr` encoded bytes plus
        // a leading ASCII `-`, which preserves the platform encoding contract.
        Some(unsafe { OsString::from_encoded_bytes_unchecked(rebuilt) })
    }
}

impl<I> Iterator for Remaining<I>
where
    I: Iterator<Item = OsString>,
{
    type Item = OsString;

    fn next(&mut self) -> Option<Self::Item> {
        self.front
            .take()
            .or_else(|| self.peeked.take())
            .or_else(|| self.iter.next())
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let queued = usize::from(self.front.is_some()) + usize::from(self.peeked.is_some());
        let (lower, upper) = self.iter.size_hint();

        (
            lower.saturating_add(queued),
            upper.and_then(|upper| upper.checked_add(queued)),
        )
    }
}

impl<I> ExactSizeIterator for Remaining<I>
where
    I: ExactSizeIterator<Item = OsString>,
{
    fn len(&self) -> usize {
        usize::from(self.front.is_some()) + usize::from(self.peeked.is_some()) + self.iter.len()
    }
}

impl<I> FusedIterator for Remaining<I> where I: FusedIterator<Item = OsString> {}

fn looks_like_option(value: &OsStr) -> bool {
    let bytes = value.as_encoded_bytes();
    bytes.len() > 1 && bytes[0] == b'-'
}

fn slice_os_str(value: &OsStr, range: Range<usize>) -> &OsStr {
    let bytes = value.as_encoded_bytes();
    let slice = &bytes[range];

    // SAFETY: `slice` comes from splitting an existing `OsStr` on ASCII byte boundaries.
    unsafe { OsStr::from_encoded_bytes_unchecked(slice) }
}
