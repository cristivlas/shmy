use super::{register_command, Exec, ShellCommand};
use crate::cmds::flags::CommandFlags;
use crate::eval::{Scope, Value};
use crate::prompt::{confirm, Answer};
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

// Add the path to the error reported to the caller
fn wrap_error<E: std::fmt::Display>(path: &Path, error: E) -> io::Error {
    io::Error::new(
        io::ErrorKind::Other,
        format!("{}: {}", path.display(), error),
    )
}

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

        Cp { flags }
    }

    fn truncate_filename(filename: &str, max_length: usize) -> String {
        if filename.len() <= max_length {
            filename.to_string()
        } else {
            let truncated = &filename[..max_length - 3];
            format!("{}...", truncated)
        }
    }

    fn copy_file(&self, src: &Path, dst: &Path, pb: Option<&ProgressBar>) -> io::Result<()> {
        if src.is_symlink() {
            return copy_symlink(src, dst).map_err(|e| wrap_error(src, e));
        }
        if src.is_dir() {
            // Re-create dirs even if empty
            return fs::create_dir_all(dst).map_err(|e| wrap_error(src, e));
        }

        let mut src_file = File::open(src)?;
        let mut dst_file = File::create(dst)?;

        if let Some(pb) = pb {
            pb.set_message(Self::truncate_filename(
                src.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string()
                    .as_str(),
                30,
            ));
        }

        let mut buffer = [0; 8192];
        loop {
            let n = src_file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            dst_file.write_all(&buffer[..n])?;
            if let Some(pb) = pb {
                pb.inc(n as u64);
            }
        }

        Ok(())
    }

    fn get_source_files_and_size(
        &self,
        scope: &Rc<Scope>,
        ignore_links: bool,
        src: &Path,
        show_progress: bool,
    ) -> io::Result<(Vec<PathBuf>, u64)> {
        let mut total_size = 0;
        let mut files = Vec::new();

        let pb = if show_progress {
            let pb = ProgressBar::new_spinner();
            pb.set_style(
                ProgressStyle::default_spinner()
                    .template("{spinner:.green} Collecting files: {total_bytes} {wide_msg}")
                    .unwrap(),
            );
            pb.enable_steady_tick(Duration::from_millis(100));
            Some(pb)
        } else {
            None
        };

        let mut collect_size = |path: &Path, size: u64| {
            total_size += size;
            if let Some(pb) = &pb {
                pb.set_message(format!(
                    "{}",
                    Self::truncate_filename(path.to_str().unwrap_or(""), 30)
                ));
                pb.set_position(total_size);
            }
        };

        if src.is_dir() {
            for entry in fs::read_dir(src).map_err(|e| wrap_error(src, e))? {
                if scope.is_interrupted() {
                    break;
                }
                let entry = entry.map_err(|e| wrap_error(src, e))?;
                let path = entry.path();

                if path.is_symlink() {
                    if !ignore_links {
                        files.push(path);
                    }
                } else if path.is_dir() {
                    files.push(path.clone()); // Ensure dirs are created, even if empty

                    let (mut sub_files, size) = self
                        .get_source_files_and_size(scope, ignore_links, &path, false)
                        .map_err(|e| wrap_error(&path, e))?;
                    collect_size(&path, size);
                    files.append(&mut sub_files);
                } else {
                    let size = fs::metadata(&path).map_err(|e| wrap_error(&path, e))?.len();
                    collect_size(&path, size);
                    files.push(path);
                }
            }
        } else if src.is_symlink() {
            if !ignore_links {
                files.push(src.to_path_buf());
            }
        } else {
            let size = fs::metadata(src).map_err(|e| wrap_error(src, e))?.len();
            collect_size(src, size);
            files.push(src.to_path_buf());
        }

        if let Some(pb) = pb {
            pb.finish_with_message("Done");
        }

        Ok((files, total_size))
    }

    fn copy_files(
        &self,
        scope: &Rc<Scope>,
        src: &Path,
        dst: &Path,
        files: &[PathBuf],
        pb: Option<&ProgressBar>,
        interactive: &mut bool,
    ) -> io::Result<bool> {
        for file in files {
            if scope.is_interrupted() {
                break; // Ctrl+C pressed
            }
            let relative_path = file.strip_prefix(src).unwrap();
            let dst_path = dst.join(relative_path);

            if let Some(parent) = dst_path.parent() {
                fs::create_dir_all(parent)?;
            }

            if *interactive && dst_path.exists() {
                match confirm(format!("Overwrite '{}'", dst_path.display()), scope, true)? {
                    Answer::No => continue,
                    Answer::Quit => return Ok(false),
                    Answer::Yes => {}
                    Answer::All => {
                        *interactive = false;
                        continue;
                    }
                }
            }

            self.copy_file(file, &dst_path, pb)?;
        }
        Ok(true)
    }

    fn copy(
        &self,
        scope: &Rc<Scope>,
        src: &Path,
        dst: &Path,
        ignore_links: bool,
        interactive: &mut bool,
        show_progress: bool,
        recursive: bool,
    ) -> io::Result<()> {
        if !recursive && src.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("Omitting directory: {}", src.display()),
            ));
        }

        let (files, total_size) =
            self.get_source_files_and_size(scope, ignore_links, src, show_progress)?;

        let pb = if show_progress {
            let pb = ProgressBar::with_draw_target(Some(total_size), ProgressDrawTarget::stdout());
            pb.set_style(ProgressStyle::default_bar()
                .template("[{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta}) {msg}")
                .unwrap()
                .progress_chars("=>-"));
            Some(pb)
        } else {
            None
        };

        let mut complete = false;

        if recursive {
            complete = self.copy_files(scope, src, dst, &files, pb.as_ref(), interactive)?;
        } else {
            if *interactive
                && dst.exists()
                && confirm(format!("Overwrite '{}'", dst.display()), scope, false)? != Answer::Yes
            {
                return Ok(());
            }
            self.copy_file(src, dst, pb.as_ref())?;
        }

        complete &= !scope.is_interrupted();

        if complete {
            if let Some(pb) = pb {
                pb.finish_with_message("done");
            }
        }

        Ok(())
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
            println!("Usage: cp [OPTIONS] SOURCE DEST");
            println!("Copy SOURCE to DESTination.");
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
        if args.len() > 2 {
            return Err("Extraneous argument".to_string());
        }

        let ignore_links = flags.is_present("no-dereference");
        let mut interactive = !flags.is_present("force") || flags.is_present("interactive");
        let recursive = flags.is_present("recursive");
        let show_progress = flags.is_present("progress");

        let src = Path::new(&args[0]);
        let dst = Path::new(&args[1]);

        self.copy(
            scope,
            src,
            dst,
            ignore_links,
            &mut interactive,
            show_progress,
            recursive,
        )
        .map_err(|e| format!("{}", e))?;

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
