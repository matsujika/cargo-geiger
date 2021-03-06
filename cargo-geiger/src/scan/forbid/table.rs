use crate::format::emoji_symbols::EmojiSymbols;
use crate::format::pattern::Pattern;
use crate::format::print_config::PrintConfig;
use crate::format::{get_kind_group_name, SymbolKind};
use crate::graph::Graph;
use crate::tree::traversal::walk_dependency_tree;
use crate::tree::TextTreeLine;

use super::super::find::find_unsafe;
use super::super::ScanMode;

use cargo::core::{Package, PackageId, PackageSet};
use cargo::{CliResult, Config};
use colored::Colorize;

pub fn scan_forbid_to_table(
    config: &Config,
    package_set: &PackageSet,
    root_package_id: PackageId,
    graph: &Graph,
    print_config: &PrintConfig,
) -> CliResult {
    let mut scan_output_lines = Vec::<String>::new();
    let emoji_symbols = EmojiSymbols::new(print_config.charset);

    let mut output_key_lines = construct_key_lines(&emoji_symbols);
    scan_output_lines.append(&mut output_key_lines);

    let tree_lines =
        walk_dependency_tree(root_package_id, &graph, &print_config);
    for tree_line in tree_lines {
        match tree_line {
            TextTreeLine::ExtraDepsGroup { kind, tree_vines } => {
                let name = get_kind_group_name(kind);
                if name.is_none() {
                    continue;
                }
                let name = name.unwrap();
                // TODO: Fix the alignment on macOS (others too?)
                scan_output_lines.push(format!("  {}{}", tree_vines, name));
            }
            TextTreeLine::Package {
                id: package_id,
                tree_vines,
            } => {
                handle_package_text_tree_line(
                    config,
                    &emoji_symbols,
                    package_id,
                    package_set,
                    print_config,
                    &mut scan_output_lines,
                    tree_vines,
                )?;
            }
        }
    }

    for scan_output_line in scan_output_lines {
        println!("{}", scan_output_line);
    }

    Ok(())
}

fn construct_key_lines(emoji_symbols: &EmojiSymbols) -> Vec<String> {
    let mut output_key_lines = Vec::<String>::new();

    output_key_lines.push(String::new());
    output_key_lines.push(String::from("Symbols: "));

    let forbids = "All entry point .rs files declare #![forbid(unsafe_code)].";
    let unknown = "This crate may use unsafe code.";

    let symbol_kinds_to_string_values = vec![
        (SymbolKind::Lock, forbids),
        (SymbolKind::QuestionMark, unknown),
    ];

    for (symbol_kind, string_values) in symbol_kinds_to_string_values {
        output_key_lines.push(format!(
            "    {: <2} = {}",
            emoji_symbols.emoji(symbol_kind),
            string_values
        ));
    }

    output_key_lines.push(String::new());
    output_key_lines
}

fn format_package_name(package: &Package, pattern: &Pattern) -> String {
    format!(
        "{}",
        pattern.display(&package.package_id(), package.manifest().metadata())
    )
}

fn handle_package_text_tree_line(
    config: &Config,
    emoji_symbols: &EmojiSymbols,
    package_id: PackageId,
    package_set: &PackageSet,
    print_config: &PrintConfig,
    scan_output_lines: &mut Vec<String>,
    tree_vines: String,
) -> CliResult {
    let geiger_ctx = find_unsafe(
        ScanMode::EntryPointsOnly,
        config,
        package_set,
        print_config,
    )?;
    let sym_lock = emoji_symbols.emoji(SymbolKind::Lock);
    let sym_qmark = emoji_symbols.emoji(SymbolKind::QuestionMark);

    let package = package_set.get_one(package_id).unwrap(); // FIXME
    let name = format_package_name(package, &print_config.format);
    let package_metrics = geiger_ctx.package_id_to_metrics.get(&package_id);
    let package_forbids_unsafe = match package_metrics {
        None => false, // no metrics available, .rs parsing failed?
        Some(package_metric) => package_metric.rs_path_to_metrics.iter().all(
            |(_k, rs_file_metrics_wrapper)| {
                rs_file_metrics_wrapper.metrics.forbids_unsafe
            },
        ),
    };
    let (symbol, name) = if package_forbids_unsafe {
        (&sym_lock, name.green())
    } else {
        (&sym_qmark, name.red())
    };
    scan_output_lines.push(format!("{} {}{}", symbol, tree_vines, name));

    Ok(())
}

#[cfg(test)]
mod forbid_tests {
    use super::*;

    use crate::format::Charset;

    use cargo::core::Workspace;
    use cargo::util::important_paths;
    use rstest::*;

    #[rstest]
    fn construct_scan_mode_forbid_only_output_key_lines_test() {
        let emoji_symbols = EmojiSymbols::new(Charset::Utf8);
        let output_key_lines = construct_key_lines(&emoji_symbols);

        assert_eq!(output_key_lines.len(), 5);
    }

    #[rstest]
    fn format_package_name_test() {
        let pattern = Pattern::try_build("{p}").unwrap();

        let config = Config::default().unwrap();
        let workspace = Workspace::new(
            &important_paths::find_root_manifest_for_wd(config.cwd()).unwrap(),
            &config,
        )
        .unwrap();

        let package = workspace.current().unwrap();

        let formatted_package_name = format_package_name(&package, &pattern);

        assert_eq!(formatted_package_name, "cargo-geiger 0.10.2");
    }
}
