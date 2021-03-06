use crate::args::Args;
use crate::format::pattern::Pattern;
use crate::format::{Charset, CrateDetectionStatus, FormatError};

use cargo::core::shell::Verbosity;
use cargo::util::errors::CliError;
use colored::Colorize;
use geiger::IncludeTests;
use petgraph::EdgeDirection;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Prefix {
    Depth,
    Indent,
    None,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputFormat {
    Json,
}

#[derive(Debug, PartialEq)]
pub struct PrintConfig {
    /// Don't truncate dependencies that have already been displayed.
    pub all: bool,

    pub allow_partial_results: bool,
    pub charset: Charset,
    pub direction: EdgeDirection,

    // Is anyone using this? This is a carry-over from cargo-tree.
    // TODO: Open a github issue to discuss deprecation.
    pub format: Pattern,

    pub include_tests: IncludeTests,
    pub prefix: Prefix,
    pub output_format: Option<OutputFormat>,
    pub verbosity: Verbosity,
}

impl PrintConfig {
    pub fn new(args: &Args) -> Result<Self, CliError> {
        // TODO: Add command line flag for this and make it default to false?
        let allow_partial_results = true;

        let direction = if args.invert {
            EdgeDirection::Incoming
        } else {
            EdgeDirection::Outgoing
        };

        let format = Pattern::try_build(&args.format).map_err(|e| {
            CliError::new(
                (FormatError {
                    message: e.to_string(),
                })
                .into(),
                1,
            )
        })?;

        let include_tests = if args.include_tests {
            IncludeTests::Yes
        } else {
            IncludeTests::No
        };

        let prefix = if args.prefix_depth {
            Prefix::Depth
        } else if args.no_indent {
            Prefix::None
        } else {
            Prefix::Indent
        };

        let verbosity = if args.verbose == 0 {
            Verbosity::Normal
        } else {
            Verbosity::Verbose
        };

        Ok(PrintConfig {
            all: args.all,
            allow_partial_results,
            charset: args.charset,
            direction,
            format,
            include_tests,
            output_format: args.output_format,
            prefix,
            verbosity,
        })
    }
}

pub fn colorize(
    string: String,
    crate_detection_status: &CrateDetectionStatus,
) -> colored::ColoredString {
    match crate_detection_status {
        CrateDetectionStatus::NoneDetectedForbidsUnsafe => string.green(),
        CrateDetectionStatus::NoneDetectedAllowsUnsafe => string.normal(),
        CrateDetectionStatus::UnsafeDetected => string.red().bold(),
    }
}

#[cfg(test)]
mod print_config_tests {
    use super::*;

    use colored::ColoredString;
    use rstest::*;

    #[rstest(
        input_invert_bool,
        expected_edge_direction,
        case(
            true,
            EdgeDirection::Incoming
        ),
        case(
            false,
            EdgeDirection::Outgoing
        )
    )]
    fn print_config_new_test_invert(
        input_invert_bool: bool,
        expected_edge_direction: EdgeDirection
    ) {
        let mut args = create_args();
        args.invert = input_invert_bool;

        let print_config_result = PrintConfig::new(&args);

        assert!(print_config_result.is_ok());
        assert_eq!(
            print_config_result.unwrap().direction,
            expected_edge_direction
        );
    }

    #[rstest(
        input_include_tests_bool,
        expected_include_tests,
        case(
            true,
            IncludeTests::Yes
        ),
        case(
            false,
            IncludeTests::No
        ),
    )]
    fn print_config_new_test_include_tests(
        input_include_tests_bool: bool,
        expected_include_tests: IncludeTests
    ) {
        let mut args = create_args();
        args.include_tests = input_include_tests_bool;

        let print_config_result = PrintConfig::new(&args);

        assert!(print_config_result.is_ok());
        assert_eq!(
            print_config_result.unwrap().include_tests,
            expected_include_tests
        );
    }

    #[rstest(
        input_prefix_depth_bool,
        input_no_indent_bool,
        expected_output_prefix,
        case(
            true,
            false,
            Prefix::Depth,
        ),
        case(
            true,
            false,
            Prefix::Depth,
        ),
        case(
            false,
            true,
            Prefix::None,
        ),
        case(
            false,
            false,
            Prefix::Indent,
        ),
    )]
    fn print_config_new_test_prefix(
        input_prefix_depth_bool: bool,
        input_no_indent_bool: bool,
        expected_output_prefix: Prefix
    ) {
        let mut args = create_args();
        args.prefix_depth = input_prefix_depth_bool;
        args.no_indent = input_no_indent_bool;

        let print_config_result = PrintConfig::new(&args);

        assert!(print_config_result.is_ok());
        assert_eq!(
            print_config_result.unwrap().prefix,
            expected_output_prefix
        );
    }

    #[rstest(
        input_verbosity_u32,
        expected_verbosity,
        case(
            0,
            Verbosity::Normal
        ),
        case(
            1,
            Verbosity::Verbose
        ),
        case(
            1,
            Verbosity::Verbose
        )
    )]
    fn print_config_new_test_verbosity(
        input_verbosity_u32: u32,
        expected_verbosity: Verbosity,
    ) {
        let mut args = create_args();
        args.verbose = input_verbosity_u32;

        let print_config_result = PrintConfig::new(&args);

        assert!(print_config_result.is_ok());
        assert_eq!(
            print_config_result.unwrap().verbosity,
            expected_verbosity
        );
    }

    #[rstest(
        input_crate_detection_status,
        expected_colorized_string,
        case(
            CrateDetectionStatus::NoneDetectedForbidsUnsafe,
            String::from("string_value").green()
        ),
        case(
            CrateDetectionStatus::NoneDetectedAllowsUnsafe,
            String::from("string_value").normal()
        ),
        case(
            CrateDetectionStatus::UnsafeDetected,
            String::from("string_value").red().bold()
        )
    )]
    fn colorize_test(
        input_crate_detection_status: CrateDetectionStatus,
        expected_colorized_string: ColoredString,
    ) {
        let string_value = String::from("string_value");

        assert_eq!(
            colorize(
                string_value,
                &input_crate_detection_status
            ),
            expected_colorized_string
        );
    }

    fn create_args() -> Args {
        Args{
            all: false,
            all_deps: false,
            all_features: false,
            all_targets: false,
            build_deps: false,
            charset: Charset::Ascii,
            color: None,
            dev_deps: false,
            features: None,
            forbid_only: false,
            format: "".to_string(),
            frozen: false,
            help: false,
            include_tests: false,
            invert: false,
            locked: false,
            manifest_path: None,
            no_default_features: false,
            no_indent: false,
            offline: false,
            package: None,
            prefix_depth: false,
            quiet: false,
            target: None,
            unstable_flags: vec![],
            verbose: 0,
            version: false,
            output_format: None
        }
    }
}
