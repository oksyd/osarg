use crate::{Arg, Error, ErrorKind, Parser, help, standard};
use std::ffi::OsString;
use std::path::PathBuf;

const _: Option<char> = Arg::Short('h').as_short();
const _: Option<&str> = Arg::Long("help").as_long();
const _: bool = Arg::Short('h').is_short('h');
const _: &str = standard::text(standard::Flag::Help, "HELP", "VERSION");

fn parser(args: &[&str]) -> Parser<std::vec::IntoIter<OsString>> {
    Parser::from_args(args.iter().copied())
}

#[test]
fn from_args_accepts_borrowed_and_owned_inputs() {
    let mut borrowed = Parser::from_args(["--port", "8080"]);
    assert_eq!(borrowed.next().unwrap(), Some(Arg::Long("port")));
    assert_eq!(borrowed.parse::<u16>().unwrap(), 8080);

    let mut owned = Parser::from_args(vec![OsString::from("--path"), OsString::from("./data")]);
    assert_eq!(owned.next().unwrap(), Some(Arg::Long("path")));
    assert_eq!(owned.os_string().unwrap(), OsString::from("./data"));
}

#[test]
fn parses_short_long_and_positional_arguments() {
    let mut parser = parser(&["-v", "--port", "8080", "app"]);

    assert_eq!(parser.next().unwrap(), Some(Arg::Short('v')));
    assert_eq!(parser.next().unwrap(), Some(Arg::Long("port")));
    assert_eq!(parser.value().unwrap().parse::<u16>().unwrap(), 8080);

    match parser.next().unwrap() {
        Some(Arg::Value(value)) => assert_eq!(value.to_str().unwrap(), "app"),
        other => panic!("unexpected argument: {other:?}"),
    }

    assert_eq!(parser.next().unwrap(), None);
}

#[test]
fn parses_grouped_short_options_one_by_one() {
    let mut parser = parser(&["-abc"]);

    assert_eq!(parser.next().unwrap(), Some(Arg::Short('a')));
    assert_eq!(parser.next().unwrap(), Some(Arg::Short('b')));
    assert_eq!(parser.next().unwrap(), Some(Arg::Short('c')));
    assert_eq!(parser.next().unwrap(), None);
}

#[test]
fn parses_long_option_with_attached_value() {
    let mut parser = parser(&["--port=8080"]);

    assert_eq!(parser.next().unwrap(), Some(Arg::Long("port")));
    assert_eq!(parser.value().unwrap().parse::<u16>().unwrap(), 8080);
    assert_eq!(parser.next().unwrap(), None);
}

#[test]
fn long_option_can_use_empty_attached_value() {
    let mut parser = parser(&["--color="]);

    assert_eq!(parser.next().unwrap(), Some(Arg::Long("color")));
    assert_eq!(parser.value().unwrap().to_str().unwrap(), "");
    assert_eq!(parser.next().unwrap(), None);
}

#[test]
fn last_short_in_group_can_consume_following_value() {
    let mut parser = parser(&["-vp", "8080"]);

    assert_eq!(parser.next().unwrap(), Some(Arg::Short('v')));
    assert_eq!(parser.next().unwrap(), Some(Arg::Short('p')));
    assert_eq!(parser.value().unwrap().parse::<u16>().unwrap(), 8080);
    assert_eq!(parser.next().unwrap(), None);
}

#[test]
fn short_option_can_use_attached_tail_as_value() {
    let mut parser = parser(&["-p8080"]);

    assert_eq!(parser.next().unwrap(), Some(Arg::Short('p')));
    assert_eq!(parser.value().unwrap().parse::<u16>().unwrap(), 8080);
    assert_eq!(parser.next().unwrap(), None);
}

#[test]
fn last_short_in_cluster_can_use_attached_tail_as_value() {
    let mut parser = parser(&["-vp8080"]);

    assert_eq!(parser.next().unwrap(), Some(Arg::Short('v')));
    assert_eq!(parser.next().unwrap(), Some(Arg::Short('p')));
    assert_eq!(parser.value().unwrap().parse::<u16>().unwrap(), 8080);
    assert_eq!(parser.next().unwrap(), None);
}

#[test]
fn short_tail_value_is_explicit_and_does_not_consume_the_next_argument() {
    let mut parser = parser(&["-pv", "8080"]);

    assert_eq!(parser.next().unwrap(), Some(Arg::Short('p')));
    assert_eq!(parser.value().unwrap().to_str().unwrap(), "v");

    match parser.next().unwrap() {
        Some(Arg::Value(value)) => assert_eq!(value.to_str().unwrap(), "8080"),
        other => panic!("unexpected argument: {other:?}"),
    }
}

#[test]
fn sentinel_stops_option_parsing() {
    let mut parser = parser(&["--", "--help"]);

    match parser.next().unwrap() {
        Some(Arg::Value(value)) => assert_eq!(value.to_str().unwrap(), "--help"),
        other => panic!("unexpected argument: {other:?}"),
    }

    assert_eq!(parser.next().unwrap(), None);
}

#[test]
fn value_opt_uses_attached_value() {
    let mut parser = parser(&["--color=auto"]);

    assert_eq!(parser.next().unwrap(), Some(Arg::Long("color")));
    assert_eq!(
        parser.value_opt().unwrap().unwrap().to_str().unwrap(),
        "auto"
    );
}

#[test]
fn value_opt_uses_short_tail() {
    let mut parser = parser(&["-Cauto"]);

    assert_eq!(parser.next().unwrap(), Some(Arg::Short('C')));
    assert_eq!(
        parser.value_opt().unwrap().unwrap().to_str().unwrap(),
        "auto"
    );
    assert_eq!(parser.next().unwrap(), None);
}

#[test]
fn value_opt_leaves_following_option_unconsumed() {
    let mut parser = parser(&["--color", "--help"]);

    assert_eq!(parser.next().unwrap(), Some(Arg::Long("color")));
    assert!(parser.value_opt().unwrap().is_none());
    assert_eq!(parser.next().unwrap(), Some(Arg::Long("help")));
}

#[test]
fn value_parse_reports_invalid_value() {
    let mut parser = parser(&["--port", "nope"]);

    assert_eq!(parser.next().unwrap(), Some(Arg::Long("port")));
    let error = parser.value().unwrap().parse::<u16>().unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidValue);
    assert_eq!(error.argument().unwrap().to_string_lossy(), "nope");
}

#[test]
fn empty_long_option_name_is_rejected() {
    let mut parser = parser(&["--=value"]);
    let error = parser.next().unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidOptionName);
    assert_eq!(
        error.to_string(),
        "option name is invalid or not valid UTF-8: --=value"
    );
}

#[test]
fn unexpected_helpers_build_structured_errors() {
    assert_eq!(
        Arg::Short('v').unexpected().kind(),
        ErrorKind::UnexpectedArgument
    );

    let mut parser = parser(&["path"]);
    let arg = parser.next().unwrap().unwrap();
    assert_eq!(arg.unexpected().kind(), ErrorKind::UnexpectedPositional);
}

#[test]
fn arg_helpers_match_standard_flags() {
    let help = Arg::Short('h');
    assert_eq!(help.as_short(), Some('h'));
    assert_eq!(help.as_long(), None);
    assert!(help.is_short('h'));
    assert!(help.matches('h', "help"));
    assert!(standard::is_help(help));
    assert!(!standard::is_version(help));
    assert_eq!(standard::classify(help), Some(standard::Flag::Help));
    assert!(standard::Flag::Help.matches(help));
    assert_eq!(standard::Flag::Help.short(), 'h');
    assert_eq!(standard::Flag::Help.long(), "help");

    let version = Arg::Long("version");
    assert_eq!(version.as_short(), None);
    assert_eq!(version.as_long(), Some("version"));
    assert!(version.is_long("version"));
    assert!(version.matches('V', "version"));
    assert!(standard::is_version(version));
    assert!(!standard::is_help(version));
    assert_eq!(standard::classify(version), Some(standard::Flag::Version));
    assert!(standard::Flag::Version.matches(version));
}

#[test]
fn standard_helpers_return_and_write_text() {
    assert_eq!(
        standard::text(standard::Flag::Help, "HELP", "VERSION"),
        "HELP"
    );
    assert_eq!(
        standard::text(standard::Flag::Version, "HELP", "VERSION"),
        "VERSION"
    );

    let mut output = Vec::new();
    standard::write(&mut output, standard::Flag::Version, "HELP", "VERSION").unwrap();
    assert_eq!(String::from_utf8(output).unwrap(), "VERSION");
}

#[test]
fn standard_try_write_handles_help_version_and_non_standard_args() {
    let sections = &[help::Section::new("options:", "  -h, --help  show help")];
    let doc = help::Help::new("demo [OPTIONS]", sections);

    let mut help_output = Vec::new();
    assert!(standard::try_write(&mut help_output, Arg::Short('h'), doc, "1.2.3").unwrap());
    assert!(
        String::from_utf8(help_output)
            .unwrap()
            .starts_with("usage: demo [OPTIONS]\n")
    );

    let mut version_output = Vec::new();
    assert!(standard::try_write(&mut version_output, Arg::Long("version"), doc, "1.2.3").unwrap());
    assert_eq!(String::from_utf8(version_output).unwrap(), "1.2.3");

    let mut none_output = Vec::new();
    assert!(!standard::try_write(&mut none_output, Arg::Short('p'), doc, "1.2.3").unwrap());
    assert!(none_output.is_empty());
}

#[test]
fn standard_try_print_returns_false_for_non_standard_args() {
    let doc = help::Help::new("demo [OPTIONS]", &[]);
    assert!(!standard::try_print(Arg::Short('p'), doc, "1.2.3").unwrap());
}

#[test]
fn standard_try_eprint_returns_false_for_non_standard_args() {
    let doc = help::Help::new("demo [OPTIONS]", &[]);
    assert!(!standard::try_eprint(Arg::Short('p'), doc, "1.2.3").unwrap());
}

#[test]
fn arg_and_value_display_are_human_readable() {
    assert_eq!(Arg::Short('p').to_string(), "-p");
    assert_eq!(Arg::Long("port").to_string(), "--port");

    let mut parser = parser(&["path"]);
    let arg = parser.next().unwrap().unwrap();
    assert_eq!(arg.to_string(), "path");
}

#[test]
fn help_module_writes_usage_and_sections() {
    let sections = &[
        help::Section::new(
            "options:",
            "  -h, --help       show help\n  -V, --version    show version",
        ),
        help::Section::new("notes:", "  caller-owned text"),
    ];
    let doc = help::Help::new("demo [OPTIONS] <PATH>", sections);

    let mut output = Vec::new();
    doc.write(&mut output).unwrap();

    assert_eq!(doc.usage(), "demo [OPTIONS] <PATH>");
    assert_eq!(doc.sections(), sections);
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "usage: demo [OPTIONS] <PATH>\n\noptions:\n  -h, --help       show help\n  -V, --version    show version\n\nnotes:\n  caller-owned text\n"
    );
}

#[test]
fn help_module_can_write_individual_sections() {
    let section = help::Section::new("positional arguments:", "  PATH    target path");
    let mut output = Vec::new();

    help::write_section(&mut output, section).unwrap();

    assert_eq!(section.heading(), "positional arguments:");
    assert_eq!(section.body(), "  PATH    target path");
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "positional arguments:\n  PATH    target path\n"
    );
}

#[test]
fn value_can_be_cloned_into_os_string() {
    let mut parser = parser(&["path"]);
    let arg = parser.next().unwrap().unwrap();

    match arg {
        Arg::Value(value) => assert_eq!(value.to_os_string(), OsString::from("path")),
        other => panic!("unexpected argument: {other:?}"),
    }
}

#[test]
fn value_can_store_os_string_once() {
    let mut parser = parser(&["path", "extra"]);
    let first = parser.next().unwrap().unwrap().as_value().unwrap();
    let mut slot = None;

    first.store_os_string(&mut slot).unwrap();
    let second = parser.next().unwrap().unwrap().as_value().unwrap();
    let error = second.store_os_string(&mut slot).unwrap_err();

    assert_eq!(slot, Some(OsString::from("path")));
    assert_eq!(error.kind(), ErrorKind::UnexpectedPositional);
    assert_eq!(error.argument().unwrap().to_string_lossy(), "extra");
}

#[test]
fn value_can_build_and_store_path_bufs() {
    let mut parser = parser(&["./data", "extra"]);
    let first = parser.next().unwrap().unwrap().as_value().unwrap();
    assert_eq!(first.to_path_buf(), PathBuf::from("./data"));

    let mut slot = None;
    first.store_path_buf(&mut slot).unwrap();

    let second = parser.next().unwrap().unwrap().as_value().unwrap();
    let error = second.store_path_buf(&mut slot).unwrap_err();

    assert_eq!(slot, Some(PathBuf::from("./data")));
    assert_eq!(error.kind(), ErrorKind::UnexpectedPositional);
    assert_eq!(error.argument().unwrap().to_string_lossy(), "extra");
}

#[test]
fn value_can_split_required_pairs() {
    let mut parser = parser(&["KEY=VALUE", "BROKEN"]);
    let value = parser.next().unwrap().unwrap().as_value().unwrap();
    assert_eq!(value.split_once_required('=').unwrap(), ("KEY", "VALUE"));

    let broken = parser.next().unwrap().unwrap().as_value().unwrap();
    let error = broken.split_once_required('=').unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidValue);
    assert_eq!(error.argument().unwrap().to_string_lossy(), "BROKEN");
}

#[test]
fn value_can_split_required_pairs_with_nonempty_keys() {
    let mut parser = parser(&["KEY=VALUE", "=VALUE"]);
    let value = parser.next().unwrap().unwrap().as_value().unwrap();
    assert_eq!(
        value.split_once_nonempty_key('=').unwrap(),
        ("KEY", "VALUE")
    );

    let empty_key = parser.next().unwrap().unwrap().as_value().unwrap();
    let error = empty_key.split_once_nonempty_key('=').unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidValue);
    assert_eq!(error.argument().unwrap().to_string_lossy(), "=VALUE");
}

#[test]
fn value_can_build_owned_utf8_strings_and_pairs() {
    let mut parser = parser(&["demo", "KEY=VALUE"]);

    let text = parser.next().unwrap().unwrap().as_value().unwrap();
    assert_eq!(text.to_owned_string().unwrap(), String::from("demo"));

    let pair = parser.next().unwrap().unwrap().as_value().unwrap();
    assert_eq!(
        pair.split_once_nonempty_key_owned('=').unwrap(),
        (String::from("KEY"), String::from("VALUE"))
    );
}

#[test]
fn value_can_split_and_parse_pairs() {
    let mut parser = parser(&["PORT=8080", "PORT=bad", "=8080"]);

    let value = parser.next().unwrap().unwrap().as_value().unwrap();
    assert_eq!(
        value.split_once_parse_value::<u16>('=').unwrap(),
        ("PORT", 8080)
    );

    let invalid = parser.next().unwrap().unwrap().as_value().unwrap();
    let invalid_error = invalid.split_once_parse_value::<u16>('=').unwrap_err();
    assert_eq!(invalid_error.kind(), ErrorKind::InvalidValue);
    assert_eq!(
        invalid_error.argument().unwrap().to_string_lossy(),
        "PORT=bad"
    );

    let empty_key = parser.next().unwrap().unwrap().as_value().unwrap();
    let empty_key_error = empty_key
        .split_once_nonempty_key_parse::<u16>('=')
        .unwrap_err();
    assert_eq!(empty_key_error.kind(), ErrorKind::InvalidValue);
    assert_eq!(
        empty_key_error.argument().unwrap().to_string_lossy(),
        "=8080"
    );
}

#[test]
fn value_can_store_utf8_string_once() {
    let mut parser = parser(&["demo", "extra"]);
    let first = parser.next().unwrap().unwrap().as_value().unwrap();
    let mut slot = None;

    first.store_string(&mut slot).unwrap();
    let second = parser.next().unwrap().unwrap().as_value().unwrap();
    let error = second.store_string(&mut slot).unwrap_err();

    assert_eq!(slot, Some(String::from("demo")));
    assert_eq!(error.kind(), ErrorKind::UnexpectedPositional);
    assert_eq!(error.argument().unwrap().to_string_lossy(), "extra");
}

#[test]
fn value_can_store_parsed_value_once() {
    let mut parser = parser(&["8080", "9090"]);
    let first = parser.next().unwrap().unwrap().as_value().unwrap();
    let mut slot = None;

    first.store_parse::<u16>(&mut slot).unwrap();
    let second = parser.next().unwrap().unwrap().as_value().unwrap();
    let error = second.store_parse::<u16>(&mut slot).unwrap_err();

    assert_eq!(slot, Some(8080));
    assert_eq!(error.kind(), ErrorKind::UnexpectedPositional);
    assert_eq!(error.argument().unwrap().to_string_lossy(), "9090");
}

#[test]
fn value_can_store_split_pairs_once() {
    let mut parser = parser(&["KEY=VALUE", "PORT=8080", "EXTRA=1"]);
    let mut pair = None;
    let mut typed_pair = None;

    parser
        .next()
        .unwrap()
        .unwrap()
        .as_value()
        .unwrap()
        .store_split_once_nonempty_key_owned('=', &mut pair)
        .unwrap();
    parser
        .next()
        .unwrap()
        .unwrap()
        .as_value()
        .unwrap()
        .store_split_once_nonempty_key_parse::<u16>('=', &mut typed_pair)
        .unwrap();

    let error = parser
        .next()
        .unwrap()
        .unwrap()
        .as_value()
        .unwrap()
        .store_split_once_nonempty_key_owned('=', &mut pair)
        .unwrap_err();

    assert_eq!(pair, Some((String::from("KEY"), String::from("VALUE"))));
    assert_eq!(typed_pair, Some((String::from("PORT"), 8080)));
    assert_eq!(error.kind(), ErrorKind::UnexpectedPositional);
    assert_eq!(error.argument().unwrap().to_string_lossy(), "EXTRA=1");
}

#[test]
fn value_can_push_into_repeated_collectors() {
    let mut parser = parser(&["include", "generated", "name", "8080", "9090"]);
    let mut includes = Vec::new();
    let mut include_paths = Vec::new();
    let mut names = Vec::new();
    let mut ports = Vec::new();

    parser
        .next()
        .unwrap()
        .unwrap()
        .as_value()
        .unwrap()
        .push_os_string(&mut includes);
    parser
        .next()
        .unwrap()
        .unwrap()
        .as_value()
        .unwrap()
        .push_path_buf(&mut include_paths);
    parser
        .next()
        .unwrap()
        .unwrap()
        .as_value()
        .unwrap()
        .push_string(&mut names)
        .unwrap();
    parser
        .next()
        .unwrap()
        .unwrap()
        .as_value()
        .unwrap()
        .push_parse::<u16>(&mut ports)
        .unwrap();
    parser
        .next()
        .unwrap()
        .unwrap()
        .as_value()
        .unwrap()
        .push_parse::<u16>(&mut ports)
        .unwrap();

    assert_eq!(includes, vec![OsString::from("include")]);
    assert_eq!(include_paths, vec![PathBuf::from("generated")]);
    assert_eq!(names, vec![String::from("name")]);
    assert_eq!(ports, vec![8080, 9090]);
}

#[test]
fn value_can_push_split_pairs_into_collectors() {
    let mut parser = parser(&["KEY=VALUE", "PORT=8080"]);
    let mut pairs = Vec::new();
    let mut typed_pairs = Vec::new();

    parser
        .next()
        .unwrap()
        .unwrap()
        .as_value()
        .unwrap()
        .push_split_once_nonempty_key_owned('=', &mut pairs)
        .unwrap();
    parser
        .next()
        .unwrap()
        .unwrap()
        .as_value()
        .unwrap()
        .push_split_once_nonempty_key_parse::<u16>('=', &mut typed_pairs)
        .unwrap();

    assert_eq!(pairs, vec![(String::from("KEY"), String::from("VALUE"))]);
    assert_eq!(typed_pairs, vec![(String::from("PORT"), 8080)]);
}

#[test]
fn value_push_collectors_report_invalid_inputs() {
    let mut parser = parser(&["bad"]);

    let error = parser
        .next()
        .unwrap()
        .unwrap()
        .as_value()
        .unwrap()
        .push_parse::<u16>(&mut Vec::new())
        .unwrap_err();

    assert_eq!(error.kind(), ErrorKind::InvalidValue);
    assert_eq!(error.argument().unwrap().to_string_lossy(), "bad");
}

#[test]
fn arg_can_extract_value_without_matching() {
    let mut parser = parser(&["path"]);
    let arg = parser.next().unwrap().unwrap();

    assert!(arg.is_value());
    assert_eq!(arg.as_value().unwrap().to_str().unwrap(), "path");
}

#[test]
fn value_supports_display_and_try_from() {
    let mut parser = parser(&["path"]);
    let arg = parser.next().unwrap().unwrap();
    let value = arg.as_value().unwrap();

    assert_eq!(value.to_string(), "path");
    assert_eq!(<&str>::try_from(value).unwrap(), "path");
    assert_eq!(OsString::from(value), OsString::from("path"));
}

#[test]
fn value_can_build_invalid_value_error() {
    let mut parser = parser(&["MODE"]);
    let arg = parser.next().unwrap().unwrap();
    let value = arg.as_value().unwrap();
    let error = value.invalid();

    assert_eq!(error.kind(), ErrorKind::InvalidValue);
    assert_eq!(error.argument().unwrap().to_string_lossy(), "MODE");
}

#[test]
fn public_error_constructors_are_structured() {
    let missing_argument = Error::missing_argument_for("<PATH>".into());
    assert_eq!(missing_argument.kind(), ErrorKind::MissingArgument);
    assert_eq!(
        missing_argument.to_string(),
        "missing required argument: <PATH>"
    );

    let missing = Error::missing_value_for("--port".into());
    assert_eq!(missing.kind(), ErrorKind::MissingValue);
    assert_eq!(missing.argument().unwrap().to_string_lossy(), "--port");

    let positional = Error::unexpected_positional("path".into());
    assert_eq!(positional.kind(), ErrorKind::UnexpectedPositional);
    assert_eq!(
        positional.to_string(),
        "unexpected positional argument: path"
    );

    let invalid_utf8 = Error::invalid_utf8("value".into());
    assert_eq!(invalid_utf8.kind(), ErrorKind::InvalidUtf8);

    let unavailable = Error::value_unavailable_for("--flag".into());
    assert_eq!(unavailable.kind(), ErrorKind::ValueUnavailable);
}

#[test]
fn required_helper_returns_value_and_reports_missing_argument() {
    assert_eq!(crate::required(Some(7u8), "<PORT>").unwrap(), 7);

    let error = crate::required::<u8, _>(None, "<PORT>").unwrap_err();
    assert_eq!(error.kind(), ErrorKind::MissingArgument);
    assert_eq!(error.argument().unwrap().to_string_lossy(), "<PORT>");
}

#[test]
fn count_flag_helper_saturates() {
    let mut verbose = u8::MAX - 1;

    crate::count_flag(&mut verbose);
    crate::count_flag(&mut verbose);
    crate::count_flag(&mut verbose);

    assert_eq!(verbose, u8::MAX);
}

#[test]
fn set_flag_helper_sets_true() {
    let mut dry_run = false;
    crate::set_flag(&mut dry_run);
    assert!(dry_run);
}

#[test]
fn parser_can_parse_typed_values_directly() {
    let mut parser = parser(&["--port", "8080"]);
    assert_eq!(parser.next().unwrap(), Some(Arg::Long("port")));
    assert_eq!(parser.parse::<u16>().unwrap(), 8080);
}

#[test]
fn parser_can_read_utf8_and_owned_os_values_directly() {
    let mut bind_parser = parser(&["--bind", "0.0.0.0"]);
    assert_eq!(bind_parser.next().unwrap(), Some(Arg::Long("bind")));
    assert_eq!(bind_parser.string().unwrap(), "0.0.0.0");
    assert_eq!(
        bind_parser.value().unwrap_err().kind(),
        ErrorKind::ValueUnavailable
    );

    let mut owned_bind_parser = parser(&["--bind", "0.0.0.0"]);
    assert_eq!(owned_bind_parser.next().unwrap(), Some(Arg::Long("bind")));
    assert_eq!(
        owned_bind_parser.string_owned().unwrap(),
        String::from("0.0.0.0")
    );

    let mut path_parser = parser(&["--path", "./data"]);
    assert_eq!(path_parser.next().unwrap(), Some(Arg::Long("path")));
    assert_eq!(path_parser.os_string().unwrap(), OsString::from("./data"));

    let mut path_buf_parser = parser(&["--path", "./data"]);
    assert_eq!(path_buf_parser.next().unwrap(), Some(Arg::Long("path")));
    assert_eq!(path_buf_parser.path_buf().unwrap(), PathBuf::from("./data"));
}

#[test]
fn parser_can_store_single_option_values_once() {
    let mut parser = parser(&[
        "--bind", "0.0.0.0", "--port", "8080", "--root", "./data", "--output", "artifact",
    ]);
    let mut bind = None;
    let mut port = None;
    let mut root = None;
    let mut output = None;

    assert_eq!(parser.next().unwrap(), Some(Arg::Long("bind")));
    parser.store_string(&mut bind).unwrap();
    assert_eq!(parser.next().unwrap(), Some(Arg::Long("port")));
    parser.store_parse::<u16>(&mut port).unwrap();
    assert_eq!(parser.next().unwrap(), Some(Arg::Long("root")));
    parser.store_path_buf(&mut root).unwrap();
    assert_eq!(parser.next().unwrap(), Some(Arg::Long("output")));
    parser.store_os_string(&mut output).unwrap();

    assert_eq!(bind, Some(String::from("0.0.0.0")));
    assert_eq!(port, Some(8080));
    assert_eq!(root, Some(PathBuf::from("./data")));
    assert_eq!(output, Some(OsString::from("artifact")));
}

#[test]
fn parser_can_store_single_pair_values_once() {
    let mut parser = parser(&["--env", "RUST_LOG=debug", "--port-map", "HTTP=8080"]);
    let mut env = None;
    let mut port_map = None;

    assert_eq!(parser.next().unwrap(), Some(Arg::Long("env")));
    parser
        .store_split_once_nonempty_key_owned('=', &mut env)
        .unwrap();
    assert_eq!(parser.next().unwrap(), Some(Arg::Long("port-map")));
    parser
        .store_split_once_nonempty_key_parse::<u16>('=', &mut port_map)
        .unwrap();

    assert_eq!(env, Some((String::from("RUST_LOG"), String::from("debug"))));
    assert_eq!(port_map, Some((String::from("HTTP"), 8080)));
}

#[test]
fn parser_can_store_single_flags_once() {
    let mut parser = parser(&["--dry-run"]);
    let mut dry_run = false;

    assert_eq!(parser.next().unwrap(), Some(Arg::Long("dry-run")));
    parser.store_flag(&mut dry_run).unwrap();
    assert!(dry_run);
}

#[test]
fn parser_store_flag_rejects_duplicate_flags() {
    let mut parser = parser(&["--dry-run", "--dry-run"]);
    let mut dry_run = false;

    assert_eq!(parser.next().unwrap(), Some(Arg::Long("dry-run")));
    parser.store_flag(&mut dry_run).unwrap();
    assert_eq!(parser.next().unwrap(), Some(Arg::Long("dry-run")));

    let error = parser.store_flag(&mut dry_run).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnexpectedArgument);
    assert_eq!(error.argument().unwrap().to_string_lossy(), "--dry-run");
}

#[test]
fn parser_store_helpers_reject_duplicate_options() {
    let mut parser = parser(&["--port", "8080", "--port", "9090"]);
    let mut port = None;

    assert_eq!(parser.next().unwrap(), Some(Arg::Long("port")));
    parser.store_parse::<u16>(&mut port).unwrap();
    assert_eq!(parser.next().unwrap(), Some(Arg::Long("port")));

    let error = parser.store_parse::<u16>(&mut port).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnexpectedArgument);
    assert_eq!(error.argument().unwrap().to_string_lossy(), "--port");
    assert_eq!(
        parser
            .next()
            .unwrap()
            .and_then(Arg::as_value)
            .unwrap()
            .to_str()
            .unwrap(),
        "9090"
    );
    assert_eq!(parser.next().unwrap(), None);
}

#[test]
fn parser_store_duplicate_does_not_consume_following_option() {
    let mut parser = parser(&["--port", "8080", "--port", "--help"]);
    let mut port = None;

    assert_eq!(parser.next().unwrap(), Some(Arg::Long("port")));
    parser.store_parse::<u16>(&mut port).unwrap();
    assert_eq!(parser.next().unwrap(), Some(Arg::Long("port")));

    assert_eq!(
        parser.store_parse::<u16>(&mut port).unwrap_err().kind(),
        ErrorKind::UnexpectedArgument
    );
    assert_eq!(parser.next().unwrap(), Some(Arg::Long("help")));
}

#[test]
fn parser_store_helpers_prefer_duplicate_error_when_duplicate_value_is_missing() {
    let mut parser = parser(&["--port", "8080", "--port"]);
    let mut port = None;

    assert_eq!(parser.next().unwrap(), Some(Arg::Long("port")));
    parser.store_parse::<u16>(&mut port).unwrap();
    assert_eq!(parser.next().unwrap(), Some(Arg::Long("port")));

    let error = parser.store_parse::<u16>(&mut port).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnexpectedArgument);
    assert_eq!(error.argument().unwrap().to_string_lossy(), "--port");
    assert_eq!(parser.next().unwrap(), None);
}

#[test]
fn parser_store_pair_helpers_reject_duplicate_options() {
    let mut parser = parser(&["--env", "RUST_LOG=debug", "--env", "MODE=release"]);
    let mut env = None;

    assert_eq!(parser.next().unwrap(), Some(Arg::Long("env")));
    parser
        .store_split_once_nonempty_key_owned('=', &mut env)
        .unwrap();
    assert_eq!(parser.next().unwrap(), Some(Arg::Long("env")));

    let error = parser
        .store_split_once_nonempty_key_owned('=', &mut env)
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnexpectedArgument);
    assert_eq!(error.argument().unwrap().to_string_lossy(), "--env");
}

#[test]
fn parser_can_parse_optional_typed_values() {
    let mut color_parser = parser(&["--color", "7"]);
    assert_eq!(color_parser.next().unwrap(), Some(Arg::Long("color")));
    assert_eq!(color_parser.parse_opt::<u8>().unwrap(), Some(7));

    let mut missing_parser = parser(&["--color", "--help"]);
    assert_eq!(missing_parser.next().unwrap(), Some(Arg::Long("color")));
    assert_eq!(missing_parser.parse_opt::<u8>().unwrap(), None);
    assert_eq!(missing_parser.next().unwrap(), Some(Arg::Long("help")));
}

#[test]
fn parser_can_parse_optional_values_with_defaults() {
    let mut missing_parser = parser(&["--retries", "--help"]);
    assert_eq!(missing_parser.next().unwrap(), Some(Arg::Long("retries")));
    assert_eq!(missing_parser.parse_opt_or::<u8>(3).unwrap(), 3);
    assert_eq!(missing_parser.next().unwrap(), Some(Arg::Long("help")));

    let mut present_parser = parser(&["--retries", "7"]);
    assert_eq!(present_parser.next().unwrap(), Some(Arg::Long("retries")));
    assert_eq!(present_parser.parse_opt_or::<u8>(3).unwrap(), 7);
}

#[test]
fn parser_can_parse_optional_values_with_default_trait() {
    let mut missing_parser = parser(&["--count", "--help"]);
    assert_eq!(missing_parser.next().unwrap(), Some(Arg::Long("count")));
    assert_eq!(missing_parser.parse_opt_or_default::<u8>().unwrap(), 0);
    assert_eq!(missing_parser.next().unwrap(), Some(Arg::Long("help")));

    let mut present_parser = parser(&["--count", "9"]);
    assert_eq!(present_parser.next().unwrap(), Some(Arg::Long("count")));
    assert_eq!(present_parser.parse_opt_or_default::<u8>().unwrap(), 9);
}

#[test]
fn parser_can_read_optional_owned_values_with_defaults() {
    let mut missing_os_parser = parser(&["--path", "--help"]);
    assert_eq!(missing_os_parser.next().unwrap(), Some(Arg::Long("path")));
    assert_eq!(
        missing_os_parser
            .os_string_opt_or(OsString::from("./fallback"))
            .unwrap(),
        OsString::from("./fallback")
    );
    assert_eq!(missing_os_parser.next().unwrap(), Some(Arg::Long("help")));

    let mut missing_os_default_parser = parser(&["--path", "--help"]);
    assert_eq!(
        missing_os_default_parser.next().unwrap(),
        Some(Arg::Long("path"))
    );
    assert_eq!(
        missing_os_default_parser
            .os_string_opt_or_default()
            .unwrap(),
        OsString::new()
    );
    assert_eq!(
        missing_os_default_parser.next().unwrap(),
        Some(Arg::Long("help"))
    );

    let mut present_os_parser = parser(&["--path", "./data"]);
    assert_eq!(present_os_parser.next().unwrap(), Some(Arg::Long("path")));
    assert_eq!(
        present_os_parser
            .os_string_opt_or(OsString::from("./fallback"))
            .unwrap(),
        OsString::from("./data")
    );

    let mut present_os_default_parser = parser(&["--path", "./data"]);
    assert_eq!(
        present_os_default_parser.next().unwrap(),
        Some(Arg::Long("path"))
    );
    assert_eq!(
        present_os_default_parser
            .os_string_opt_or_default()
            .unwrap(),
        OsString::from("./data")
    );

    let mut missing_path_parser = parser(&["--path", "--help"]);
    assert_eq!(missing_path_parser.next().unwrap(), Some(Arg::Long("path")));
    assert_eq!(
        missing_path_parser
            .path_buf_opt_or(PathBuf::from("./fallback"))
            .unwrap(),
        PathBuf::from("./fallback")
    );
    assert_eq!(missing_path_parser.next().unwrap(), Some(Arg::Long("help")));

    let mut missing_path_default_parser = parser(&["--path", "--help"]);
    assert_eq!(
        missing_path_default_parser.next().unwrap(),
        Some(Arg::Long("path"))
    );
    assert_eq!(
        missing_path_default_parser
            .path_buf_opt_or_default()
            .unwrap(),
        PathBuf::new()
    );
    assert_eq!(
        missing_path_default_parser.next().unwrap(),
        Some(Arg::Long("help"))
    );

    let mut present_path_parser = parser(&["--path", "./data"]);
    assert_eq!(present_path_parser.next().unwrap(), Some(Arg::Long("path")));
    assert_eq!(
        present_path_parser
            .path_buf_opt_or(PathBuf::from("./fallback"))
            .unwrap(),
        PathBuf::from("./data")
    );

    let mut present_path_default_parser = parser(&["--path", "./data"]);
    assert_eq!(
        present_path_default_parser.next().unwrap(),
        Some(Arg::Long("path"))
    );
    assert_eq!(
        present_path_default_parser
            .path_buf_opt_or_default()
            .unwrap(),
        PathBuf::from("./data")
    );

    let mut missing_string_parser = parser(&["--color", "--help"]);
    assert_eq!(
        missing_string_parser.next().unwrap(),
        Some(Arg::Long("color"))
    );
    assert_eq!(
        missing_string_parser
            .string_opt_or(String::from("auto"))
            .unwrap(),
        String::from("auto")
    );
    assert_eq!(
        missing_string_parser.next().unwrap(),
        Some(Arg::Long("help"))
    );

    let mut missing_string_default_parser = parser(&["--color", "--help"]);
    assert_eq!(
        missing_string_default_parser.next().unwrap(),
        Some(Arg::Long("color"))
    );
    assert_eq!(
        missing_string_default_parser
            .string_opt_or_default()
            .unwrap(),
        String::new()
    );
    assert_eq!(
        missing_string_default_parser.next().unwrap(),
        Some(Arg::Long("help"))
    );

    let mut present_string_parser = parser(&["--color", "always"]);
    assert_eq!(
        present_string_parser.next().unwrap(),
        Some(Arg::Long("color"))
    );
    assert_eq!(
        present_string_parser
            .string_opt_or(String::from("auto"))
            .unwrap(),
        String::from("always")
    );

    let mut present_string_default_parser = parser(&["--color", "always"]);
    assert_eq!(
        present_string_default_parser.next().unwrap(),
        Some(Arg::Long("color"))
    );
    assert_eq!(
        present_string_default_parser
            .string_opt_or_default()
            .unwrap(),
        String::from("always")
    );
}

#[test]
fn parser_can_split_current_values_directly() {
    let mut parser = parser(&["--define", "KEY=VALUE", "--port", "PORT=8080"]);

    assert_eq!(parser.next().unwrap(), Some(Arg::Long("define")));
    assert_eq!(
        parser.split_once_nonempty_key_owned('=').unwrap(),
        (String::from("KEY"), String::from("VALUE"))
    );

    assert_eq!(parser.next().unwrap(), Some(Arg::Long("port")));
    assert_eq!(
        parser.split_once_nonempty_key_parse::<u16>('=').unwrap(),
        ("PORT", 8080)
    );
}

#[test]
fn parser_can_push_repeated_values_directly() {
    let mut parser = parser(&[
        "--include",
        "include",
        "--include",
        "generated",
        "--name",
        "demo",
        "--port",
        "8080",
        "--port",
        "9090",
    ]);
    let mut includes = Vec::new();
    let mut names = Vec::new();
    let mut ports = Vec::new();

    assert_eq!(parser.next().unwrap(), Some(Arg::Long("include")));
    parser.push_path_buf(&mut includes).unwrap();
    assert_eq!(parser.next().unwrap(), Some(Arg::Long("include")));
    parser.push_path_buf(&mut includes).unwrap();

    assert_eq!(parser.next().unwrap(), Some(Arg::Long("name")));
    parser.push_string(&mut names).unwrap();

    assert_eq!(parser.next().unwrap(), Some(Arg::Long("port")));
    parser.push_parse::<u16>(&mut ports).unwrap();
    assert_eq!(parser.next().unwrap(), Some(Arg::Long("port")));
    parser.push_parse::<u16>(&mut ports).unwrap();

    assert_eq!(
        includes,
        vec![PathBuf::from("include"), PathBuf::from("generated")]
    );
    assert_eq!(names, vec![String::from("demo")]);
    assert_eq!(ports, vec![8080, 9090]);
}

#[test]
fn parser_can_push_repeated_pairs_directly() {
    let mut parser = parser(&[
        "--env",
        "RUST_LOG=debug",
        "--env",
        "MODE=release",
        "--port-map",
        "HTTP=8080",
    ]);
    let mut envs = Vec::new();
    let mut port_maps = Vec::new();

    assert_eq!(parser.next().unwrap(), Some(Arg::Long("env")));
    parser
        .push_split_once_nonempty_key_owned('=', &mut envs)
        .unwrap();
    assert_eq!(parser.next().unwrap(), Some(Arg::Long("env")));
    parser
        .push_split_once_nonempty_key_owned('=', &mut envs)
        .unwrap();
    assert_eq!(parser.next().unwrap(), Some(Arg::Long("port-map")));
    parser
        .push_split_once_nonempty_key_parse::<u16>('=', &mut port_maps)
        .unwrap();

    assert_eq!(
        envs,
        vec![
            (String::from("RUST_LOG"), String::from("debug")),
            (String::from("MODE"), String::from("release")),
        ]
    );
    assert_eq!(port_maps, vec![(String::from("HTTP"), 8080)]);
}

#[test]
fn parser_can_read_optional_utf8_and_owned_os_values_directly() {
    let mut string_parser = parser(&["--color", "auto"]);
    assert_eq!(string_parser.next().unwrap(), Some(Arg::Long("color")));
    assert_eq!(string_parser.string_opt().unwrap(), Some("auto"));

    let mut owned_string_parser = parser(&["--color", "auto"]);
    assert_eq!(
        owned_string_parser.next().unwrap(),
        Some(Arg::Long("color"))
    );
    assert_eq!(
        owned_string_parser.string_opt_owned().unwrap(),
        Some(String::from("auto"))
    );

    let mut missing_string_parser = parser(&["--color", "--help"]);
    assert_eq!(
        missing_string_parser.next().unwrap(),
        Some(Arg::Long("color"))
    );
    assert_eq!(missing_string_parser.string_opt().unwrap(), None);
    assert_eq!(
        missing_string_parser.next().unwrap(),
        Some(Arg::Long("help"))
    );

    let mut missing_owned_string_parser = parser(&["--color", "--help"]);
    assert_eq!(
        missing_owned_string_parser.next().unwrap(),
        Some(Arg::Long("color"))
    );
    assert_eq!(
        missing_owned_string_parser.string_opt_owned().unwrap(),
        None
    );
    assert_eq!(
        missing_owned_string_parser.next().unwrap(),
        Some(Arg::Long("help"))
    );

    let mut os_parser = parser(&["--path", "./data"]);
    assert_eq!(os_parser.next().unwrap(), Some(Arg::Long("path")));
    assert_eq!(
        os_parser.os_string_opt().unwrap(),
        Some(OsString::from("./data"))
    );

    let mut path_parser = parser(&["--path", "./data"]);
    assert_eq!(path_parser.next().unwrap(), Some(Arg::Long("path")));
    assert_eq!(
        path_parser.path_buf_opt().unwrap(),
        Some(PathBuf::from("./data"))
    );
}

#[test]
fn into_remaining_preserves_unread_arguments() {
    let mut parser = parser(&["--port", "8080", "cmd", "--tail"]);
    assert_eq!(parser.next().unwrap(), Some(Arg::Long("port")));

    let remaining = parser.into_remaining();
    assert_eq!(remaining.size_hint(), (3, Some(3)));
    assert_eq!(remaining.len(), 3);

    let remaining = remaining.collect::<Vec<_>>();
    assert_eq!(
        remaining,
        vec![
            OsString::from("8080"),
            OsString::from("cmd"),
            OsString::from("--tail")
        ]
    );
}

#[test]
fn into_remaining_reconstructs_grouped_short_tail() {
    let mut parser = parser(&["-abc", "tail"]);
    assert_eq!(parser.next().unwrap(), Some(Arg::Short('a')));

    let remaining = parser.into_remaining();
    assert_eq!(remaining.size_hint(), (2, Some(2)));
    assert_eq!(remaining.len(), 2);

    let remaining = remaining.collect::<Vec<_>>();
    assert_eq!(
        remaining,
        vec![OsString::from("-bc"), OsString::from("tail")]
    );
}

#[test]
fn into_remaining_keeps_optional_value_lookahead() {
    let mut parser = parser(&["--color", "--help", "tail"]);
    assert_eq!(parser.next().unwrap(), Some(Arg::Long("color")));
    assert_eq!(parser.parse_opt::<u8>().unwrap(), None);

    let remaining = parser.into_remaining();
    assert_eq!(remaining.size_hint(), (2, Some(2)));
    assert_eq!(remaining.len(), 2);

    let remaining = remaining.collect::<Vec<_>>();
    assert_eq!(
        remaining,
        vec![OsString::from("--help"), OsString::from("tail")]
    );
}

#[test]
fn current_value_and_remaining_returns_command_and_tail() {
    let mut parser = parser(&["cargo", "test", "--", "--nocapture"]);
    assert!(matches!(parser.next().unwrap(), Some(Arg::Value(_))));

    let (command, remaining) = parser.current_value_and_remaining().unwrap();
    assert_eq!(command, OsString::from("cargo"));
    assert_eq!(
        remaining,
        vec![
            OsString::from("test"),
            OsString::from("--"),
            OsString::from("--nocapture"),
        ]
    );
}

#[test]
fn current_value_and_remaining_rejects_options() {
    let mut parser = parser(&["--env", "RUST_LOG=debug"]);
    assert_eq!(parser.next().unwrap(), Some(Arg::Long("env")));

    let error = parser.current_value_and_remaining().unwrap_err();
    assert_eq!(error.kind(), ErrorKind::ValueUnavailable);
    assert_eq!(error.argument().unwrap().to_string_lossy(), "--env");
}

#[test]
fn current_value_and_remaining_rejects_exhausted_parser() {
    let mut parser = parser(&["cargo"]);
    assert!(matches!(parser.next().unwrap(), Some(Arg::Value(_))));
    assert_eq!(parser.next().unwrap(), None);

    let error = parser.current_value_and_remaining().unwrap_err();
    assert_eq!(error.kind(), ErrorKind::ValueUnavailable);
}

#[cfg(unix)]
#[test]
fn preserves_non_utf8_values() {
    use std::ffi::OsString;
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    let value = OsString::from_vec(vec![0xff, b'a', b't', b'h']);
    let mut parser = Parser::new(vec![value].into_iter());

    match parser.next().unwrap() {
        Some(Arg::Value(raw)) => {
            assert_eq!(raw.as_os_str().as_bytes(), &[0xff, b'a', b't', b'h']);
            assert_eq!(raw.to_str().unwrap_err().kind(), ErrorKind::InvalidUtf8);
        }
        other => panic!("unexpected argument: {other:?}"),
    }
}

#[cfg(unix)]
#[test]
fn rejects_non_utf8_long_option_names() {
    use std::os::unix::ffi::OsStringExt;

    let mut parser = Parser::new(vec![OsString::from_vec(vec![b'-', b'-', 0xff])].into_iter());
    let error = parser.next().unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidOptionName);
}
