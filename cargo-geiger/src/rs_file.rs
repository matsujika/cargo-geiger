mod custom_executor;

use custom_executor::{CustomExecutor, CustomExecutorInnerContext};

use cargo::core::compiler::Executor;
use cargo::core::manifest::TargetKind;
use cargo::core::Workspace;
use cargo::ops;
use cargo::ops::{CleanOptions, CompileOptions};
use cargo::util::{interning::InternedString, paths, CargoResult};
use cargo::Config;
use geiger::RsFileMetrics;
use std::collections::HashSet;
use std::error::Error;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use walkdir::{DirEntry, WalkDir};

/// Provides information needed to scan for crate root
/// `#![forbid(unsafe_code)]`.
/// The wrapped PathBufs are canonicalized.
#[derive(Debug, PartialEq)]
pub enum RsFile {
    /// Library entry point source file, usually src/lib.rs
    LibRoot(PathBuf),

    /// Executable entry point source file, usually src/main.rs
    BinRoot(PathBuf),

    /// Not sure if this is relevant but let's be conservative for now.
    CustomBuildRoot(PathBuf),

    /// All other .rs files.
    Other(PathBuf),
}

#[derive(Debug, Default)]
pub struct RsFileMetricsWrapper {
    /// The information returned by the `geiger` crate for a `.rs` file.
    pub metrics: RsFileMetrics,

    /// All crate entry points must declare forbid(unsafe_code) to make it count
    /// for the crate as a whole. The `geiger` crate is decoupled from `cargo`
    /// and cannot know if a file is a crate entry point or not, so we add this
    /// information here.
    pub is_crate_entry_point: bool,
}

#[derive(Debug)]
pub enum RsResolveError {
    /// This should not happen unless incorrect assumptions have been made in
    /// cargo-geiger about how the cargo API works.
    ArcUnwrap(),

    /// Would like cargo::Error here, but it's private, why?
    /// This is still way better than a panic though.
    Cargo(String),

    /// Failed to parse a .dep file.
    DepParse(String, PathBuf),

    /// Failed to get the inner context out of the mutex.
    InnerContextMutex(String),

    /// Like io::Error but with the related path.
    Io(io::Error, PathBuf),

    Walkdir(walkdir::Error),
}

impl Error for RsResolveError {}

/// Forward Display to Debug.
impl fmt::Display for RsResolveError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

impl From<PoisonError<CustomExecutorInnerContext>> for RsResolveError {
    fn from(e: PoisonError<CustomExecutorInnerContext>) -> Self {
        RsResolveError::InnerContextMutex(e.to_string())
    }
}

pub fn into_rs_code_file(kind: &TargetKind, path: PathBuf) -> RsFile {
    match kind {
        TargetKind::Lib(_) => RsFile::LibRoot(path),
        TargetKind::Bin => RsFile::BinRoot(path),
        TargetKind::Test => RsFile::Other(path),
        TargetKind::Bench => RsFile::Other(path),
        TargetKind::ExampleLib(_) => RsFile::Other(path),
        TargetKind::ExampleBin => RsFile::Other(path),
        TargetKind::CustomBuild => RsFile::CustomBuildRoot(path),
    }
}

pub fn is_file_with_ext(entry: &DirEntry, file_ext: &str) -> bool {
    if !entry.file_type().is_file() {
        return false;
    }
    let p = entry.path();
    let ext = match p.extension() {
        Some(e) => e,
        None => return false,
    };
    // to_string_lossy is ok since we only want to match against an ASCII
    // compatible extension and we do not keep the possibly lossy result
    // around.
    ext.to_string_lossy() == file_ext
}

/// Trigger a `cargo clean` + `cargo check` and listen to the cargo/rustc
/// communication to figure out which source files were used by the build.
pub fn resolve_rs_file_deps(
    compile_options: &CompileOptions,
    workspace: &Workspace,
) -> Result<HashSet<PathBuf>, RsResolveError> {
    let config = workspace.config();
    // Need to run a cargo clean to identify all new .d deps files.
    // TODO: Figure out how this can be avoided to improve performance, clean
    // Rust builds are __slow__.
    let clean_options = CleanOptions {
        config: &config,
        spec: vec![],
        targets: vec![],
        profile_specified: false,
        // A temporary hack to get cargo 0.43 to build, TODO: look closer at the updated cargo API
        // later.
        requested_profile: InternedString::new("dev"),
        doc: false,
    };

    ops::clean(workspace, &clean_options)
        .map_err(|e| RsResolveError::Cargo(e.to_string()))?;

    let inner_arc = Arc::new(Mutex::new(CustomExecutorInnerContext::default()));
    {
        compile_with_exec(
            compile_options,
            config,
            inner_arc.clone(),
            workspace,
        )?;
    }

    let workspace_root = workspace.root().to_path_buf();
    let inner_mutex =
        Arc::try_unwrap(inner_arc).map_err(|_| RsResolveError::ArcUnwrap())?;
    let (rs_files, out_dir_args) = {
        let ctx = inner_mutex.into_inner()?;
        (ctx.rs_file_args, ctx.out_dir_args)
    };
    let mut path_buf_hash_set = HashSet::<PathBuf>::new();
    for out_dir in out_dir_args {
        // TODO: Figure out if the `.d` dep files are used by one or more rustc
        // calls. It could be useful to know which `.d` dep files belong to
        // which rustc call. That would allow associating each `.rs` file found
        // in each dep file with a PackageId.
        add_dir_entries_to_path_buf_hash_set(
            out_dir,
            &mut path_buf_hash_set,
            workspace_root.clone(),
        )?;
    }
    for path_buf in rs_files {
        // rs_files must already be canonicalized
        path_buf_hash_set.insert(path_buf);
    }

    Ok(path_buf_hash_set)
}

fn add_dir_entries_to_path_buf_hash_set(
    out_dir: PathBuf,
    path_buf_hash_set: &mut HashSet<PathBuf>,
    workspace_root: PathBuf,
) -> Result<(), RsResolveError> {
    for entry in WalkDir::new(&out_dir) {
        let entry = entry.map_err(RsResolveError::Walkdir)?;
        if !is_file_with_ext(&entry, "d") {
            continue;
        }
        let dependencies = parse_rustc_dep_info(entry.path()).map_err(|e| {
            RsResolveError::DepParse(e.to_string(), entry.path().to_path_buf())
        })?;
        let canonical_paths = dependencies
            .into_iter()
            .flat_map(|t| t.1)
            .map(PathBuf::from)
            .map(|pb| workspace_root.join(pb))
            .map(|pb| pb.canonicalize().map_err(|e| RsResolveError::Io(e, pb)));
        for path_buf in canonical_paths {
            path_buf_hash_set.insert(path_buf?);
        }
    }

    Ok(())
}

fn compile_with_exec(
    compile_options: &CompileOptions,
    config: &Config,
    inner_arc: Arc<Mutex<CustomExecutorInnerContext>>,
    workspace: &Workspace,
) -> Result<(), RsResolveError> {
    let custom_executor = CustomExecutor {
        cwd: config.cwd().to_path_buf(),
        inner_ctx: inner_arc,
    };

    let custom_executor_arc: Arc<dyn Executor> = Arc::new(custom_executor);

    ops::compile_with_exec(workspace, &compile_options, &custom_executor_arc)
        .map_err(|e| RsResolveError::Cargo(e.to_string()))?;

    Ok(())
}

/// Copy-pasted (almost) from the private module cargo::core::compiler::fingerprint.
///
/// TODO: Make a PR to the cargo project to expose this function or to expose
/// the dependency data in some other way.
fn parse_rustc_dep_info(
    rustc_dep_info: &Path,
) -> CargoResult<Vec<(String, Vec<String>)>> {
    let contents = paths::read(rustc_dep_info)?;
    contents
        .lines()
        .filter_map(|l| l.find(": ").map(|i| (l, i)))
        .map(|(line, pos)| {
            let target = &line[..pos];
            let mut deps = line[pos + 2..].split_whitespace();
            let mut ret = Vec::new();
            while let Some(s) = deps.next() {
                let mut file = s.to_string();
                while file.ends_with('\\') {
                    file.pop();
                    file.push(' ');
                    //file.push_str(deps.next().ok_or_else(|| {
                    //internal("malformed dep-info format, trailing \\".to_string())
                    //})?);
                    file.push_str(
                        deps.next()
                            .expect("malformed dep-info format, trailing \\"),
                    );
                }
                ret.push(file);
            }
            Ok((target.to_string(), ret))
        })
        .collect()
}

#[cfg(test)]
mod rs_file_tests {
    use super::*;
    use rstest::*;

    #[rstest(
        input_target_kind,
        expected_rs_file,
        case(
            TargetKind::Lib(vec![]),
            RsFile::LibRoot(
                Path::new("test_path.ext").to_path_buf()
            )
        ),
        case(
            TargetKind::Bin,
            RsFile::BinRoot(
                Path::new("test_path.ext").to_path_buf()
            )
        ),
        case(
            TargetKind::Test,
            RsFile::Other(
                Path::new("test_path.ext").to_path_buf()
            )
        ),
        case(
            TargetKind::Bench,
            RsFile::Other(
                Path::new("test_path.ext").to_path_buf()
            )
        ),
        case(
            TargetKind::ExampleLib(vec![]),
            RsFile::Other(
                Path::new("test_path.ext").to_path_buf()
            )
        ),
        case(
            TargetKind::ExampleBin,
            RsFile::Other(
                Path::new("test_path.ext").to_path_buf()
            )
        ),
        case(
            TargetKind::CustomBuild,
            RsFile::CustomBuildRoot(
                Path::new("test_path.ext").to_path_buf()
            )
        ),
    )]
    fn into_rs_code_file_test(
        input_target_kind: TargetKind,
        expected_rs_file: RsFile
    ) {
        let path_buf = Path::new("test_path.ext").to_path_buf();

        assert_eq!(
            into_rs_code_file(&input_target_kind, path_buf),
            expected_rs_file
        );
    }

    #[rstest]
    fn is_file_with_ext_test() {
        let config = Config::default().unwrap();
        let cwd = config.cwd();

        let walk_dir_rust_files = WalkDir::new(&cwd)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().to_str().unwrap().ends_with(".rs"));

        for entry in walk_dir_rust_files {
            assert_eq!(is_file_with_ext(&entry, "rs"), true);
        }

        let walk_dir_readme_files = WalkDir::new(&cwd)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().to_str().unwrap().contains("README"));

        for entry in walk_dir_readme_files {
            assert_eq!(is_file_with_ext(&entry, "rs"), false);
        }
    }
}
