use super::{flags::CommandFlags, register_command, Exec, ShellCommand};
use crate::prompt::{confirm, Answer};
use crate::symlnk::SymLink;
use crate::{eval::Value, scope::Scope};
use filetime::FileTime;
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File};
use std::io::{self, ErrorKind::Other, Read, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

#[derive(Debug)]
enum Action {
    Copy,
    CreateDir,
    Link,
}

#[derive(Debug)]
struct WorkItem<'a> {
    top: &'a str, // Top source path as given in the command line
    action: Action,
    src: PathBuf,
}

impl<'a> WorkItem<'a> {
    fn new(top: &'a str, action: Action, src: PathBuf) -> Self {
        Self { top, action, src }
    }
}

trait WrapErr<T> {
    fn wrap_err(self, fc: &FileCopier, top: &str, path: &Path) -> T;
    fn wrap_err_with_msg(self, fc: &FileCopier, top: &str, path: &Path, msg: Option<&str>) -> T;
}

impl<T> WrapErr<Result<T, io::Error>> for Result<T, io::Error> {
    /// `top` is the path name as specified in the command line,
    /// `path` is the path the error is related to -- in most cases the resolved, or
    /// canonicalized version of `top`. `top` is looked up in the original command
    /// args, so that when the error is reported to the user, the error location
    /// that is shown is as close as possible to the argument that caused the error.
    fn wrap_err_with_msg(
        self,
        fc: &FileCopier,
        top: &str,
        path: &Path,
        msg: Option<&str>,
    ) -> Result<T, io::Error> {
        match self {
            Ok(v) => Ok(v),
            Err(e) => {
                // Map the top source path to its position in the command line arguments.
                let pos = fc.args.iter().position(|a| a == top).unwrap_or(0);
                // Store the position of the argument that originated the error.
                fc.scope.set_err_arg(pos);

                // Format error message to include path.
                let message = if let Some(msg) = msg {
                    format!("{} {}: {}", msg, fc.scope.err_path(path), e)
                } else {
                    format!("{}: {}", fc.scope.err_path(path), e)
                };
                Err(io::Error::new(io::ErrorKind::Other, message))
            }
        }
    }

    fn wrap_err(self, fc: &FileCopier, top: &str, path: &Path) -> Result<T, io::Error> {
        self.wrap_err_with_msg(fc, top, path, None)
    }
}

struct FileCopier<'a> {
    dest: PathBuf, // Destination
    debug: bool,
    ignore_links: bool,      // Skip symbolic links
    confirm_overwrite: bool, // Ask for overwrite confirmation?
    no_hidden: bool,         // Ignore entries starting with '.'
    preserve_metadata: bool,
    progress: Option<ProgressBar>,
    recursive: bool,
    scope: &'a Rc<Scope>,
    srcs: &'a [String], // Source paths from the command line
    args: &'a [String], // All the original command line args
    visited: HashSet<PathBuf>,
    work: BTreeMap<PathBuf, WorkItem<'a>>,
    total_size: u64,
}

impl<'a> FileCopier<'a> {
    fn new(
        paths: &'a [String],
        flags: &CommandFlags,
        scope: &'a Rc<Scope>,
        args: &'a [String],
    ) -> Self {
        Self {
            dest: PathBuf::from(paths.last().unwrap()),
            // Command line flags
            debug: flags.is_present("debug"),
            ignore_links: flags.is_present("no-dereference"),
            confirm_overwrite: !flags.is_present("force") || flags.is_present("interactive"),
            no_hidden: flags.is_present("no-hidden"),
            preserve_metadata: !flags.is_present("no-preserve"),
            recursive: flags.is_present("recursive"),
            // Progress indicator
            progress: if flags.is_present("progress") {
                let template = if scope.use_colors(&std::io::stdout()) {
                    "{spinner:.green} [{elapsed_precise}] {msg:>30.cyan.bright} {total_bytes}"
                } else {
                    "{spinner} [{elapsed_precise}] {msg:>30} {total_bytes}"
                };
                let pb = ProgressBar::with_draw_target(None, ProgressDrawTarget::stdout());
                pb.set_style(ProgressStyle::default_spinner().template(template).unwrap());
                pb.enable_steady_tick(Duration::from_millis(100));
                Some(pb)
            } else {
                None
            },
            scope,
            srcs: &paths[..paths.len() - 1],
            args,
            visited: HashSet::new(),
            work: BTreeMap::new(),
            total_size: 0,
        }
    }

    fn resolve_dest(&self, top: &'a str, parent: &Path, src: &Path) -> io::Result<PathBuf> {
        if self.dest.is_dir() {
            if src == parent {
                Ok(self.dest.join(src.file_name().unwrap()))
            } else {
                let path = src.strip_prefix(parent).map_err(|e| {
                    self.error(top, src, &format!("Could not remove prefix: {}", e))
                })?;
                Ok(self.dest.join(path))
            }
        } else {
            Ok(self.dest.to_path_buf())
        }
    }

    /// Add work item to create a directory.
    fn add_create_dir(&mut self, top: &'a str, parent: &Path, src: &Path) -> io::Result<()> {
        let actual_dest = self.resolve_dest(top, parent, src)?;
        let work_item = WorkItem::new(top, Action::CreateDir, src.to_path_buf());
        self.work.insert(actual_dest, work_item);

        Ok(())
    }

    fn check_dir_dest(&mut self, top: &'a str, parent: &Path) -> io::Result<()> {
        if self.dest.exists() {
            // Copying multiple files over a regular file?
            if !self.dest.is_dir() && !self.work.is_empty() {
                return Err(self.error(
                    &top,
                    parent,
                    "Copying multiple sources over single destination",
                ));
            }
        } else if !self.work.is_empty() {
            return Err(self.error(
                &top,
                parent,
                "Copying multiple sources to non-existing directory",
            ));
        }
        Ok(())
    }

    /// Add a work item to copy the contents of a regular file (i.e. not symlink, not dir).
    fn add_copy(&mut self, top: &'a str, parent: &Path, src: &Path) -> io::Result<()> {
        assert!(!src.is_dir());

        self.check_dir_dest(top, parent)?;
        let actual_dest = self.resolve_dest(top, parent, src)?;

        if actual_dest.exists() && actual_dest.canonicalize()? == src.canonicalize()? {
            return Err(self.error(top, &actual_dest, "Source and destination are the same"));
        }

        let work_item = WorkItem::new(top, Action::Copy, src.to_path_buf());
        self.work.insert(actual_dest, work_item);

        Ok(())
    }

    fn add_link(&mut self, top: &'a str, parent: &Path, src: &Path) -> io::Result<()> {
        let target =
            src.resolve()
                .wrap_err_with_msg(&self, top, src, Some("Could not get link target"))?;
        let actual_dest = self.resolve_dest(top, parent, &target)?;
        let work_item = WorkItem::new(top, Action::Link, src.to_path_buf());
        self.work.insert(actual_dest, work_item);

        Ok(())
    }

    /// Collect info about one path and its size, recurse if directory.
    /// Return Ok(false) if interrupted by Ctrl+C.
    /// Update progress indicator in verbose mode.
    fn collect_path_info(&mut self, top: &'a str, parent: &Path, path: &Path) -> io::Result<bool> {
        // Check for Ctrl+C
        if self.scope.is_interrupted() {
            return Ok(false);
        }
        // Check symlinks first; canonicalize() further down may error out on WSL links.
        if self.ignore_links && path.is_symlink() {
            return Ok(true);
        }
        // Ignore files and dirs starting with '.'? Useful for
        // copying project directories without .git, .vscode, etc.
        if self.no_hidden
            && path
                .file_name()
                .is_some_and(|f| f.to_string_lossy().starts_with("."))
        {
            if self.debug {
                eprintln!("{}: skip hidden", path.display());
            }
            return Ok(true);
        }
        // Bail if the path has been seen before
        if !self
            .visited
            .insert(path.resolve().wrap_err(&self, top, path)?)
        {
            if self.debug {
                eprintln!("{}: already visited", path.display());
            }
            return Ok(true);
        }

        if path.is_symlink() {
            assert!(!self.ignore_links);
            self.add_link(top, parent, path)?;
        } else if path.is_dir() {
            if !self.recursive {
                my_warning!(self.scope, "{} is a directory", self.scope.err_path(path));
                return Ok(true);
            }
            // Replicate dirs from the source into the destination, even if empty.
            self.add_create_dir(top, parent, path)?;

            // Collect info recursively
            for entry in fs::read_dir(path).wrap_err(&self, top, path)? {
                let entry = entry.wrap_err(&self, top, path)?;
                let child = entry.path();

                if !self.collect_path_info(top, parent, &child)? {
                    return Ok(false); // User interrupted
                }
            }
        } else {
            let size = fs::metadata(&path).wrap_err(&self, top, path)?.len();

            self.total_size += size;
            self.add_copy(top, parent, path)?;

            // Update progress indicator, if set up (-v flag specified)
            if let Some(pb) = &self.progress {
                pb.set_message(format!("{}", Self::truncate_path(path)));
                pb.set_position(self.total_size);
            }
        }
        Ok(true)
    }

    /// Collect the list of files to copy and their sizes.
    /// Create work items. Return Ok(false) on Ctrl+C.
    fn collect_src_info(&mut self) -> io::Result<bool> {
        assert!(!self.srcs.is_empty());

        // Always resolve symbolic links in the destination.
        self.dest = self.dest.resolve().wrap_err(
            &self,
            self.dest.as_os_str().to_str().unwrap_or(""),
            &self.dest,
        )?;
        if self.debug {
            eprintln!("{}: exists={}", self.dest.display(), self.dest.exists());
        }
        for src in self.srcs {
            // Always resolve symbolic links for the source paths given in the command line.
            let path = Path::new(src).resolve()?;
            let parent = path.parent().unwrap_or(&path);

            if self.debug {
                eprintln!("Collect: {} (resolved: {})", src, path.display());
            }

            // Collect source info for the top paths, checking for cancellation.
            if !self.collect_path_info(src, &parent, &path)? {
                if let Some(pb) = self.progress.as_mut() {
                    pb.abandon_with_message("Aborted");
                }
                return Ok(false);
            }
        }
        if let Some(pb) = self.progress.as_mut() {
            pb.finish_with_message("Collected source file(s)");
        }
        Ok(true)
    }

    /// Construct io::Error with given path and message.
    fn error(&self, top: &str, path: &Path, msg: &str) -> io::Error {
        // Map the top source path to its position in the command line arguments.
        let pos = self.args.iter().position(|a| a == top).unwrap_or(0);
        // Store the position of the argument that originated the error.
        self.scope.set_err_arg(pos);

        io::Error::new(Other, format!("{}: {}", self.scope.err_path(path), msg))
    }

    /// Truncate path for display in progress indicator.
    fn truncate_path(path: &Path) -> String {
        const MAX_LENGTH: usize = 30;
        let filename = path.to_str().unwrap_or("");
        if filename.len() <= MAX_LENGTH {
            filename.to_uppercase()
        } else {
            let start_index = filename.len() - (MAX_LENGTH - 3);
            format!("...{}", &filename[start_index..])
        }
    }

    fn reset_progress_indicator(&mut self, size: u64) {
        let template = if self.scope.use_colors(&std::io::stdout()) {
            "{spinner:.green} [{elapsed_precise}] {msg:>30.cyan.bright} [{bar:45.green/}] {bytes}/{total_bytes} ({eta})"
        } else {
            "{spinner:} [{elapsed_precise}] {msg:>30} [{bar:45}] {bytes}/{total_bytes} ({eta})"
        };

        let pb = ProgressBar::with_draw_target(Some(size), ProgressDrawTarget::stdout());
        pb.set_style(
            ProgressStyle::default_bar()
                .template(&template)
                .unwrap()
                .progress_chars("=> "),
        );

        self.progress = Some(pb);
    }

    /// Collect all source files, their total size, re-create all dirs in the
    /// source(s) and copy the files; symlinks require Admin privilege on Windows.
    fn copy(&mut self) -> io::Result<()> {
        if !self.collect_src_info()? {
            return Ok(());
        }

        if self.progress.is_some() {
            self.reset_progress_indicator(self.total_size);
        }

        self.do_work()
    }

    fn do_work(&mut self) -> io::Result<()> {
        let work = std::mem::take(&mut self.work);

        for (dest, w) in &work {
            if let Some(pb) = self.progress.as_mut() {
                pb.set_message(Self::truncate_path(&w.src));
            }

            if !self.do_work_item(work.len(), &dest, &w)? {
                if let Some(pb) = self.progress.as_mut() {
                    pb.abandon_with_message("Aborted");
                }
                return Ok(());
            }
        }

        if let Some(pb) = self.progress.as_mut() {
            pb.finish_with_message("Ok");
        }

        Ok(())
    }

    fn do_work_item(&mut self, count: usize, dest: &PathBuf, w: &WorkItem) -> io::Result<bool> {
        match w.action {
            Action::Copy => {
                if self.debug {
                    eprintln!("COPY: {} -> {}", w.src.display(), dest.display());
                }
                assert!(!dest.is_dir());

                if self.confirm_overwrite && dest.exists() {
                    match confirm(
                        format!("Overwrite {}", dest.display()),
                        self.scope,
                        count > 1,
                    )? {
                        Answer::Yes => {}
                        Answer::No => return Ok(true), // Continue
                        Answer::All => {
                            self.confirm_overwrite = false;
                        }
                        Answer::Quit => return Ok(false), // Cancel all
                    }
                }
                if !self.copy_file(w.top, &w.src, dest)? {
                    return Ok(false);
                }
            }
            Action::CreateDir => {
                if self.debug {
                    eprintln!("CREATE: {} ({})", dest.display(), w.src.display());
                }
                if !dest.exists() {
                    fs::create_dir(dest).wrap_err(&self, w.top, &w.src)?;
                }
            }
            Action::Link => {
                if self.debug {
                    eprintln!("LINK: {} -> {}", w.src.display(), dest.display());
                }
                //TODO
                //copy_symlink(&w.src, &dest).wrap_err(&self, w.top, &w.src)?;
            }
        }
        Ok(true)
    }

    /// Copy the contents of a regular file.
    /// Update progress indicator in verbose mode.
    fn copy_file(&mut self, top: &str, src: &Path, dest: &PathBuf) -> io::Result<bool> {
        #[cfg(unix)]
        self.handle_unix_special_file(src, dest)?;

        let mut src_file = File::open(src).wrap_err(&self, top, src)?;
        let mut dst_file = File::create(&dest).wrap_err(&self, top, dest)?;

        let mut buffer = [0; 8192]; // TODO: allow user to specify buffer size?
        loop {
            if self.scope.is_interrupted() {
                return Ok(false);
            }
            let n = src_file.read(&mut buffer).wrap_err(&self, top, src)?;
            if n == 0 {
                break;
            }
            dst_file
                .write_all(&buffer[..n])
                .wrap_err(&self, top, dest)?;

            if let Some(pb) = self.progress.as_mut() {
                pb.inc(n as u64);
            }
        }

        if self.preserve_metadata {
            self.preserve_metadata(top, src, dest)?;
        }

        Ok(true)
    }

    #[cfg(unix)]
    fn handle_unix_special_file(&self, src: &Path, dest: &PathBuf) -> io::Result<()> {
        use std::os::unix::fs::FileTypeExt;
        let file_type = fs::symlink_metadata(src)?.file_type();

        if file_type.is_fifo() {
            // Recreate the FIFO rather than copying contents
            nix::unistd::mkfifo(dest, nix::sys::stat::Mode::S_IRWXU)?;
        } else if file_type.is_socket() {
            my_warning!(self.scope, "Skipping socket: {}", self.scope.err_path(src));
        } else if file_type.is_block_device() || file_type.is_char_device() {
            my_warning!(
                self.scope,
                "Skipping device file: {}",
                self.scope.err_path(src)
            );
        }
        Ok(())
    }

    fn preserve_metadata(&self, top: &str, src: &Path, dest: &PathBuf) -> io::Result<()> {
        // Get metadata of source file
        let metadata = fs::metadata(src).wrap_err_with_msg(
            &self,
            top,
            src,
            Some("Could not read metadata"),
        )?;

        // Set timestamps on destination file
        filetime::set_file_times(
            dest,
            FileTime::from_last_access_time(&metadata),
            FileTime::from_last_modification_time(&metadata),
        )
        .wrap_err_with_msg(&self, top, dest, Some("Could not set file time"))?;

        // Set permissions on the destination
        fs::set_permissions(dest, metadata.permissions()).wrap_err(&self, top, dest)?;

        #[cfg(unix)]
        {
            use nix::unistd::{chown, Gid, Uid};
            use std::os::unix::fs::MetadataExt;

            let uid = metadata.uid();
            let gid = metadata.gid();

            chown(dest, Some(Uid::from_raw(uid)), Some(Gid::from_raw(gid)))?;
        }

        Ok(())
    }
}

#[allow(dead_code)]
fn copy_symlink(src: &Path, dst: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs as unix_fs;
        let target = fs::read_link(src)?;
        unix_fs::symlink(target, dst)
    }
    #[cfg(windows)]
    {
        // use std::os::windows::fs as windows_fs;
        // let target = fs::read_link(src)?;
        // if src.is_dir() {
        //     windows_fs::symlink_dir(target, dst)
        // } else {
        //     windows_fs::symlink_file(target, dst)
        // }
        //
        // ...fails with reparse errors, use Windows APIs instead (Admin required)
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::Storage::FileSystem::{
            CreateSymbolicLinkW, SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE,
            SYMBOLIC_LINK_FLAG_DIRECTORY,
        };

        let target = src;

        let dst_wstr = PCWSTR(
            dst.as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<u16>>()
                .as_ptr(),
        );
        let target_wstr = PCWSTR(
            target
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<u16>>()
                .as_ptr(),
        );

        let flags = if src.is_dir() {
            SYMBOLIC_LINK_FLAG_DIRECTORY | SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE
        } else {
            SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE
        };

        let result = unsafe { CreateSymbolicLinkW(dst_wstr, target_wstr, flags) };
        if result.0 != 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

struct Cp {
    flags: CommandFlags,
}

impl Cp {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message");
        flags.add_flag('d', "debug", "Show debugging details");
        flags.add_flag('v', "progress", "Show progress bar");
        flags.add_flag('r', "recursive", "Copy directories recursively");
        flags.add_flag('f', "force", "Overwrite without prompting");
        flags.add_flag('i', "interactive", "Prompt before overwrite (default)");
        flags.add_flag('P', "no-dereference", "Ignore symbolic links in SOURCE");
        flags.add(None, "no-hidden", false, "Ignore hidden files");
        flags.add(
            None,
            "no-preserve",
            false,
            "Do not preserve permissions and time stamps",
        );
        Cp { flags }
    }
}

impl Exec for Cp {
    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Rc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let paths = flags.parse(scope, args)?;

        if flags.is_present("help") {
            println!("Usage: cp [OPTIONS] SOURCE... DEST");
            println!("Copy SOURCE(s) to DESTination.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        if paths.is_empty() {
            return Err("Missing source and destination".to_string());
        }
        if paths.len() < 2 {
            return Err("Missing destination".to_string());
        }

        let mut copier = FileCopier::new(&paths, &flags, scope, &args);
        copier.copy().map_err(|e| e.to_string())?;

        Ok(Value::success())
    }
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "cp".to_string(),
        inner: Rc::new(Cp::new()),
    });
}
