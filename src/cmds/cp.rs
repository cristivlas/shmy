use super::{register_command, Exec, ShellCommand};
use crate::cmds::flags::CommandFlags;
use crate::eval::{Scope, Value};
use crate::prompt::{confirm, Answer};
use filetime::FileTime;
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

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
        use windows_sys::Win32::Storage::FileSystem::{
            CreateSymbolicLinkW, SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE,
            SYMBOLIC_LINK_FLAG_DIRECTORY,
        };

        let target = src;

        let dst_wstr: Vec<u16> = dst.as_os_str().encode_wide().chain(Some(0)).collect();
        let target_wstr: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();

        let flags = if src.is_dir() {
            SYMBOLIC_LINK_FLAG_DIRECTORY
        } else {
            0
        } | SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE;

        let result = unsafe { CreateSymbolicLinkW(dst_wstr.as_ptr(), target_wstr.as_ptr(), flags) };

        if result == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

struct FileCopier<'a> {
    dest: PathBuf,           // Destination
    ignore_links: bool,      // Skip symbolic links
    confirm_overwrite: bool, // Ask for overwrite confirmation?
    no_hidden: bool,         // Ignore entries starting with '.'
    preserve_metadata: bool,
    progress: Option<ProgressBar>,
    recursive: bool,
    scope: &'a Rc<Scope>,
    srcs: &'a [String], // Source paths
}

impl<'a> FileCopier<'a> {
    fn new(args: &'a Vec<String>, flags: &CommandFlags, scope: &'a Rc<Scope>) -> Self {
        Self {
            dest: PathBuf::from(args.last().unwrap()),
            ignore_links: flags.is_present("no-dereference"),
            confirm_overwrite: !flags.is_present("force") || flags.is_present("interactive"),
            no_hidden: flags.is_present("no-hidden"),
            preserve_metadata: !flags.is_present("no-preserve"),
            progress: if flags.is_present("progress") {
                let template =
                    "{spinner:.green} [{elapsed_precise}] {msg:>30.green.bright} {total_bytes}";
                let pb = ProgressBar::with_draw_target(None, ProgressDrawTarget::stdout());
                pb.set_style(ProgressStyle::default_spinner().template(template).unwrap());
                pb.enable_steady_tick(Duration::from_millis(100));
                Some(pb)
            } else {
                None
            },
            recursive: flags.is_present("recursive"),
            scope,
            srcs: &args[..args.len() - 1],
        }
    }

    // Add the path to the error reported to the caller
    fn wrap_error<E: std::fmt::Display>(&self, path: &Path, error: E) -> io::Error {
        let canonical_path = path.canonicalize().unwrap_or(path.to_path_buf());
        io::Error::new(
            io::ErrorKind::Other,
            format!("{}: {}", self.scope.err_path(&canonical_path), error),
        )
    }

    /// Collect info about one path and its size, recurse if directory.
    /// Return Ok(false) if interrupted by Ctrl+C.
    /// Update progress indicator in verbose mode.
    fn collect_path_info(
        &self,
        start: &'a str,
        path: &Path,
        info: &mut (Vec<(&'a str, PathBuf)>, u64),
    ) -> io::Result<bool> {
        // Ignore files and dirs starting with '.'? Useful for
        // copying project directories without .git, .vscode, etc.
        if self.no_hidden
            && path
                .file_name()
                .is_some_and(|f| f.to_string_lossy().starts_with("."))
        {
            return Ok(true);
        }

        if path.is_symlink() {
            if !self.ignore_links {
                info.0.push((start, path.to_path_buf()));
            }
        } else if path.is_dir() {
            if !self.recursive {
                my_warning!(self.scope, "Omitting dir: {}", self.scope.err_path(path));
                return Ok(true);
            }
            // Replicate dirs from the source into the destination (even if empty)
            info.0.push((start, path.to_path_buf()));

            // Collect info recursively
            for entry in fs::read_dir(path).map_err(|e| self.wrap_error(path, e))? {
                if self.scope.is_interrupted() {
                    return Ok(false);
                }
                let entry = entry.map_err(|e| self.wrap_error(path, e))?;
                let child = entry.path();

                if !self.collect_path_info(start, &child, info)? {
                    return Ok(false); // User interrupted
                }
            }
        } else {
            let size = fs::metadata(&path)
                .map_err(|e| self.wrap_error(&path, e))?
                .len();

            info.0.push((start, path.to_path_buf()));
            info.1 += size;

            // Update progress indicator, if set up.
            if let Some(pb) = &self.progress {
                pb.set_message(format!("{}", Self::truncate_path(path)));
                pb.set_position(info.1);
            }
        }
        Ok(true)
    }

    /// Recursively collect the list of files to copy and their sizes.
    /// Return Ok(false) if interrupted by the user pressing Ctrl+C.
    fn collect_src_info(&mut self, info: &mut (Vec<(&'a str, PathBuf)>, u64)) -> io::Result<bool> {
        for src in self.srcs {
            let path = Path::new(src);
            if !self.collect_path_info(src, path, info)? {
                if let Some(pb) = self.progress.as_mut() {
                    pb.finish_with_message("Aborted");
                }
                return Ok(false);
            }
        }

        if let Some(pb) = self.progress.as_mut() {
            pb.finish_with_message("Done");
        }
        Ok(true)
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

    /// Collect all source files, their total size, re-create all dirs in the
    /// source(s) and copy the files; symlinks require Admin privilege on Windows.
    fn copy(&mut self) -> io::Result<()> {
        assert!(!self.srcs.is_empty());

        let dest_is_dir = self.dest.is_dir();
        // Some sanity checks first...
        if !dest_is_dir {
            if self.srcs.len() > 1 {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    "Multiple sources with non-directory destination".to_string(),
                ));
            }
            if Path::new(&self.srcs[0]).is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    "Source is a directory and destination is not".to_string(),
                ));
            }
        }

        // Collect sources and their total size
        let mut info = (Vec::new(), 0u64);
        if !self.collect_src_info(&mut info)? {
            return Ok(());
        }

        if self.progress.is_some() {
            // Reset the progress indicator.
            let template =
                "{spinner:.green} [{elapsed_precise}] {msg:>30.green.bright} [{bar:45.cyan/blue}] {bytes}/{total_bytes} ({eta})";

            let pb = ProgressBar::with_draw_target(Some(info.1), ProgressDrawTarget::stdout());
            pb.set_style(
                ProgressStyle::default_bar()
                    .template(&template)
                    .unwrap()
                    .progress_chars("=>-"),
            );
            self.progress = Some(pb);
        }

        let many = info.0.len() > 1;

        for (start, path) in &info.0 {
            if self.scope.is_interrupted() {
                if let Some(pb) = self.progress.as_mut() {
                    pb.abandon_with_message("Interrupted");
                }
                return Ok(()); // User interrupted (pressed Ctrl+C)
            }

            let dest = if dest_is_dir {
                let src_path = if let Some(parent) = Path::new(start).parent() {
                    path.strip_prefix(parent)
                        .map_err(|e| self.wrap_error(path, e))?
                } else {
                    path
                };
                self.dest.join(src_path)
            } else {
                self.dest.to_path_buf()
            };

            // Copy the individual entry
            if !self.copy_entry(many, path, &dest)? {
                if let Some(pb) = self.progress.as_mut() {
                    pb.abandon_with_message("Aborted");
                }
                return Ok(());
            }
        }
        if let Some(pb) = self.progress.as_mut() {
            pb.finish_with_message("Done");
        }
        Ok(())
    }

    /// Copy the individual entry: create a dir, symlink or copy regular file.
    /// Update progress indicator in verbose mode.
    fn copy_entry(&mut self, one_of_many: bool, src: &Path, dest: &PathBuf) -> io::Result<bool> {
        if dest.exists() && src.canonicalize()? == dest.canonicalize()? {
            return Err(self.wrap_error(dest, "Source and destination are the same"));
        }

        // Ask for confirmation if needed
        if self.confirm_overwrite && dest.exists() && !dest.is_dir() {
            match confirm(
                format!("Overwrite {}", dest.display()),
                self.scope,
                one_of_many,
            )? {
                Answer::Yes => {}
                Answer::No => return Ok(true), // Continue
                Answer::All => {
                    self.confirm_overwrite = false;
                }
                Answer::Quit => return Ok(false), // Cancel all
            }
        }

        if let Some(pb) = self.progress.as_mut() {
            pb.set_message(Self::truncate_path(dest));
        }

        if src.is_dir() {
            if !dest.exists() {
                fs::create_dir(dest)?;
            }
        } else if src.is_symlink() {
            copy_symlink(src, &dest).map_err(|e| self.wrap_error(src, e))?;
        } else {
            #[cfg(unix)]
            self.handle_unix_special_file(src, dest)?;

            let mut src_file = File::open(src).map_err(|e| self.wrap_error(src, e))?;
            let mut dst_file = File::create(&dest).map_err(|e| self.wrap_error(dest, e))?;
            let mut buffer = [0; 8192]; // TODO: allow user to specify buffer size?
            loop {
                if self.scope.is_interrupted() {
                    return Ok(false);
                }
                let n = src_file
                    .read(&mut buffer)
                    .map_err(|e| self.wrap_error(src, e))?;

                if n == 0 {
                    break;
                }
                dst_file
                    .write_all(&buffer[..n])
                    .map_err(|e| self.wrap_error(dest, e))?;

                if let Some(pb) = self.progress.as_mut() {
                    pb.inc(n as u64);
                }
            }
        }

        if self.preserve_metadata {
            self.preserve_metadata(src, dest)?;
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

    fn preserve_metadata(&self, src: &Path, dest: &PathBuf) -> io::Result<()> {
        // Get metadata of source file
        let metadata = fs::metadata(src)
            .map_err(|e| self.wrap_error(src, format!("Cannot read metadata: {}", e)))?;

        // Set timestamps on destination file
        filetime::set_file_times(
            dest,
            FileTime::from_last_access_time(&metadata),
            FileTime::from_last_modification_time(&metadata),
        )
        .map_err(|e| self.wrap_error(dest, format!("Cannot set file time: {}", e)))?;

        // Set permissions on the destination
        fs::set_permissions(dest, metadata.permissions())
            .map_err(|e| self.wrap_error(dest, format!("Cannot set permissions: {}", e)))?;

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

struct Cp {
    flags: CommandFlags,
}

impl Cp {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message");
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
    fn is_external(&self) -> bool {
        false
    }

    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Rc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let args = flags.parse(args)?;

        if flags.is_present("help") {
            println!("Usage: cp [OPTIONS] SOURCE... DEST");
            println!("Copy SOURCE(s) to DESTination.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        if args.is_empty() {
            return Err("Missing source and destination".to_string());
        }
        if args.len() < 2 {
            return Err("Missing destination".to_string());
        }

        let mut copier = FileCopier::new(&args, &flags, scope);
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
