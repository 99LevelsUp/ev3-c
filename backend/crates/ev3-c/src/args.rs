//! The command line of `ev3 c`, parsed by hand like Node's `util.parseArgs` in strict mode:
//! `--name value` or `--name=value` for the options that take a value, the switches alone,
//! anything after `--` positional, and any other option refused.

/// What the command line asked for, before the values are checked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Arguments {
    pub positionals: Vec<String>,
    pub brick: Option<String>,
    pub toolchain: Option<String>,
    pub keep: Option<String>,
    pub json: bool,
    pub events: bool,
    pub help: bool,
    pub version: bool,
}

/// Options that take a value.
const VALUED: [&str; 3] = ["brick", "toolchain", "keep"];
/// Options that are switches.
const SWITCHES: [&str; 4] = ["json", "events", "help", "version"];

pub fn parse_arguments(argv: &[String]) -> Result<Arguments, String> {
    let mut parsed = Arguments::default();
    let mut rest = argv.iter();
    while let Some(arg) = rest.next() {
        if arg == "--" {
            parsed.positionals.extend(rest.cloned());
            break;
        }
        let Some(option) = arg.strip_prefix("--") else {
            if arg.len() > 1 && arg.starts_with('-') {
                return Err(format!("unknown option \"{arg}\""));
            }
            parsed.positionals.push(arg.clone());
            continue;
        };
        let (name, inline) = match option.split_once('=') {
            Some((name, value)) => (name, Some(value.to_owned())),
            None => (option, None),
        };
        if VALUED.contains(&name) {
            let value = match inline {
                Some(value) => value,
                // A value that looks like an option is more likely a forgotten value.
                None => match rest.next() {
                    Some(value) if !value.starts_with('-') => value.clone(),
                    _ => return Err(format!("option \"--{name}\" needs a value")),
                },
            };
            let slot = match name {
                "brick" => &mut parsed.brick,
                "toolchain" => &mut parsed.toolchain,
                _ => &mut parsed.keep,
            };
            *slot = Some(value);
        } else if SWITCHES.contains(&name) {
            if inline.is_some() {
                return Err(format!("option \"--{name}\" takes no value"));
            }
            match name {
                "json" => parsed.json = true,
                "events" => parsed.events = true,
                "help" => parsed.help = true,
                _ => parsed.version = true,
            }
        } else {
            return Err(format!("unknown option \"{arg}\""));
        }
    }
    Ok(parsed)
}

/// Whether the command line asks for `--events`, also when it is otherwise wrong: a caller
/// reading events then still gets its terminal event for a usage error.
pub fn wants_events(argv: &[String]) -> bool {
    argv.iter()
        .take_while(|arg| *arg != "--")
        .any(|arg| arg == "--events")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Arguments, String> {
        let argv: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
        parse_arguments(&argv)
    }

    #[test]
    fn options_take_their_value_after_a_space_or_an_equals_sign() {
        let parsed = parse(&[
            "run",
            "--brick",
            "BBB",
            "robot",
            "--toolchain=/z/zig",
            "--json",
        ])
        .unwrap();
        assert_eq!(parsed.positionals, ["run", "robot"]);
        assert_eq!(parsed.brick.as_deref(), Some("BBB"));
        assert_eq!(parsed.toolchain.as_deref(), Some("/z/zig"));
        assert!(parsed.json && !parsed.events && !parsed.help && !parsed.version);
        assert_eq!(
            parse(&["stop", "--brick", "A", "--brick", "B"])
                .unwrap()
                .brick
                .as_deref(),
            Some("B"),
            "the last one wins"
        );
    }

    #[test]
    fn everything_after_a_double_dash_is_positional() {
        let parsed = parse(&["get", "--", "--odd", "-x"]).unwrap();
        assert_eq!(parsed.positionals, ["get", "--odd", "-x"]);
        assert!(!wants_events(&[
            "run".into(),
            "--".into(),
            "--events".into()
        ]));
        assert!(wants_events(&["fly".into(), "--events".into()]));
    }

    #[test]
    fn unknown_options_missing_values_and_values_for_switches_are_refused() {
        assert_eq!(
            parse(&["run", "--fast"]).unwrap_err(),
            "unknown option \"--fast\""
        );
        assert_eq!(parse(&["run", "-f"]).unwrap_err(), "unknown option \"-f\"");
        assert_eq!(
            parse(&["runs", "--keep"]).unwrap_err(),
            "option \"--keep\" needs a value"
        );
        assert_eq!(
            parse(&["run", "--brick", "--json"]).unwrap_err(),
            "option \"--brick\" needs a value"
        );
        assert_eq!(
            parse(&["run", "--json=yes"]).unwrap_err(),
            "option \"--json\" takes no value"
        );
        assert_eq!(
            parse(&["get", "-"]).unwrap().positionals,
            ["get", "-"],
            "a lone dash is a value"
        );
    }
}
